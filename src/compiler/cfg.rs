//! Per-function control-flow lowering of immutable checked bodies.
//!
//! Explicit value snapshots and edges preserve source evaluation order. This
//! module selects Copy/Move operands from projected semantic types. Ownership
//! dataflow is established separately by the analyzer before native emission.
use super::frontend::{BinaryOp, UnaryOp};
use super::ir::{self, ExprId, IntTy, IntValue, LocalId, LoopId, Place, Projection, Ty};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoanId(pub usize);

#[derive(Clone, Debug)]
pub struct LoanSite {
    pub kind: ir::BorrowKind,
    pub place: Place,
    pub origin: Option<ExprId>,
}

#[derive(Clone, Debug)]
pub struct Located<T> {
    pub value: T,
    pub origin: Option<ExprId>,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub ref_contract: std::sync::Arc<ir::RefContract>,
    pub loan_sites: Vec<LoanSite>,
    pub name: String,
    /// Parameters are implicitly live and initialized on entry, before the
    /// first statement. All other locals require explicit StorageLive.
    pub params: Vec<LocalId>,
    pub result: Ty,
    pub locals: Vec<ir::LocalDecl>,
    pub blocks: Vec<BasicBlock>,
    pub entry: BlockId,
    pub return_local: LocalId,
}

#[derive(Clone, Debug)]
pub struct BasicBlock {
    pub statements: Vec<Located<Statement>>,
    pub terminator: Located<Terminator>,
}

#[derive(Clone, Debug)]
pub enum Statement {
    IndexAssignOp {
        array: Place,
        index: Operand,
        value: Operand,
        op: BinaryOp,
    },
    IndexAssign {
        array: Place,
        index: Operand,
        value: Operand,
    },
    IndexInspect {
        array: Place,
        index: Operand,
    },
    /// Nonconsuming initialized-place read needed while evaluating an index.
    /// This has no runtime load or value result.
    Inspect(Place),
    StorageLive(LocalId),
    StorageDead(LocalId),
    Assign {
        destination: Place,
        value: Rvalue,
    },
}

#[derive(Clone, Debug)]
pub enum Constant {
    Enum { ty: ir::EnumTy, variant: usize },
    Int(IntValue),
    Bool(bool),
    Unit,
}

#[derive(Clone, Debug)]
pub enum Operand {
    Constant(Constant),
    Copy(Place),
    Move(Place),
}

#[derive(Clone, Debug)]
pub enum Rvalue {
    IntCast {
        value: Operand,
        to: IntTy,
    },
    Borrow(LoanId),
    IndexCopy {
        array: Place,
        index: Operand,
    },
    Repeat {
        ty: Ty,
        value: Operand,
    },
    EnumVariant {
        ty: ir::EnumTy,
        variant: usize,
        fields: Vec<Operand>,
    },
    /// Reads an initialized enum tag without moving the enum value.
    Discriminant(Place),
    Use(Operand),
    Unary {
        op: UnaryOp,
        value: Operand,
    },
    Binary {
        op: BinaryOp,
        left: Operand,
        right: Operand,
    },
    Aggregate {
        ty: Ty,
        fields: Vec<Operand>,
    },
    Println(String),
}

#[derive(Clone, Debug)]
pub enum Terminator {
    SwitchVariant {
        place: Place,
        variant: ir::VariantId,
        then_block: BlockId,
        else_block: BlockId,
    },
    Goto(BlockId),
    Switch {
        condition: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },
    Call {
        target: ir::CallTarget,
        name: String,
        signature: ir::Signature,
        args: Vec<Operand>,
        destination: Place,
        next: BlockId,
    },
    /// Consumes the value held in Function::return_local.
    Return,
    Unreachable,
}

impl Rvalue {
    fn visit_operands(
        &self,
        visit: &mut impl FnMut(&Operand) -> Result<(), String>,
    ) -> Result<(), String> {
        match self {
            Self::Use(value)
            | Self::IntCast { value, .. }
            | Self::Unary { value, .. }
            | Self::Repeat { value, .. } => visit(value)?,
            Self::Binary { left, right, .. } => {
                visit(left)?;
                visit(right)?;
            }
            Self::Aggregate { fields, .. } | Self::EnumVariant { fields, .. } => {
                for value in fields {
                    visit(value)?;
                }
            }
            Self::IndexCopy { index, .. } => visit(index)?,
            Self::Borrow(_) | Self::Discriminant(_) | Self::Println(_) => {}
        }
        Ok(())
    }
}
impl Statement {
    fn visit_operands(
        &self,
        visit: &mut impl FnMut(&Operand) -> Result<(), String>,
    ) -> Result<(), String> {
        match self {
            Self::Assign { value, .. } => value.visit_operands(visit)?,
            Self::IndexAssignOp { index, value, .. } | Self::IndexAssign { index, value, .. } => {
                visit(index)?;
                visit(value)?;
            }
            Self::IndexInspect { index, .. } => visit(index)?,
            Self::Inspect(_) | Self::StorageLive(_) | Self::StorageDead(_) => {}
        }
        Ok(())
    }
}
impl Terminator {
    fn visit_operands(
        &self,
        visit: &mut impl FnMut(&Operand) -> Result<(), String>,
    ) -> Result<(), String> {
        match self {
            Self::Call { args, .. } => {
                for value in args {
                    visit(value)?;
                }
            }
            Self::Switch { condition, .. } => visit(condition)?,
            Self::SwitchVariant { .. } | Self::Goto(_) | Self::Return | Self::Unreachable => {}
        }
        Ok(())
    }
}

fn coerces(got: &Ty, want: &Ty) -> bool {
    if got == &Ty::Never || got == want {
        return true;
    }
    match (got, want) {
        (
            Ty::Array {
                element: a,
                len: an,
            },
            Ty::Array {
                element: b,
                len: bn,
            },
        ) if an == bn => coerces(a, b),
        (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
            a.iter().zip(b.iter()).all(|(a, b)| coerces(a, b))
        }
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub struct TemplateCfg {
    source: std::sync::Arc<ir::CheckedFunction>,
    env: std::sync::Arc<ir::GenericEnv>,
    function: std::sync::Arc<Function>,
    budget: ir::PlanBudget,
}
impl TemplateCfg {
    pub fn source(&self) -> &std::sync::Arc<ir::CheckedFunction> {
        &self.source
    }
    pub fn env(&self) -> &std::sync::Arc<ir::GenericEnv> {
        &self.env
    }
    pub fn function(&self) -> &std::sync::Arc<Function> {
        &self.function
    }
    pub fn budget(&self) -> &ir::PlanBudget {
        &self.budget
    }
}
#[derive(Clone, Debug)]
pub struct ClosedCfg {
    function: std::sync::Arc<Function>,
    budget: ir::PlanBudget,
}
impl ClosedCfg {
    pub fn function(&self) -> &std::sync::Arc<Function> {
        &self.function
    }
    pub fn budget(&self) -> &ir::PlanBudget {
        &self.budget
    }
}
pub fn close(
    function: std::sync::Arc<Function>,
    budget: &ir::PlanBudget,
) -> Result<ClosedCfg, String> {
    function.preflight_plan(ir::FactsCtx::Concrete, budget)?;
    function.validate_with_facts(ir::FactsCtx::Concrete)?;
    Ok(ClosedCfg {
        function,
        budget: budget.clone(),
    })
}
pub fn lower_template(
    source: std::sync::Arc<ir::CheckedFunction>,
    env: std::sync::Arc<ir::GenericEnv>,
    budget: &ir::PlanBudget,
) -> Result<TemplateCfg, String> {
    budget.charge_nodes(source.exprs.len().saturating_add(source.locals.len()))?;
    let facts = ir::FactsCtx::Universal(&env);
    let function = std::sync::Arc::new(lower_with_facts(&source, facts, Some(budget))?);
    function.preflight_plan(facts, budget)?;
    function.validate_with_facts(facts)?;
    Ok(TemplateCfg {
        source,
        env,
        function,
        budget: budget.clone(),
    })
}

fn validate_type(ty: &Ty, depth: usize) -> Result<(), String> {
    let mut pending = vec![(ty, depth)];
    let mut work = 0usize;
    let mut nominals = std::collections::HashSet::new();
    let mut tuples = std::collections::HashSet::new();
    let mut arrays = std::collections::HashSet::new();
    while let Some((ty, depth)) = pending.pop() {
        work += 1;
        if work > ir::MAX_PATTERN_PROJECTION_WORK {
            return Err("CFG type work limit exceeded".into());
        }
        if depth > 128 {
            return Err("CFG type nesting exceeds limit 128".into());
        }
        match ty {
            Ty::Ref { referent, .. } => {
                if depth != 0 {
                    return Err("unsupported: nested or aggregate references".into());
                }
                pending.push((referent, depth + 1));
            }
            Ty::Array { element, len } => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err("CFG array length exceeds limit 4096".into());
                }
                if arrays.insert(std::sync::Arc::as_ptr(element)) {
                    // Validate element metadata even for an empty array.
                    pending.push((element, depth + 1));
                }
            }
            Ty::Enum(ty) if nominals.insert((true, ty.def().id)) => {
                ty.validate()?;
                for variant in ty.def().variants.iter() {
                    pending.extend(variant.fields.iter().map(|field| (&field.ty, depth + 1)));
                }
            }
            Ty::Struct(ty) if nominals.insert((false, ty.def().id)) => {
                if ty.def().fields.is_empty() {
                    return Err(
                        "empty CFG aggregates are unsupported; empty tuples must be Unit".into(),
                    );
                }
                pending.extend(ty.def().fields.iter().map(|field| (&field.ty, depth + 1)));
            }
            Ty::Tuple(fields) if tuples.insert(fields.as_ptr()) => {
                if fields.is_empty() {
                    return Err(
                        "empty CFG aggregates are unsupported; empty tuples must be Unit".into(),
                    );
                }
                if fields.len() > ir::MAX_PATTERN_PROJECTION_WORK {
                    return Err("CFG type work limit exceeded".into());
                }
                pending.extend(fields.iter().map(|field| (field, depth + 1)));
            }
            _ => {}
        }
        if pending.len() > ir::MAX_PATTERN_PROJECTION_WORK - work {
            return Err("CFG type work limit exceeded".into());
        }
    }
    Ok(())
}

impl Function {
    fn preflight_plan(
        &self,
        facts: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        fn path(place: &Place, budget: &ir::PlanBudget) -> Result<(), String> {
            budget.check_depth(place.projection.len())?;
            budget.charge_work(place.projection.len() + 1)
        }
        fn operand(
            value: &Operand,
            budget: &ir::PlanBudget,
            facts: ir::FactsCtx<'_>,
        ) -> Result<(), String> {
            budget.charge_work(1)?;
            match value {
                Operand::Constant(Constant::Enum { ty, .. }) => {
                    facts.validate_ty_with_budget(&Ty::Enum(ty.clone()), budget)?
                }
                Operand::Copy(place) | Operand::Move(place) => path(place, budget)?,
                Operand::Constant(Constant::Int(_) | Constant::Bool(_) | Constant::Unit) => {}
            }
            Ok(())
        }
        budget.charge_work(self.locals.len())?;
        for local in &self.locals {
            facts.validate_ty_with_budget(&local.ty, budget)?;
        }
        facts.validate_ty_with_budget(&self.result, budget)?;
        for site in &self.loan_sites {
            path(&site.place, budget)?;
        }
        for block in &self.blocks {
            budget.charge_work(1)?;
            for located in &block.statements {
                budget.charge_work(1)?;
                match &located.value {
                    Statement::IndexAssignOp {
                        array,
                        index,
                        value,
                        ..
                    }
                    | Statement::IndexAssign {
                        array,
                        index,
                        value,
                    } => {
                        path(array, budget)?;
                        operand(index, budget, facts)?;
                        operand(value, budget, facts)?;
                    }
                    Statement::IndexInspect { array, index } => {
                        path(array, budget)?;
                        operand(index, budget, facts)?;
                    }
                    Statement::Inspect(place) => path(place, budget)?,
                    Statement::StorageLive(_) | Statement::StorageDead(_) => {}
                    Statement::Assign { destination, value } => {
                        path(destination, budget)?;
                        match value {
                            Rvalue::Use(value)
                            | Rvalue::IntCast { value, .. }
                            | Rvalue::Unary { value, .. } => operand(value, budget, facts)?,
                            Rvalue::Repeat { ty, value } => {
                                facts.validate_ty_with_budget(ty, budget)?;
                                operand(value, budget, facts)?;
                            }
                            Rvalue::Binary { left, right, .. } => {
                                operand(left, budget, facts)?;
                                operand(right, budget, facts)?;
                            }
                            Rvalue::Aggregate { ty, fields } => {
                                facts.validate_ty_with_budget(ty, budget)?;
                                for value in fields {
                                    operand(value, budget, facts)?;
                                }
                            }
                            Rvalue::EnumVariant { ty, fields, .. } => {
                                facts.validate_ty_with_budget(&Ty::Enum(ty.clone()), budget)?;
                                for value in fields {
                                    operand(value, budget, facts)?;
                                }
                            }
                            Rvalue::IndexCopy { array, index } => {
                                path(array, budget)?;
                                operand(index, budget, facts)?;
                            }
                            Rvalue::Discriminant(place) => path(place, budget)?,
                            Rvalue::Borrow(_) | Rvalue::Println(_) => {}
                        }
                    }
                }
            }
            match &block.terminator.value {
                Terminator::Call {
                    target,
                    signature,
                    args,
                    destination,
                    ..
                } => {
                    if target.type_args.len() > 32 {
                        return Err("generic call argument limit exceeded".into());
                    }
                    for ty in target
                        .type_args
                        .iter()
                        .chain(signature.params.iter())
                        .chain(std::iter::once(&signature.result))
                    {
                        facts.validate_ty_with_budget(ty, budget)?;
                    }
                    for value in args {
                        operand(value, budget, facts)?;
                    }
                    path(destination, budget)?;
                }
                Terminator::Switch { condition, .. } => operand(condition, budget, facts)?,
                Terminator::SwitchVariant { place, .. } => path(place, budget)?,
                Terminator::Goto(_) | Terminator::Return | Terminator::Unreachable => {}
            }
        }
        Ok(())
    }
    pub fn place_ty(&self, place: &Place) -> Result<&Ty, String> {
        let ty = &self
            .locals
            .get(place.local.0)
            .ok_or("invalid CFG local")?
            .ty;
        ir::projected_ty(ty, &place.projection)
    }

    pub fn operand_ty(&self, operand: &Operand) -> Result<Ty, String> {
        match operand {
            Operand::Constant(Constant::Int(value)) => {
                value.validate()?;
                Ok(Ty::Int(value.ty))
            }
            Operand::Constant(Constant::Bool(_)) => Ok(Ty::Bool),
            Operand::Constant(Constant::Unit) => Ok(Ty::Unit),
            Operand::Constant(Constant::Enum { ty, variant }) => {
                ty.validate()?;
                if *variant >= ty.def().variants.len() {
                    return Err("invalid CFG enum variant".into());
                }
                if ty.has_payload() || ty.def().variants[*variant].shape != ir::VariantShape::Unit {
                    return Err("CFG enum constant requires unit-only enum".into());
                }
                Ok(Ty::Enum(ty.clone()))
            }
            Operand::Copy(place) => {
                let ty = self.place_ty(place)?;
                if !ty.is_copy() {
                    return Err("CFG Copy operand requires a Copy type".into());
                }
                Ok(ty.clone())
            }
            Operand::Move(place) => {
                let ty = self.place_ty(place)?;
                if !ty.is_copy()
                    && place.projection.iter().any(|part| {
                        matches!(part, Projection::ConstantIndex(_) | Projection::Deref)
                    })
                {
                    return Err(
                        "cannot move non-Copy value out of array index or dereference".into(),
                    );
                }
                Ok(ty.clone())
            }
        }
    }

    fn require_operand(&self, value: &Operand, want: &Ty) -> Result<(), String> {
        let got = self.operand_ty(value)?;
        if coerces(&got, want) {
            Ok(())
        } else {
            Err("CFG operand type mismatch".into())
        }
    }

    fn indexed_element_ty(&self, array: &Place, index: &Operand) -> Result<&Ty, String> {
        self.require_operand(index, &Ty::Int(IntTy::Usize))?;
        let array_ty = self.place_ty(array)?;
        validate_type(array_ty, 0)?;
        let Ty::Array { element, .. } = array_ty else {
            return Err("dynamic indexing requires array place".into());
        };
        Ok(element)
    }

    fn binary_ty(op: BinaryOp, left: Ty, right: Ty, count: Option<IntValue>) -> Result<Ty, String> {
        if left == Ty::Never || right == Ty::Never {
            return Err("CFG binary operator on never is unsupported".into());
        }
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            return Err("CFG short-circuit operator must be control flow".into());
        }
        if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
            let (Ty::Int(integer), Ty::Int(_)) = (&left, &right) else {
                return Err("CFG shift requires integer operands".into());
            };
            if let Some(count) = count {
                let negative =
                    count.ty.is_signed() && count.bits & (1_u64 << (count.ty.bits() - 1)) != 0;
                if negative || count.bits >= u64::from(integer.bits()) {
                    return Err("CFG constant shift count is negative or exceeds LHS width".into());
                }
            }
            return Ok(left);
        }
        let comparison = matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        );
        if !matches!(left, Ty::Int(_))
            && !(matches!(op, BinaryOp::Eq | BinaryOp::Ne) && matches!(left, Ty::Bool | Ty::Unit))
            && !(matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor)
                && left == Ty::Bool)
        {
            return Err("CFG binary operator requires compatible integer operands".into());
        }
        if left != right {
            return Err("CFG operand type mismatch".into());
        }
        Ok(if comparison { Ty::Bool } else { left })
    }

    fn compound_ty(&self, op: BinaryOp, target: &Ty, value: &Operand) -> Result<(), String> {
        let right = self.operand_ty(value)?;
        if !matches!(
            op,
            BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Rem
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::Shl
                | BinaryOp::Shr
        ) {
            return Err("invalid CFG compound assignment operator".into());
        }
        let count = match value {
            Operand::Constant(Constant::Int(count)) => Some(*count),
            _ => None,
        };
        let result = Self::binary_ty(op, target.clone(), right, count)?;
        if result != *target {
            return Err("CFG compound assignment result type mismatch".into());
        }
        Ok(())
    }

    fn rvalue_ty(&self, value: &Rvalue) -> Result<Ty, String> {
        match value {
            Rvalue::IntCast { value, to } => {
                if !matches!(self.operand_ty(value)?, Ty::Int(_)) {
                    return Err("CFG integer cast requires integer source".into());
                }
                Ok(Ty::Int(*to))
            }
            Rvalue::Borrow(id) => {
                let site = self.loan_sites.get(id.0).ok_or("invalid CFG loan site")?;
                let referent = self.place_ty(&site.place)?;
                if matches!(referent, Ty::Ref { .. }) {
                    return Err("unsupported: reference to reference".into());
                }
                Ok(Ty::Ref {
                    kind: site.kind,
                    referent: std::sync::Arc::new(referent.clone()),
                })
            }
            Rvalue::IndexCopy { array, index } => {
                let element = self.indexed_element_ty(array, index)?;
                if !element.is_copy() {
                    return Err("unsupported: non-Copy dynamic array element extraction".into());
                }
                Ok(element.clone())
            }
            Rvalue::Repeat { ty, value } => {
                validate_type(ty, 0)?;
                let Ty::Array { element, len } = ty else {
                    return Err("CFG repeat requires array type".into());
                };
                self.require_operand(value, element)?;
                if *len > 1 && !element.is_copy() {
                    return Err(
                        "CFG array repeat requires Copy element for length greater than one".into(),
                    );
                }
                Ok(ty.clone())
            }
            Rvalue::EnumVariant {
                ty,
                variant,
                fields,
            } => {
                validate_type(&Ty::Enum(ty.clone()), 0)?;
                let def = ty
                    .def()
                    .variants
                    .get(*variant)
                    .ok_or("invalid CFG enum variant")?;
                if fields.len() != def.fields.len() {
                    return Err("CFG enum initializer arity mismatch".into());
                }
                for (field, declaration) in fields.iter().zip(def.fields.iter()) {
                    self.require_operand(field, &declaration.ty)?;
                }
                Ok(Ty::Enum(ty.clone()))
            }
            Rvalue::Discriminant(place) => {
                let Ty::Enum(ty) = self.place_ty(place)? else {
                    return Err("CFG discriminant requires enum place".into());
                };
                ty.validate()?;
                Ok(Ty::Int(IntTy::I32))
            }
            Rvalue::Use(operand) => self.operand_ty(operand),
            Rvalue::Println(_) => Ok(Ty::Unit),
            Rvalue::Unary { op, value } => {
                let ty = self.operand_ty(value)?;
                match (op, &ty) {
                    (UnaryOp::Neg, Ty::Int(integer)) if integer.is_signed() => Ok(ty),
                    (UnaryOp::Not, Ty::Int(_) | Ty::Bool) => Ok(ty),
                    (UnaryOp::Not, Ty::Never) => Ok(Ty::Never),
                    _ => Err("CFG unary operator requires compatible integer or bool type".into()),
                }
            }
            Rvalue::Binary { op, left, right } => {
                let left_ty = self.operand_ty(left)?;
                let right_ty = self.operand_ty(right)?;
                Self::binary_ty(
                    *op,
                    left_ty,
                    right_ty,
                    match right {
                        Operand::Constant(Constant::Int(count)) => Some(*count),
                        _ => None,
                    },
                )
            }
            Rvalue::Aggregate { ty, fields } => {
                validate_type(ty, 0)?;
                if let Ty::Array { element, len } = ty {
                    if fields.len() != *len {
                        return Err("CFG array aggregate length mismatch".into());
                    }
                    for field in fields {
                        self.require_operand(field, element)?;
                    }
                    return Ok(ty.clone());
                }
                let count = ty
                    .field_count()
                    .ok_or("CFG aggregate requires aggregate type")?;
                if fields.len() != count {
                    return Err("CFG aggregate arity mismatch".into());
                }
                for (index, value) in fields.iter().enumerate() {
                    self.require_operand(
                        value,
                        ty.field_ty(index).ok_or("invalid CFG aggregate field")?,
                    )?;
                }
                Ok(ty.clone())
            }
        }
    }

    /// Verify representation invariants once before publication. This does not
    /// redo source type checking, prove initialization/liveness/ownership, check
    /// provenance IDs against an absent source arena, or resolve call targets.
    /// Reference calls additionally require whole-program noescape and matching
    /// callee-signature evidence at the trusted emission boundary.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_with_facts(ir::FactsCtx::Concrete)
    }
    pub fn validate_with_facts(&self, facts: ir::FactsCtx<'_>) -> Result<(), String> {
        for local in &self.locals {
            facts.validate_ty(&local.ty)?;
        }
        facts.validate_ty(&self.result)?;
        let block = |id: BlockId| -> Result<(), String> {
            if id.0 < self.blocks.len() {
                Ok(())
            } else {
                Err("invalid CFG block".into())
            }
        };
        block(self.entry)?;
        for local in &self.locals {
            validate_type(&local.ty, 0)?;
        }
        validate_type(&self.result, 0)?;
        if self.params.len() >= ir::MAX_PATTERN_PROJECTION_WORK {
            return Err("reference contract parameter resource limit exceeded".into());
        }
        let parameter_types = self
            .params
            .iter()
            .map(|param| self.place_ty(&place(*param)).cloned())
            .collect::<Result<Vec<_>, _>>()?;
        self.ref_contract.validate(&parameter_types, &self.result)?;
        drop(parameter_types);
        if self.loan_sites.len() > ir::MAX_LOAN_SITES {
            return Err("CFG loan site limit exceeded".into());
        }
        let mut origins = self.loan_sites.len();
        for param in &self.params {
            if matches!(self.place_ty(&place(*param))?, Ty::Ref { .. }) {
                origins += 1;
                if origins > ir::MAX_LOAN_SITES {
                    return Err("CFG combined reference origin limit exceeded".into());
                }
            }
        }
        for block in &self.blocks {
            if matches!(&block.terminator.value, Terminator::Call { signature, .. } if matches!(signature.result, Ty::Ref { .. }))
            {
                origins += 1;
                if origins > ir::MAX_LOAN_SITES {
                    return Err("CFG combined reference origin limit exceeded".into());
                }
            }
        }
        let mut loan_uses = vec![0usize; self.loan_sites.len()];
        for site in &self.loan_sites {
            self.place_ty(&site.place)?;
        }
        if self.place_ty(&place(self.return_local))? != &self.result {
            return Err("CFG return local type mismatch".into());
        }
        let mut params = std::collections::HashSet::new();
        for &param in &self.params {
            self.place_ty(&place(param))?;
            if !params.insert(param) {
                return Err("duplicate CFG parameter".into());
            }
            if param == self.return_local {
                return Err("CFG return local aliases parameter".into());
            }
        }
        drop(params);
        for item in &self.blocks {
            for statement in &item.statements {
                statement.value.visit_operands(&mut |operand| {
                    if let Operand::Constant(Constant::Enum { ty, .. }) = operand {
                        facts.validate_ty(&Ty::Enum(ty.clone()))?;
                    }
                    Ok(())
                })?;
                match &statement.value {
                    Statement::IndexAssignOp {
                        array,
                        index,
                        value,
                        op,
                    } => {
                        if array.projection.contains(&Projection::Deref) {
                            return Err(
                                "compound runtime indexing through dereference is unsupported"
                                    .into(),
                            );
                        }
                        let element = self.indexed_element_ty(array, index)?;
                        self.compound_ty(*op, element, value)?;
                    }
                    Statement::IndexAssign {
                        array,
                        index,
                        value,
                    } => {
                        let element = self.indexed_element_ty(array, index)?;
                        self.require_operand(value, element)?;
                    }
                    Statement::IndexInspect { array, index } => {
                        self.indexed_element_ty(array, index)?;
                    }
                    Statement::Inspect(place) => {
                        self.place_ty(place)?;
                    }
                    Statement::StorageLive(local) | Statement::StorageDead(local) => {
                        self.place_ty(&place(*local))?;
                    }
                    Statement::Assign { destination, value } => {
                        if let Rvalue::Borrow(id) = value {
                            let count = loan_uses.get_mut(id.0).ok_or("invalid CFG loan site")?;
                            *count += 1;
                            if *count != 1 {
                                return Err("CFG loan site used more than once".into());
                            }
                        }
                        let got = self.rvalue_ty(value)?;
                        facts.validate_ty(&got)?;
                        let want = self.place_ty(destination)?;
                        if !coerces(&got, want) {
                            return Err("CFG assignment type mismatch".into());
                        }
                    }
                }
            }
            item.terminator.value.visit_operands(&mut |operand| {
                if let Operand::Constant(Constant::Enum { ty, .. }) = operand {
                    facts.validate_ty(&Ty::Enum(ty.clone()))?;
                }
                Ok(())
            })?;
            match &item.terminator.value {
                Terminator::SwitchVariant {
                    place,
                    variant,
                    then_block,
                    else_block,
                } => {
                    if place.projection.contains(&Projection::Deref) {
                        return Err("unsupported: enum variant matching through dereference".into());
                    }
                    let Ty::Enum(ty) = self.place_ty(place)? else {
                        return Err("CFG variant switch requires enum place".into());
                    };
                    ty.validate()?;
                    if variant.owner != ty.def().id || variant.index >= ty.def().variants.len() {
                        return Err("invalid CFG variant switch identity".into());
                    }
                    block(*then_block)?;
                    block(*else_block)?;
                }
                Terminator::Goto(next) => block(*next)?,
                Terminator::Switch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    self.require_operand(condition, &Ty::Bool)?;
                    block(*then_block)?;
                    block(*else_block)?;
                }
                Terminator::Call {
                    target,
                    signature,
                    args,
                    destination,
                    next,
                    ..
                } => {
                    if target.type_args.len() > 32 {
                        return Err("generic call argument limit exceeded".into());
                    }
                    for ty in target
                        .type_args
                        .iter()
                        .chain(signature.params.iter())
                        .chain(std::iter::once(&signature.result))
                    {
                        facts.validate_ty(ty)?;
                    }
                    if signature.params.len() != args.len() {
                        return Err("CFG call arity mismatch".into());
                    }
                    for ty in &signature.params {
                        validate_type(ty, 0)?;
                    }
                    validate_type(&signature.result, 0)?;
                    signature
                        .ref_contract
                        .validate(&signature.params, &signature.result)?;
                    for (arg, ty) in args.iter().zip(&signature.params) {
                        self.require_operand(arg, ty)?;
                    }
                    let want = self.place_ty(destination)?;
                    if !coerces(&signature.result, want) {
                        return Err("CFG call destination type mismatch".into());
                    }
                    block(*next)?;
                }
                Terminator::Return | Terminator::Unreachable => {}
            }
        }
        if loan_uses.iter().any(|count| *count != 1) {
            return Err("CFG loan site must have exactly one borrow instruction".into());
        }
        Ok(())
    }
}

fn place(local: LocalId) -> Place {
    Place {
        local,
        projection: Vec::new(),
    }
}

struct PendingBlock {
    statements: Vec<Located<Statement>>,
    terminator: Option<Located<Terminator>>,
}

struct LoopTarget {
    id: LoopId,
    head: BlockId,
    exit: BlockId,
    destination: Option<Place>,
    scope_depth: usize,
}

struct Lowerer<'a> {
    budget: Option<&'a ir::PlanBudget>,
    resource_error: Option<String>,
    lowering_depth: usize,
    lowering_work: usize,
    origins: usize,
    loans: Vec<Option<LoanId>>,
    used_loans: std::collections::HashSet<LoanId>,
    source: &'a ir::CheckedFunction,
    result: Function,
    expressions: Vec<LocalId>,
    parameters: std::collections::HashSet<LocalId>,
    blocks: Vec<PendingBlock>,
    current: BlockId,
    scopes: Vec<Vec<LocalId>>,
    loops: Vec<LoopTarget>,
}

impl Lowerer<'_> {
    fn allocate(&mut self, nodes: usize, bytes: usize) -> bool {
        if self.resource_error.is_some() {
            return false;
        }
        if let Some(budget) = self.budget
            && let Err(error) = budget
                .charge_nodes(nodes)
                .and_then(|()| budget.charge_work(nodes))
                .and_then(|()| budget.retain_bytes(bytes))
        {
            self.resource_error = Some(error);
            return false;
        }
        true
    }
    fn enter_lowering(&mut self) -> Result<(), String> {
        if let Some(error) = &self.resource_error {
            return Err(error.clone());
        }
        if self.lowering_depth >= ir::MAX_PATTERN_PROJECTION_DEPTH {
            return Err("CFG lowering depth resource limit exceeded (128)".into());
        }
        if self.lowering_work >= ir::MAX_PATTERN_PROJECTION_WORK {
            return Err("CFG lowering work resource limit exceeded".into());
        }
        self.lowering_depth += 1;
        self.lowering_work += 1;
        Ok(())
    }

    fn temporary(&mut self, ty: Ty, origin: Option<ExprId>) -> Place {
        if !self.allocate(1, std::mem::size_of::<ir::LocalDecl>() + 32) {
            return place(self.result.return_local);
        }
        let local = LocalId(self.result.locals.len());
        self.result.locals.push(ir::LocalDecl {
            name: format!("$match{}", local.0),
            ty,
            mutable: true,
        });
        self.live(local, origin);
        place(local)
    }

    fn match_expression(
        &mut self,
        id: ExprId,
        scrutinee: ExprId,
        arms: &[ir::MatchArm],
        destination: Place,
    ) -> Result<(), String> {
        self.source
            .validate_match(scrutinee, arms, &self.parameters)?;
        let source = self.pattern_source(scrutinee)?;
        if source.projection.contains(&Projection::Deref)
            && arms
                .iter()
                .any(|arm| matches!(arm.pattern, ir::MatchPattern::EnumVariant { .. }))
        {
            return Err("unsupported: enum variant matching through dereference".into());
        }
        let completes = self.source.exprs[scrutinee.0].completion == ir::Completion::Returns;
        let join = self.block();
        for arm in arms {
            let selected = self.block();
            let next = self.block();
            self.scopes.push(Vec::new());
            let comparison = match &arm.pattern {
                ir::MatchPattern::Bool(value) => {
                    Some((Operand::Copy(source.clone()), Constant::Bool(*value)))
                }
                ir::MatchPattern::Int(value) => {
                    Some((Operand::Copy(source.clone()), Constant::Int(*value)))
                }
                _ => None,
            };
            if let ir::MatchPattern::EnumVariant { ty, variant, .. } = &arm.pattern
                && completes
                && ty.def().variants.len() > 1
            {
                self.terminate(
                    Terminator::SwitchVariant {
                        place: source.clone(),
                        variant: ir::VariantId {
                            owner: ty.def().id,
                            index: *variant,
                        },
                        then_block: selected,
                        else_block: next,
                    },
                    Some(id),
                )?;
            } else if let Some((left, right)) = comparison.filter(|_| completes) {
                let condition = self.temporary(Ty::Bool, Some(id));
                self.assign(
                    condition.clone(),
                    Rvalue::Binary {
                        op: BinaryOp::Eq,
                        left,
                        right: Operand::Constant(right),
                    },
                    Some(id),
                );
                self.terminate(
                    Terminator::Switch {
                        condition: Operand::Copy(condition),
                        then_block: selected,
                        else_block: next,
                    },
                    Some(id),
                )?;
            } else {
                // Keep dead arm bodies visible to conservative ownership
                // checking without introducing a runtime edge or tag read.
                self.terminate(
                    Terminator::Switch {
                        condition: Operand::Constant(Constant::Bool(true)),
                        then_block: selected,
                        else_block: next,
                    },
                    Some(id),
                )?;
            }
            let test_scope = self.scopes.pop().unwrap();
            self.current = selected;
            for local in test_scope.iter().rev() {
                self.statement(Statement::StorageDead(*local), Some(id));
            }
            self.scopes.push(Vec::new());
            if let ir::MatchPattern::Binding(local) = arm.pattern {
                if local == source.local {
                    return Err("match binding aliases scrutinee storage".into());
                }
                self.live(local, Some(id));
                if completes {
                    self.assign(
                        place(local),
                        Rvalue::Use(self.consume(source.clone())?),
                        Some(id),
                    );
                }
            }
            if let ir::MatchPattern::EnumVariant { bindings, .. } = &arm.pattern {
                for binding in bindings {
                    if binding.local == source.local {
                        return Err("enum binding aliases scrutinee storage".into());
                    }
                    self.live(binding.local, Some(id));
                    if completes {
                        let mut selected = source.clone();
                        selected.projection.extend_from_slice(&binding.projection);
                        self.assign(
                            place(binding.local),
                            Rvalue::Use(self.consume(selected)?),
                            Some(id),
                        );
                    }
                }
            }
            let value = self.expression(arm.body)?;
            self.assign(destination.clone(), Rvalue::Use(value), Some(id));
            self.cleanup(self.scopes.len() - 1, Some(id));
            self.scopes.pop();
            self.terminate(Terminator::Goto(join), Some(id))?;
            self.current = next;
            for local in test_scope.iter().rev() {
                self.statement(Statement::StorageDead(*local), Some(id));
            }
        }
        self.terminate(Terminator::Unreachable, Some(id))?;
        self.current = join;
        Ok(())
    }

    fn block(&mut self) -> BlockId {
        if !self.allocate(1, std::mem::size_of::<PendingBlock>()) {
            return BlockId(0);
        }
        let id = BlockId(self.blocks.len());
        self.blocks.push(PendingBlock {
            statements: Vec::new(),
            terminator: None,
        });
        id
    }

    fn statement(&mut self, value: Statement, origin: Option<ExprId>) {
        if !self.allocate(1, std::mem::size_of::<Located<Statement>>()) {
            return;
        }
        self.blocks[self.current.0]
            .statements
            .push(Located { value, origin });
    }

    fn terminate(&mut self, value: Terminator, origin: Option<ExprId>) -> Result<(), String> {
        if let Some(error) = &self.resource_error {
            return Err(error.clone());
        }
        let slot = &mut self.blocks[self.current.0].terminator;
        if slot.is_some() {
            return Err("CFG block already terminated".into());
        }
        *slot = Some(Located { value, origin });
        Ok(())
    }

    /// Keep lowering already type-checked dead syntax, but never attach an edge
    /// from an abrupt transfer into its lexical continuation.
    fn disconnected(&mut self) {
        self.current = self.block();
    }

    fn live(&mut self, local: LocalId, origin: Option<ExprId>) {
        self.statement(Statement::StorageLive(local), origin);
        self.scopes.last_mut().expect("lowering scope").push(local);
    }

    fn cleanup(&mut self, depth: usize, origin: Option<ExprId>) {
        let locals: Vec<_> = self.scopes[depth..]
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev().copied())
            .collect();
        for local in locals {
            self.statement(Statement::StorageDead(local), origin);
        }
    }

    fn assign(&mut self, destination: Place, value: Rvalue, origin: Option<ExprId>) {
        self.statement(Statement::Assign { destination, value }, origin);
    }

    fn consume(&self, place: Place) -> Result<Operand, String> {
        if self.result.place_ty(&place)?.is_copy() {
            Ok(Operand::Copy(place))
        } else {
            Ok(Operand::Move(place))
        }
    }

    fn dynamic_source(&mut self, base: ExprId, index: ExprId) -> Result<(Place, Operand), String> {
        // Static place prefixes are evaluated now. In particular a[0][i]
        // inspects a before evaluating i, unlike a[i]'s final array read.
        let mut root = base;
        for depth in 0..=ir::MAX_PATTERN_PROJECTION_DEPTH {
            let expression = self
                .source
                .exprs
                .get(root.0)
                .ok_or("invalid dynamic array base")?;
            match &expression.kind {
                ir::ExprKind::DynamicIndex { .. } => return Err("unsupported: chained dynamic array indexing; snapshot the intermediate array in a local".into()),
                ir::ExprKind::Index { base, .. } | ir::ExprKind::Field { base, .. } => {
                    if depth == ir::MAX_PATTERN_PROJECTION_DEPTH { return Err("dynamic array base projection depth exceeds limit 128".into()); }
                    root = *base;
                }
                _ => break,
            }
        }
        let array = self.pattern_source(base)?;
        if !matches!(self.result.place_ty(&array)?, Ty::Array { .. }) {
            return Err("dynamic indexing requires array place".into());
        }
        let index = self.expression(index)?;
        self.result
            .require_operand(&index, &Ty::Int(IntTy::Usize))?;
        Ok((array, index))
    }

    /// Evaluate the non-place root once, retaining field/index chains as
    /// addresses. Fields do not read their parent. An index inspects its array
    /// base even when the selected element is ignored, but never moves it.
    fn pattern_source(&mut self, id: ExprId) -> Result<Place, String> {
        let mut root = id;
        let mut projection = Vec::new();
        let mut source = loop {
            let expression = self
                .source
                .exprs
                .get(root.0)
                .ok_or("invalid pattern initializer expression")?;
            match &expression.kind {
                ir::ExprKind::Local(local) => break place(*local),
                ir::ExprKind::Deref { value } => {
                    if projection.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                        return Err("dereference projection depth exceeds limit 128".into());
                    }
                    projection.push(Projection::Deref);
                    root = *value;
                }
                ir::ExprKind::DynamicIndex { .. }
                    if projection
                        .iter()
                        .any(|part| matches!(part, Projection::ConstantIndex(_))) =>
                {
                    return Err("unsupported: chained dynamic array indexing; snapshot the intermediate array in a local".into());
                }
                ir::ExprKind::Field { base, index } | ir::ExprKind::Index { base, index } => {
                    if projection.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                        return Err("pattern initializer projection depth exceeds limit 128".into());
                    }
                    projection.push(if matches!(expression.kind, ir::ExprKind::Index { .. }) {
                        Projection::ConstantIndex(*index)
                    } else {
                        Projection::Field(*index)
                    });
                    root = *base;
                }
                _ => break self.materialize_expression(root)?,
            }
        };
        for part in projection.into_iter().rev() {
            if matches!(part, Projection::ConstantIndex(_)) {
                self.result.place_ty(&source)?;
                self.statement(Statement::Inspect(source.clone()), Some(id));
            }
            source.projection.push(part);
        }
        // Final structural metadata validation is not an additional value read.
        // Field-only pattern places remain addressable after partial moves.
        self.result.place_ty(&source)?;
        Ok(source)
    }

    fn inspect_index_prefixes(
        &mut self,
        destination: &Place,
        origin: Option<ExprId>,
    ) -> Result<(), String> {
        self.result.place_ty(destination)?;
        for (index, part) in destination.projection.iter().enumerate() {
            if matches!(part, Projection::ConstantIndex(_)) {
                self.statement(
                    Statement::Inspect(Place {
                        local: destination.local,
                        projection: destination.projection[..index].to_vec(),
                    }),
                    origin,
                );
            }
        }
        Ok(())
    }

    fn body(&mut self, body: &ir::Block, destination: Option<Place>) -> Result<(), String> {
        self.enter_lowering()?;
        let result = self.body_inner(body, destination);
        self.lowering_depth -= 1;
        result
    }

    fn body_inner(&mut self, body: &ir::Block, destination: Option<Place>) -> Result<(), String> {
        self.scopes.push(Vec::new());
        for statement in &body.statements {
            match statement {
                ir::Stmt::AssignOp { place, op, value } => {
                    let operand = self.expression(*value)?;
                    self.inspect_index_prefixes(place, Some(*value))?;
                    self.assign(
                        place.clone(),
                        Rvalue::Binary {
                            op: *op,
                            left: Operand::Copy(place.clone()),
                            right: operand,
                        },
                        Some(*value),
                    );
                }
                ir::Stmt::AssignIndexOp {
                    array,
                    index,
                    op,
                    value,
                } => {
                    let value = self.expression(*value)?;
                    self.inspect_index_prefixes(array, Some(*index))?;
                    let index_value = self.expression(*index)?;
                    self.statement(
                        Statement::IndexAssignOp {
                            array: array.clone(),
                            index: index_value,
                            value,
                            op: *op,
                        },
                        Some(*index),
                    );
                }
                ir::Stmt::AssignIndex {
                    array,
                    index,
                    value,
                } => {
                    // Own the RHS snapshot before any left-hand-side effects.
                    let value = self.expression(*value)?;
                    self.inspect_index_prefixes(array, Some(*index))?;
                    let index_value = self.expression(*index)?;
                    self.statement(
                        Statement::IndexAssign {
                            array: array.clone(),
                            index: index_value,
                            value,
                        },
                        Some(*index),
                    );
                }
                ir::Stmt::Declare { local } => {
                    self.result.place_ty(&place(*local))?;
                    if self.parameters.contains(local) {
                        return Err("declaration local aliases parameter".into());
                    }
                    self.live(*local, None);
                }
                ir::Stmt::Let { local, value } => {
                    let operand = self.expression(*value)?;
                    self.live(*local, Some(*value));
                    self.assign(place(*local), Rvalue::Use(operand), Some(*value));
                }
                ir::Stmt::LetPattern {
                    value,
                    ty,
                    bindings,
                } => {
                    self.source.validate_pattern_bindings_with_params(
                        ty,
                        bindings,
                        &self.parameters,
                    )?;
                    if bindings.is_empty()
                        && let ir::ExprKind::DynamicIndex { base, index } = self
                            .source
                            .exprs
                            .get(value.0)
                            .ok_or("invalid ignored dynamic index")?
                            .kind
                    {
                        let (array, index) = self.dynamic_source(base, index)?;
                        self.statement(Statement::IndexInspect { array, index }, Some(*value));
                        continue;
                    }
                    let source = self.pattern_source(*value)?;
                    let completes =
                        self.source.exprs[value.0].completion == ir::Completion::Returns;
                    for binding in bindings {
                        if binding.local == source.local {
                            return Err("pattern binding aliases initializer storage".into());
                        }
                        // The entire initializer runs in the old scope before
                        // any new binding starts its storage lifetime.
                        self.live(binding.local, Some(*value));
                        if completes {
                            let mut selected = source.clone();
                            selected.projection.extend_from_slice(&binding.projection);
                            let operand = self.consume(selected)?;
                            self.assign(place(binding.local), Rvalue::Use(operand), Some(*value));
                        }
                    }
                    // Wildcard/rest have no bindings, loads, moves or storage
                    // markers. A non-place initializer has still run once.
                    // Divergent initializers never produce projection loads.
                }
                ir::Stmt::Assign { local, value } => {
                    let operand = self.expression(*value)?;
                    self.assign(place(*local), Rvalue::Use(operand), Some(*value));
                }
                ir::Stmt::AssignPlace { place, value } => {
                    let operand = self.expression(*value)?;
                    self.inspect_index_prefixes(place, Some(*value))?;
                    self.assign(place.clone(), Rvalue::Use(operand), Some(*value));
                }
                ir::Stmt::Expr(value) => {
                    self.expression(*value)?;
                }
                ir::Stmt::Return(value) => {
                    let operand = match value {
                        Some(id) => self.expression(*id)?,
                        None => Operand::Constant(Constant::Unit),
                    };
                    self.assign(
                        place(self.result.return_local),
                        Rvalue::Use(operand),
                        *value,
                    );
                    self.cleanup(0, *value);
                    self.terminate(Terminator::Return, *value)?;
                    self.disconnected();
                }
                ir::Stmt::While {
                    id,
                    condition,
                    body,
                } => {
                    let head = self.block();
                    let run = self.block();
                    let condition_false = self.block();
                    let exit = self.block();
                    self.terminate(Terminator::Goto(head), Some(*condition))?;
                    self.loops.push(LoopTarget {
                        id: *id,
                        head,
                        exit,
                        destination: None,
                        scope_depth: self.scopes.len(),
                    });
                    self.current = head;
                    // The condition itself has an iteration-local temp scope.
                    self.scopes.push(Vec::new());
                    let condition_value = self.expression(*condition)?;
                    // The condition value is copied to the branch register by
                    // the consumer; StorageDead changes lifetime, not its bytes.
                    // Keep its temporary live through the Switch instead of
                    // ending it before the terminator.
                    self.terminate(
                        Terminator::Switch {
                            condition: condition_value,
                            then_block: run,
                            else_block: condition_false,
                        },
                        Some(*condition),
                    )?;
                    let condition_scope = self.scopes.pop().unwrap();
                    self.current = run;
                    for local in condition_scope.iter().rev() {
                        self.statement(Statement::StorageDead(*local), Some(*condition));
                    }
                    self.body(body, None)?;
                    self.terminate(Terminator::Goto(head), None)?;
                    self.current = condition_false;
                    for local in condition_scope.iter().rev() {
                        self.statement(Statement::StorageDead(*local), Some(*condition));
                    }
                    self.terminate(Terminator::Goto(exit), Some(*condition))?;
                    self.current = exit;
                    self.loops.pop();
                }
            }
        }
        if let Some(tail) = body.tail {
            let value = self.expression(tail)?;
            if let Some(destination) = destination {
                self.assign(destination, Rvalue::Use(value), Some(tail));
            }
        } else if let Some(destination) = destination {
            // A non-unit destination with no source tail is on a structurally
            // dead continuation after return/break/continue. Do not manufacture
            // a unit value of a different semantic type there.
            if self.result.place_ty(&destination)? == &Ty::Unit {
                self.assign(
                    destination,
                    Rvalue::Use(Operand::Constant(Constant::Unit)),
                    None,
                );
            }
        }
        self.cleanup(self.scopes.len() - 1, None);
        self.scopes.pop();
        Ok(())
    }

    fn expression(&mut self, id: ExprId) -> Result<Operand, String> {
        let storage = self.materialize_expression(id)?;
        self.consume(storage)
    }

    /// Evaluate once into an independent owned snapshot without consuming that
    /// snapshot. Projection can address it before selecting a field to consume.
    fn materialize_expression(&mut self, id: ExprId) -> Result<Place, String> {
        self.enter_lowering()?;
        // Keep long admitted cast chains out of the large aggregate/control
        // lowering frame. The same depth/work guard still covers every cast.
        let result = match self.source.exprs.get(id.0).map(|expr| &expr.kind) {
            Some(ir::ExprKind::IntCast { value, to }) => self.materialize_cast(id, *value, *to),
            _ => self.materialize_expression_inner(id),
        };
        self.lowering_depth -= 1;
        result
    }

    fn materialize_cast(&mut self, id: ExprId, value: ExprId, to: IntTy) -> Result<Place, String> {
        let destination = place(
            *self
                .expressions
                .get(id.0)
                .ok_or("missing expression local")?,
        );
        self.live(destination.local, Some(id));
        let value = self.expression(value)?;
        self.assign(destination.clone(), Rvalue::IntCast { value, to }, Some(id));
        Ok(destination)
    }

    fn materialize_expression_inner(&mut self, id: ExprId) -> Result<Place, String> {
        let expression = self
            .source
            .exprs
            .get(id.0)
            .ok_or("invalid checked expression")?;
        let destination = place(
            *self
                .expressions
                .get(id.0)
                .ok_or("missing expression local")?,
        );
        self.live(destination.local, Some(id));
        let origin = Some(id);
        match &expression.kind {
            ir::ExprKind::IntCast { value, to } => {
                let value = self.expression(*value)?;
                self.assign(
                    destination.clone(),
                    Rvalue::IntCast { value, to: *to },
                    origin,
                );
            }
            ir::ExprKind::Borrow { place, .. } => {
                let loan = self
                    .loans
                    .get(id.0)
                    .copied()
                    .flatten()
                    .ok_or("missing borrow site")?;
                if !self.used_loans.insert(loan) {
                    return Err("unsupported: repeated borrow expression materialization".into());
                }
                self.inspect_index_prefixes(place, origin)?;
                self.assign(destination.clone(), Rvalue::Borrow(loan), origin);
            }
            ir::ExprKind::Deref { .. } => {
                let source = self.pattern_source(id)?;
                let operand = self.consume(source)?;
                self.result.operand_ty(&operand)?;
                self.assign(destination.clone(), Rvalue::Use(operand), origin);
            }
            ir::ExprKind::DynamicIndex { base, index } => {
                if !expression.ty.is_copy() {
                    return Err("unsupported: non-Copy dynamic array element extraction; only direct let _ is supported".into());
                }
                let (array, index) = self.dynamic_source(*base, *index)?;
                self.assign(
                    destination.clone(),
                    Rvalue::IndexCopy { array, index },
                    origin,
                );
            }
            ir::ExprKind::Array(elements) => {
                let Ty::Array { len, .. } = &expression.ty else {
                    return Err("checked array expression requires array type".into());
                };
                if *len > ir::MAX_ARRAY_LEN || elements.len() != *len {
                    return Err("checked array length mismatch".into());
                }
                let fields = elements
                    .iter()
                    .map(|id| self.expression(*id))
                    .collect::<Result<Vec<_>, _>>()?;
                self.assign(
                    destination.clone(),
                    Rvalue::Aggregate {
                        ty: expression.ty.clone(),
                        fields,
                    },
                    origin,
                );
            }
            ir::ExprKind::Repeat { value, len } => {
                let Ty::Array { len: count, .. } = &expression.ty else {
                    return Err("checked repeat requires array type".into());
                };
                if len != count || *len > ir::MAX_ARRAY_LEN {
                    return Err("checked array repeat length mismatch".into());
                }
                // The operand is evaluated and consumed exactly once, even
                // when no array element is produced for a zero-length repeat.
                let value = self.expression(*value)?;
                self.assign(
                    destination.clone(),
                    Rvalue::Repeat {
                        ty: expression.ty.clone(),
                        value,
                    },
                    origin,
                );
            }
            ir::ExprKind::EnumVariant {
                ty,
                variant,
                fields,
            } => {
                ty.validate()?;
                if expression.ty != Ty::Enum(ty.clone()) || *variant >= ty.def().variants.len() {
                    return Err("invalid checked enum variant expression".into());
                }
                let mut operands = vec![None; ty.def().variants[*variant].fields.len()];
                for &(index, value) in fields {
                    if index >= operands.len() || operands[index].is_some() {
                        return Err("invalid checked enum initializer field".into());
                    }
                    operands[index] = Some(self.expression(value)?);
                }
                let fields = operands
                    .into_iter()
                    .map(|value| {
                        value.ok_or_else(|| "missing checked enum initializer field".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.assign(
                    destination.clone(),
                    Rvalue::EnumVariant {
                        ty: ty.clone(),
                        variant: *variant,
                        fields,
                    },
                    origin,
                );
            }
            ir::ExprKind::Match { scrutinee, arms } => {
                self.match_expression(id, *scrutinee, arms, destination.clone())?;
            }
            ir::ExprKind::Int(value) => self.assign(
                destination.clone(),
                Rvalue::Use(Operand::Constant(Constant::Int(*value))),
                origin,
            ),
            ir::ExprKind::Bool(value) => self.assign(
                destination.clone(),
                Rvalue::Use(Operand::Constant(Constant::Bool(*value))),
                origin,
            ),
            ir::ExprKind::Local(local) => self.assign(
                destination.clone(),
                Rvalue::Use(self.consume(place(*local))?),
                origin,
            ),
            ir::ExprKind::Field { .. } | ir::ExprKind::Index { .. } => {
                let p = self.pattern_source(id)?;
                self.assign(destination.clone(), Rvalue::Use(self.consume(p)?), origin);
            }
            ir::ExprKind::Tuple(fields) => {
                let operands = fields
                    .iter()
                    .map(|id| self.expression(*id))
                    .collect::<Result<Vec<_>, _>>()?;
                self.assign(
                    destination.clone(),
                    Rvalue::Aggregate {
                        ty: expression.ty.clone(),
                        fields: operands,
                    },
                    origin,
                );
            }
            ir::ExprKind::Struct { ty, fields } => {
                if expression.ty != Ty::Struct(ty.clone()) {
                    return Err("checked struct expression type mismatch".into());
                }
                let mut operands = vec![None; ty.def().fields.len()];
                for &(index, value) in fields {
                    if index >= operands.len() || operands[index].is_some() {
                        return Err("invalid checked struct initializer field".into());
                    }
                    // Evaluate in source order. Only the resulting independent
                    // snapshots are permuted into declaration order below.
                    operands[index] = Some(self.expression(value)?);
                }
                let fields = operands
                    .into_iter()
                    .map(|value| {
                        value.ok_or_else(|| "missing checked struct initializer field".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.assign(
                    destination.clone(),
                    Rvalue::Aggregate {
                        ty: expression.ty.clone(),
                        fields,
                    },
                    origin,
                );
            }
            ir::ExprKind::Unary { op, value } => {
                let operand = self.expression(*value)?;
                self.assign(
                    destination.clone(),
                    Rvalue::Unary {
                        op: *op,
                        value: operand,
                    },
                    origin,
                );
            }
            ir::ExprKind::Binary { op, left, right }
                if matches!(op, BinaryOp::And | BinaryOp::Or) =>
            {
                let left = self.expression(*left)?;
                let evaluate = self.block();
                let shortcut = self.block();
                let join = self.block();
                let (then_block, else_block) = if matches!(op, BinaryOp::And) {
                    (evaluate, shortcut)
                } else {
                    (shortcut, evaluate)
                };
                self.terminate(
                    Terminator::Switch {
                        condition: left,
                        then_block,
                        else_block,
                    },
                    origin,
                )?;
                self.current = shortcut;
                self.assign(
                    destination.clone(),
                    Rvalue::Use(Operand::Constant(Constant::Bool(matches!(
                        op,
                        BinaryOp::Or
                    )))),
                    origin,
                );
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = evaluate;
                self.scopes.push(Vec::new());
                let right = self.expression(*right)?;
                self.assign(destination.clone(), Rvalue::Use(right), origin);
                self.cleanup(self.scopes.len() - 1, origin);
                self.scopes.pop();
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = join;
            }
            ir::ExprKind::Binary { op, left, right } => {
                let left = self.expression(*left)?;
                let right = self.expression(*right)?;
                self.assign(
                    destination.clone(),
                    Rvalue::Binary {
                        op: *op,
                        left,
                        right,
                    },
                    origin,
                );
            }
            ir::ExprKind::Call {
                target,
                name,
                args,
                signature,
            } => {
                if matches!(signature.result, Ty::Ref { .. }) {
                    if self.origins >= ir::MAX_LOAN_SITES {
                        return Err("CFG combined reference origin limit exceeded".into());
                    }
                    self.origins += 1;
                }
                let args = args
                    .iter()
                    .map(|id| self.expression(*id))
                    .collect::<Result<Vec<_>, _>>()?;
                let next = self.block();
                self.terminate(
                    Terminator::Call {
                        target: target.clone(),
                        name: name.clone(),
                        signature: signature.clone(),
                        args,
                        destination: destination.clone(),
                        next,
                    },
                    origin,
                )?;
                self.current = next;
            }
            ir::ExprKind::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.expression(*condition)?;
                let yes = self.block();
                let no = self.block();
                let join = self.block();
                self.terminate(
                    Terminator::Switch {
                        condition,
                        then_block: yes,
                        else_block: no,
                    },
                    origin,
                )?;
                self.current = yes;
                self.body(then_block, Some(destination.clone()))?;
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = no;
                self.body(else_block, Some(destination.clone()))?;
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = join;
            }
            ir::ExprKind::Block(body) => self.body(body, Some(destination.clone()))?,
            ir::ExprKind::Loop { id: loop_id, body } => {
                let head = self.block();
                let exit = self.block();
                self.terminate(Terminator::Goto(head), origin)?;
                self.loops.push(LoopTarget {
                    id: *loop_id,
                    head,
                    exit,
                    destination: Some(destination.clone()),
                    scope_depth: self.scopes.len(),
                });
                self.current = head;
                self.body(body, None)?;
                self.terminate(Terminator::Goto(head), origin)?;
                self.loops.pop();
                self.current = exit;
            }
            ir::ExprKind::Break { target, value } => {
                let value = match value {
                    Some(value) => self.expression(*value)?,
                    None => Operand::Constant(Constant::Unit),
                };
                let frame = self
                    .loops
                    .iter()
                    .rev()
                    .find(|frame| frame.id == *target)
                    .ok_or("missing CFG break target")?;
                let (exit, depth, result) =
                    (frame.exit, frame.scope_depth, frame.destination.clone());
                if let Some(result) = result {
                    self.assign(result, Rvalue::Use(value), origin);
                }
                self.cleanup(depth, origin);
                self.terminate(Terminator::Goto(exit), origin)?;
                self.disconnected();
            }
            ir::ExprKind::Continue { target } => {
                let frame = self
                    .loops
                    .iter()
                    .rev()
                    .find(|frame| frame.id == *target)
                    .ok_or("missing CFG continue target")?;
                let (head, depth) = (frame.head, frame.scope_depth);
                self.cleanup(depth, origin);
                self.terminate(Terminator::Goto(head), origin)?;
                self.disconnected();
            }
            ir::ExprKind::Println(text) => {
                self.assign(destination.clone(), Rvalue::Println(text.clone()), origin)
            }
        }
        Ok(destination)
    }
}

/// Lower without modifying the checked input. Each value transfer selects Copy
/// or Move from its projected type. The result still requires ownership analysis.
pub fn lower(source: &ir::CheckedFunction) -> Result<Function, String> {
    lower_with_facts(source, ir::FactsCtx::Concrete, None)
}
pub fn lower_with_budget(
    source: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<Function, String> {
    lower_with_facts(source, ir::FactsCtx::Concrete, Some(budget))
}
fn lower_with_facts(
    source: &ir::CheckedFunction,
    facts: ir::FactsCtx<'_>,
    budget: Option<&ir::PlanBudget>,
) -> Result<Function, String> {
    if let Some(budget) = budget {
        source.preflight_plan(facts, budget)?;
        budget.retain_bytes(
            source
                .exprs
                .len()
                .checked_add(source.locals.len())
                .and_then(|n| n.checked_add(1))
                .and_then(|n| n.checked_mul(std::mem::size_of::<ir::LocalDecl>() + 32))
                .ok_or("CFG allocation overflow")?,
        )?;
        budget.charge_nodes(1)?;
        budget.retain_bytes(std::mem::size_of::<PendingBlock>())?;
    }
    source.validate_with_facts(facts)?;
    let arena_size = source
        .exprs
        .len()
        .checked_add(source.locals.len())
        .and_then(|count| count.checked_add(1))
        .ok_or("CFG lowering arena resource limit exceeded")?;
    if arena_size > ir::MAX_PATTERN_PROJECTION_WORK {
        return Err("CFG lowering arena resource limit exceeded".into());
    }
    let mut formal_origins = 0usize;
    for param in &source.params {
        let ty = &source
            .locals
            .get(param.0)
            .ok_or("invalid reference parameter local")?
            .ty;
        if matches!(ty, Ty::Ref { .. }) {
            formal_origins += 1;
            if formal_origins > ir::MAX_LOAN_SITES {
                return Err("CFG combined reference origin limit exceeded".into());
            }
        }
    }
    let formal_parameter_origins = formal_origins;
    for expression in &source.exprs {
        if matches!(expression.kind, ir::ExprKind::Call { .. })
            && matches!(expression.ty, Ty::Ref { .. })
        {
            formal_origins += 1;
            if formal_origins > ir::MAX_LOAN_SITES {
                return Err("CFG combined reference origin limit exceeded".into());
            }
        }
    }
    let mut loan_sites = Vec::new();
    let mut loans = vec![None; source.exprs.len()];
    for (index, expression) in source.exprs.iter().enumerate() {
        if let ir::ExprKind::Borrow { kind, place } = &expression.kind {
            if loan_sites.len() >= ir::MAX_LOAN_SITES - formal_origins {
                return Err("CFG loan site limit exceeded".into());
            }
            loans[index] = Some(LoanId(loan_sites.len()));
            loan_sites.push(LoanSite {
                kind: *kind,
                place: place.clone(),
                origin: Some(ExprId(index)),
            });
        }
    }
    let mut locals = source.locals.clone();
    let expressions = source
        .exprs
        .iter()
        .enumerate()
        .map(|(index, expression)| {
            let id = LocalId(locals.len());
            locals.push(ir::LocalDecl {
                name: format!("$expr{index}"),
                ty: expression.ty.clone(),
                mutable: true,
            });
            id
        })
        .collect();
    let return_local = LocalId(locals.len());
    locals.push(ir::LocalDecl {
        name: "$return".into(),
        ty: source.result.clone(),
        mutable: true,
    });
    let mut lowerer = Lowerer {
        budget,
        resource_error: None,
        lowering_depth: 0,
        lowering_work: 0,
        origins: formal_parameter_origins + loan_sites.len(),
        loans,
        used_loans: std::collections::HashSet::new(),
        source,
        result: Function {
            ref_contract: source.ref_contract.clone(),
            loan_sites,
            name: source.name.clone(),
            params: source.params.clone(),
            result: source.result.clone(),
            locals,
            blocks: Vec::new(),
            entry: BlockId(0),
            return_local,
        },
        expressions,
        parameters: source.params.iter().copied().collect(),
        blocks: vec![PendingBlock {
            statements: Vec::new(),
            terminator: None,
        }],
        current: BlockId(0),
        scopes: vec![source.params.clone()],
        loops: Vec::new(),
    };
    lowerer.statement(Statement::StorageLive(return_local), None);
    if source.result == Ty::Unit {
        lowerer.assign(
            place(return_local),
            Rvalue::Use(Operand::Constant(Constant::Unit)),
            None,
        );
    }
    let body_result = lowerer.body(&source.body, Some(place(return_local)));
    if let Some(error) = lowerer.resource_error.take() {
        return Err(error);
    }
    body_result?;
    lowerer.cleanup(0, None);
    lowerer.terminate(Terminator::Return, None)?;
    lowerer.result.blocks = lowerer
        .blocks
        .into_iter()
        .map(|block| BasicBlock {
            statements: block.statements,
            terminator: block.terminator.unwrap_or(Located {
                value: Terminator::Unreachable,
                origin: None,
            }),
        })
        .collect();
    Ok(lowerer.result)
}

#[cfg(test)]
mod tests {
    struct NoNominals;
    impl ir::SubstitutionCx for NoNominals {
        fn nominal_application(&self, _: ir::NominalAppId) -> Result<ir::NominalAppInfo, String> {
            Err("unexpected nominal".into())
        }
        fn intern_nominal(
            &mut self,
            _: ir::DeclId,
            _: &[Ty],
            _: ir::FactsCtx<'_>,
            _: &ir::PlanBudget,
        ) -> Result<Ty, String> {
            Err("unexpected nominal".into())
        }
        fn instantiate_signature(
            &mut self,
            _: &ir::CallTarget,
            _: ir::FactsCtx<'_>,
            _: &ir::PlanBudget,
        ) -> Result<ir::Signature, String> {
            Err("unexpected call".into())
        }
    }
    #[test]
    fn associated_template_proof_covers_impl_and_method_parameters_before_closed_emission() {
        let budget = ir::PlanBudget::default();
        let scopes = [test_decl(), test_decl()].map(|declaration| {
            std::sync::Arc::new(ir::ParamEnv::new(ir::BinderId { declaration }, 1).unwrap())
        });
        let env = std::sync::Arc::new(ir::GenericEnv::new(&scopes, &budget).unwrap());
        let types = env.ordered_params().map(Ty::Param).collect::<Vec<_>>();
        let result = Ty::Tuple(types.clone().into());
        let source = ir::CheckedFunction {
            ref_contract: Default::default(),
            name: "associated".into(),
            params: vec![LocalId(0), LocalId(1)],
            result: result.clone(),
            locals: types
                .iter()
                .enumerate()
                .map(|(index, ty)| ir::LocalDecl {
                    name: format!("arg{index}"),
                    ty: ty.clone(),
                    mutable: false,
                })
                .collect(),
            exprs: vec![
                expr(types[0].clone(), ir::ExprKind::Local(LocalId(0))),
                expr(types[1].clone(), ir::ExprKind::Local(LocalId(1))),
                expr(result, ir::ExprKind::Tuple(vec![ExprId(0), ExprId(1)])),
            ],
            body: ir::Block {
                statements: vec![],
                tail: Some(ExprId(2)),
            },
        };
        assert!(lower(&source).is_err());
        let template =
            lower_template(std::sync::Arc::new(source.clone()), env.clone(), &budget).unwrap();
        assert!(std::sync::Arc::ptr_eq(template.env(), &env));
        let verified = super::super::ownership::verify_template(template).unwrap();
        let instance = ir::instantiate_checked(
            &verified,
            &[Ty::Int(IntTy::I32), Ty::Bool],
            &mut NoNominals,
            &budget,
        )
        .unwrap();
        assert_eq!(
            instance.result,
            Ty::Tuple(vec![Ty::Int(IntTy::I32), Ty::Bool].into())
        );
        let function = lower_with_budget(&instance, &budget).unwrap();
        assert!(
            super::super::ownership::analyze_closed(
                close(std::sync::Arc::new(function), &budget).unwrap()
            )
            .is_ok()
        );
        let incomplete = std::sync::Arc::new(ir::GenericEnv::new(&scopes[..1], &budget).unwrap());
        assert!(lower_template(std::sync::Arc::new(source.clone()), incomplete, &budget).is_err());
        let mut duplicate = source;
        duplicate.exprs[1] = expr(types[0].clone(), ir::ExprKind::Local(LocalId(0)));
        duplicate.result = Ty::Tuple(vec![types[0].clone(), types[0].clone()].into());
        duplicate.exprs[2].ty = duplicate.result.clone();
        assert!(
            super::super::ownership::verify_template(
                lower_template(std::sync::Arc::new(duplicate), env, &budget).unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn generic_templates_prove_moves_before_relowering_concrete_copy_instances() {
        let budget = ir::PlanBudget::default();
        let scope = std::sync::Arc::new(
            ir::ParamEnv::new(
                ir::BinderId {
                    declaration: test_decl(),
                },
                1,
            )
            .unwrap(),
        );
        let ty = Ty::Param(ir::ParamId {
            owner: scope.binder(),
            index: 0,
        });
        let env = std::sync::Arc::new(ir::GenericEnv::new(&[scope], &budget).unwrap());
        let source = ir::CheckedFunction {
            ref_contract: Default::default(),
            name: "id".into(),
            params: vec![LocalId(0)],
            result: ty.clone(),
            locals: vec![ir::LocalDecl {
                name: "x".into(),
                ty: ty.clone(),
                mutable: false,
            }],
            exprs: vec![expr(ty.clone(), ir::ExprKind::Local(LocalId(0)))],
            body: ir::Block {
                statements: vec![],
                tail: Some(ExprId(0)),
            },
        };
        assert!(lower(&source).is_err());
        let template =
            lower_template(std::sync::Arc::new(source.clone()), env.clone(), &budget).unwrap();
        assert!(template.function().blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value,Statement::Assign { value:Rvalue::Use(Operand::Move(place)),.. } if place.local==LocalId(0))));
        let verified = super::super::ownership::verify_template(template).unwrap();
        assert!(
            ir::instantiate_checked(
                &verified,
                &[Ty::Int(IntTy::I32)],
                &mut NoNominals,
                &ir::PlanBudget::default()
            )
            .is_err()
        );
        let instance =
            ir::instantiate_checked(&verified, &[Ty::Int(IntTy::I32)], &mut NoNominals, &budget)
                .unwrap();
        let concrete = lower_with_budget(&instance, &budget).unwrap();
        assert!(concrete.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value,Statement::Assign { value:Rvalue::Use(Operand::Copy(place)),.. } if place.local==LocalId(0))));
        assert!(
            super::super::ownership::analyze_closed(
                close(std::sync::Arc::new(concrete), &budget).unwrap()
            )
            .is_ok()
        );
        let mut duplicate = source;
        duplicate.result = Ty::Tuple(std::sync::Arc::from([ty.clone(), ty.clone()]));
        duplicate
            .exprs
            .push(expr(ty, ir::ExprKind::Local(LocalId(0))));
        duplicate.exprs.push(expr(
            duplicate.result.clone(),
            ir::ExprKind::Tuple(vec![ExprId(0), ExprId(1)]),
        ));
        duplicate.body.tail = Some(ExprId(2));
        let invalid = lower_template(std::sync::Arc::new(duplicate), env, &budget).unwrap();
        assert!(super::super::ownership::verify_template(invalid).is_err());
        let app = ir::NominalAppId {
            snapshot: ir::SnapshotId::allocate().unwrap(),
            index: 0,
        };
        let declaration = test_decl();
        let enumeration = |arguments: std::sync::Arc<[Ty]>| {
            ir::EnumTy::new(ir::EnumDef {
                id: app,
                source_decl: declaration,
                type_args: arguments,
                name: "UnitEnum".into(),
                module: String::new(),
                public: false,
                variants: vec![ir::VariantDef {
                    name: "A".into(),
                    shape: ir::VariantShape::Unit,
                    fields: std::sync::Arc::from([]),
                }]
                .into(),
            })
        };
        let closed_enum = enumeration(std::sync::Arc::from([]));
        let forged = enumeration(std::sync::Arc::from([Ty::Param(ir::ParamId {
            owner: ir::BinderId {
                declaration: test_decl(),
            },
            index: 0,
        })]));
        let aggregate = Ty::Tuple(std::sync::Arc::from([Ty::Enum(closed_enum)]));
        let malformed = Function {
            ref_contract: Default::default(),
            loan_sites: vec![],
            name: "forged".into(),
            params: vec![],
            result: Ty::Unit,
            locals: vec![
                ir::LocalDecl {
                    name: "ret".into(),
                    ty: Ty::Unit,
                    mutable: true,
                },
                ir::LocalDecl {
                    name: "aggregate".into(),
                    ty: aggregate.clone(),
                    mutable: true,
                },
            ],
            entry: BlockId(0),
            return_local: LocalId(0),
            blocks: vec![
                BasicBlock {
                    statements: vec![],
                    terminator: Located {
                        value: Terminator::Return,
                        origin: None,
                    },
                },
                BasicBlock {
                    statements: vec![Located {
                        value: Statement::Assign {
                            destination: place(LocalId(1)),
                            value: Rvalue::Aggregate {
                                ty: aggregate,
                                fields: vec![Operand::Constant(Constant::Enum {
                                    ty: forged,
                                    variant: 0,
                                })],
                            },
                        },
                        origin: None,
                    }],
                    terminator: Located {
                        value: Terminator::Unreachable,
                        origin: None,
                    },
                },
            ],
        };
        assert!(
            malformed
                .validate_with_facts(ir::FactsCtx::Concrete)
                .unwrap_err()
                .contains("authoritative binder")
        );
        assert!(close(std::sync::Arc::new(malformed), &budget).is_err());
    }
    #[test]
    fn generic_instantiation_reinterns_variant_owners_and_resolved_call_metadata() {
        let budget = ir::PlanBudget::default();
        let declaration = test_decl();
        let callee = test_decl();
        let scope = std::sync::Arc::new(
            ir::ParamEnv::new(
                ir::BinderId {
                    declaration: test_decl(),
                },
                1,
            )
            .unwrap(),
        );
        let param = Ty::Param(ir::ParamId {
            owner: scope.binder(),
            index: 0,
        });
        let env = std::sync::Arc::new(ir::GenericEnv::new(&[scope], &budget).unwrap());
        let snapshot = ir::SnapshotId::allocate().unwrap();
        let make = |index, element: Ty| {
            ir::EnumTy::new(ir::EnumDef {
                id: ir::NominalAppId { snapshot, index },
                source_decl: declaration,
                type_args: std::sync::Arc::from([element.clone()]),
                name: "E".into(),
                module: String::new(),
                public: false,
                variants: vec![ir::VariantDef {
                    name: "A".into(),
                    shape: ir::VariantShape::Tuple,
                    fields: vec![ir::FieldDef {
                        name: "0".into(),
                        ty: element,
                        public: false,
                    }]
                    .into(),
                }]
                .into(),
            })
        };
        let symbolic = make(0, param.clone());
        let concrete = make(1, Ty::Int(IntTy::I32));
        struct Nominals {
            symbolic: ir::EnumTy,
            concrete: ir::EnumTy,
            callee: ir::DeclId,
        }
        impl ir::SubstitutionCx for Nominals {
            fn nominal_application(
                &self,
                id: ir::NominalAppId,
            ) -> Result<ir::NominalAppInfo, String> {
                let value = if id == self.symbolic.def().id {
                    &self.symbolic
                } else if id == self.concrete.def().id {
                    &self.concrete
                } else {
                    return Err("unknown application".into());
                };
                Ok(ir::NominalAppInfo {
                    source_decl: value.def().source_decl,
                    type_args: value.def().type_args.clone(),
                })
            }
            fn intern_nominal(
                &mut self,
                declaration: ir::DeclId,
                args: &[Ty],
                caller: ir::FactsCtx<'_>,
                budget: &ir::PlanBudget,
            ) -> Result<Ty, String> {
                budget.charge_work(1)?;
                if declaration != self.concrete.def().source_decl || args != [Ty::Int(IntTy::I32)] {
                    return Err("unexpected nominal arguments".into());
                }
                caller.validate_ty(&args[0])?;
                Ok(Ty::Enum(self.concrete.clone()))
            }
            fn instantiate_signature(
                &mut self,
                target: &ir::CallTarget,
                _: ir::FactsCtx<'_>,
                budget: &ir::PlanBudget,
            ) -> Result<ir::Signature, String> {
                budget.charge_work(1)?;
                if target.declaration != self.callee || target.type_args.len() != 1 {
                    return Err("unknown callee".into());
                }
                Ok(ir::Signature {
                    ref_contract: Default::default(),
                    params: target.type_args.to_vec(),
                    result: target.type_args[0].clone(),
                })
            }
        }
        let source = ir::CheckedFunction {
            ref_contract: Default::default(),
            name: "extract".into(),
            params: vec![LocalId(0)],
            result: param.clone(),
            locals: vec![
                ir::LocalDecl {
                    name: "e".into(),
                    ty: Ty::Enum(symbolic.clone()),
                    mutable: false,
                },
                ir::LocalDecl {
                    name: "x".into(),
                    ty: param.clone(),
                    mutable: false,
                },
            ],
            exprs: vec![
                expr(Ty::Enum(symbolic.clone()), ir::ExprKind::Local(LocalId(0))),
                expr(param.clone(), ir::ExprKind::Local(LocalId(1))),
                expr(
                    param.clone(),
                    ir::ExprKind::Call {
                        target: ir::CallTarget {
                            declaration: callee,
                            type_args: std::sync::Arc::from([param.clone()]),
                        },
                        name: "identity".into(),
                        args: vec![ExprId(1)],
                        signature: ir::Signature {
                            ref_contract: Default::default(),
                            params: vec![param.clone()],
                            result: param.clone(),
                        },
                    },
                ),
                expr(
                    param,
                    ir::ExprKind::Match {
                        scrutinee: ExprId(0),
                        arms: vec![ir::MatchArm {
                            pattern: ir::MatchPattern::EnumVariant {
                                ty: symbolic.clone(),
                                variant: 0,
                                bindings: vec![ir::PatternBinding {
                                    local: LocalId(1),
                                    projection: vec![
                                        Projection::Downcast(ir::VariantId {
                                            owner: symbolic.def().id,
                                            index: 0,
                                        }),
                                        Projection::Field(0),
                                    ],
                                }],
                            },
                            body: ExprId(2),
                        }],
                    },
                ),
            ],
            body: ir::Block {
                statements: vec![],
                tail: Some(ExprId(3)),
            },
        };
        let verified = super::super::ownership::verify_template(
            lower_template(std::sync::Arc::new(source), env, &budget).unwrap(),
        )
        .unwrap();
        let mut cx = Nominals {
            symbolic,
            concrete: concrete.clone(),
            callee,
        };
        let mut instance =
            ir::instantiate_checked(&verified, &[Ty::Int(IntTy::I32)], &mut cx, &budget).unwrap();
        let ir::ExprKind::Call {
            target, signature, ..
        } = &instance.exprs[2].kind
        else {
            unreachable!()
        };
        assert_eq!(target.type_args.as_ref(), [Ty::Int(IntTy::I32)]);
        assert_eq!(signature.result, Ty::Int(IntTy::I32));
        let ir::ExprKind::Match { arms, .. } = &instance.exprs[3].kind else {
            unreachable!()
        };
        let ir::MatchPattern::EnumVariant { ty, bindings, .. } = &arms[0].pattern else {
            unreachable!()
        };
        assert_eq!(ty.def().id, concrete.def().id);
        assert_eq!(
            bindings[0].projection[0],
            Projection::Downcast(ir::VariantId {
                owner: concrete.def().id,
                index: 0
            })
        );
        assert!(
            super::super::ownership::analyze(&lower_with_budget(&instance, &budget).unwrap())
                .passed()
        );
        let ir::ExprKind::Match { arms, .. } = &mut instance.exprs[3].kind else {
            unreachable!()
        };
        let ir::MatchPattern::EnumVariant { bindings, .. } = &mut arms[0].pattern else {
            unreachable!()
        };
        bindings[0].projection[0] = Projection::Downcast(ir::VariantId {
            owner: cx.symbolic.def().id,
            index: 0,
        });
        assert!(instance.validate().is_err());
    }
    fn test_decl() -> ir::DeclId {
        ir::DeclId {
            snapshot: ir::SnapshotId::allocate().unwrap(),
            index: 0,
        }
    }
    fn test_target() -> ir::CallTarget {
        ir::CallTarget {
            declaration: test_decl(),
            type_args: std::sync::Arc::from([]),
        }
    }
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn mutable_returns_move_handles_and_reject_dead_copy_mut() {
        let program = super::super::frontend::parse(
            "fn id<'a>(x:&'a mut i32)->&'a mut i32{x} fn main(){let mut x=7;let n=*id(&mut x);}",
        )
        .unwrap();
        let signatures = super::super::frontend::signature_table(&program).unwrap();
        let source =
            super::super::frontend::type_function_with_signatures(&program, 0, &signatures)
                .unwrap();
        let callee = nominal_lowered(&source);
        assert!(std::sync::Arc::ptr_eq(
            &source.ref_contract,
            &callee.ref_contract
        ));
        assert!(matches!(
            callee.ref_contract.returns,
            ir::ReturnContract::MutInputPaths(_)
        ));
        assert!(callee.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value, Statement::Assign { destination, value: Rvalue::Use(Operand::Move(_)) } if destination.local == callee.return_local)));
        let mut malformed = callee.clone();
        malformed.blocks.push(BasicBlock {
            statements: vec![Located {
                value: Statement::Assign {
                    destination: place(callee.return_local),
                    value: Rvalue::Use(Operand::Copy(place(callee.params[0]))),
                },
                origin: None,
            }],
            terminator: Located {
                value: Terminator::Unreachable,
                origin: None,
            },
        });
        assert!(malformed.validate().unwrap_err().contains("Copy type"));
        let mut forged = callee.clone();
        let mut contract = forged.ref_contract.as_ref().clone();
        contract.returns =
            ir::ReturnContract::SharedInputPaths(std::sync::Arc::from([ir::InputPath {
                param: 0,
                projection: std::sync::Arc::from([]),
            }]));
        forged.ref_contract = std::sync::Arc::new(contract);
        assert!(forged.validate().unwrap_err().contains("kind mismatch"));
        let source =
            super::super::frontend::type_function_with_signatures(&program, 1, &signatures)
                .unwrap();
        let caller = nominal_lowered(&source);
        let call = source
            .exprs
            .iter()
            .find_map(|expr| {
                if let ir::ExprKind::Deref { value } = expr.kind {
                    Some(value)
                } else {
                    None
                }
            })
            .unwrap();
        let carrier = LocalId(source.locals.len() + call.0);
        assert!(caller.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value, Statement::Assign { value: Rvalue::Use(Operand::Copy(place)), .. } if place.local == carrier && place.projection == [Projection::Deref])));
        assert!(!caller.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value, Statement::Assign { value: Rvalue::Use(Operand::Copy(place)), .. } if place.local == carrier && place.projection.is_empty())));
    }

    #[test]
    fn shared_return_contracts_keep_canonical_arcs_and_real_call_carriers() {
        let program = super::super::frontend::parse(
            "fn id<'a>(x:&'a i32)->&'a i32{x} fn main(){let x=7;let r=id(&x);let n=*id(&x);}",
        )
        .unwrap();
        let signatures = super::super::frontend::signature_table(&program).unwrap();
        let callee =
            super::super::frontend::type_function_with_signatures(&program, 0, &signatures)
                .unwrap();
        let lowered = nominal_lowered(&callee);
        assert!(std::sync::Arc::ptr_eq(
            &callee.ref_contract,
            &lowered.ref_contract
        ));
        assert!(matches!(
            lowered.ref_contract.returns,
            ir::ReturnContract::SharedInputPaths(_)
        ));
        let source =
            super::super::frontend::type_function_with_signatures(&program, 1, &signatures)
                .unwrap();
        let function = nominal_lowered(&source);
        let call_block = function
            .blocks
            .iter()
            .find(|block| matches!(block.terminator.value, Terminator::Call { .. }))
            .unwrap();
        let call_count = function.blocks.iter().filter(|block| matches!(&block.terminator.value, Terminator::Call { signature, .. } if matches!(signature.result, Ty::Ref { .. }))).count();
        assert_eq!(call_count, 2);
        let call_expr = source
            .exprs
            .iter()
            .find_map(|expr| {
                if let ir::ExprKind::Deref { value } = expr.kind {
                    Some(value)
                } else {
                    None
                }
            })
            .unwrap();
        let actual_carrier = LocalId(source.locals.len() + call_expr.0);
        assert!(function.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value, Statement::Assign { value: Rvalue::Use(Operand::Copy(place)), .. } if place.local == actual_carrier && place.projection == [Projection::Deref])));
        let mut malformed = function.clone();
        for block in &mut malformed.blocks {
            if let Terminator::Call { signature, .. } = &mut block.terminator.value {
                signature.ref_contract = Default::default();
                break;
            }
        }
        assert!(
            malformed
                .validate()
                .unwrap_err()
                .contains("invalid noescape reference contract")
        );
        let mut excess = function.clone();
        excess.blocks = vec![call_block.clone(); ir::MAX_LOAN_SITES];
        assert!(
            excess
                .validate()
                .unwrap_err()
                .contains("combined reference origin limit")
        );
    }

    #[test]
    fn dereferenced_enum_variant_matches_fail_closed_without_refinement() {
        for source in [
            "enum E{A,B} fn f(p:&E)->i32{match *p{E::A=>1,E::B=>2}} fn main(){}",
            "enum E{A} fn f(p:&E)->i32{match *p{E::A=>1}} fn main(){}",
        ] {
            let program = super::super::frontend::parse(source).unwrap();
            let signatures = super::super::frontend::signature_table(&program).unwrap();
            assert!(
                super::super::frontend::type_function_with_signatures(&program, 0, &signatures)
                    .and_then(|source| lower(&source))
                    .unwrap_err()
                    .contains("unsupported")
            );
        }
        let source = checked("enum E{A,B} fn f(p:&E){match *p{_=>()};} fn main(){}", 0);
        let mut function = nominal_lowered(&source);
        let Ty::Ref { referent, .. } = &function.locals[0].ty else {
            unreachable!()
        };
        let Ty::Enum(ty) = referent.as_ref() else {
            unreachable!()
        };
        let mut variant_source = source.clone();
        for expression in &mut variant_source.exprs {
            if let ir::ExprKind::Match { arms, .. } = &mut expression.kind {
                arms.push(ir::MatchArm {
                    pattern: ir::MatchPattern::EnumVariant {
                        ty: ty.clone(),
                        variant: 0,
                        bindings: vec![],
                    },
                    body: arms[0].body,
                });
            }
        }
        assert!(
            lower(&variant_source)
                .unwrap_err()
                .contains("unsupported: enum variant matching through dereference")
        );
        function.blocks[function.entry.0].terminator.value = Terminator::SwitchVariant {
            place: Place {
                local: LocalId(0),
                projection: vec![Projection::Deref],
            },
            variant: ir::VariantId {
                owner: ty.def().id,
                index: 0,
            },
            then_block: function.entry,
            else_block: function.entry,
        };
        assert!(
            function
                .validate()
                .unwrap_err()
                .contains("unsupported: enum variant matching through dereference")
        );
        let analysis = super::super::ownership::analyze(&function);
        assert!(!analysis.passed());
    }

    #[test]
    fn reference_call_signatures_and_argument_snapshots_are_noescape_shaped() {
        for kind in [ir::BorrowKind::Shared, ir::BorrowKind::Mut] {
            let reference = Ty::Ref {
                kind,
                referent: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            };
            let source = with_root(
                Ty::Int(IntTy::I32),
                vec![
                    expr(
                        reference.clone(),
                        ir::ExprKind::Borrow {
                            kind,
                            place: place(LocalId(0)),
                        },
                    ),
                    expr(
                        Ty::Unit,
                        ir::ExprKind::Call {
                            target: test_target(),
                            name: "callee".into(),
                            args: vec![ExprId(0)],
                            signature: ir::Signature {
                                ref_contract: Default::default(),
                                params: vec![reference.clone()],
                                result: Ty::Unit,
                            },
                        },
                    ),
                ],
                vec![ir::Stmt::Expr(ExprId(1))],
            );
            let function = nominal_lowered(&source);
            let call_block = function
                .blocks
                .iter()
                .position(|block| matches!(block.terminator.value, Terminator::Call { .. }))
                .unwrap();
            let Terminator::Call { args, next, .. } = &function.blocks[call_block].terminator.value
            else {
                unreachable!()
            };
            let carrier = match &args[0] {
                Operand::Copy(place) if kind == ir::BorrowKind::Shared => place.local,
                Operand::Move(place) if kind == ir::BorrowKind::Mut => place.local,
                _ => panic!("reference argument must use a stable owning snapshot"),
            };
            assert!(!function.blocks[call_block].statements.iter().any(|statement| matches!(statement.value, Statement::StorageDead(local) if local == carrier)));
            assert!(function.blocks[next.0].statements.iter().any(|statement| matches!(statement.value, Statement::StorageDead(local) if local == carrier)));
            for malformed in [
                Ty::Tuple(std::sync::Arc::from([reference.clone()])),
                Ty::Array {
                    element: std::sync::Arc::new(reference.clone()),
                    len: 0,
                },
                Ty::Ref {
                    kind,
                    referent: std::sync::Arc::new(reference.clone()),
                },
            ] {
                let mut invalid = function.clone();
                let Terminator::Call { signature, .. } =
                    &mut invalid.blocks[call_block].terminator.value
                else {
                    unreachable!()
                };
                signature.params[0] = malformed;
                assert!(
                    invalid
                        .validate()
                        .unwrap_err()
                        .contains("nested or aggregate references")
                );
            }
            let mut invalid = function.clone();
            let Terminator::Call { signature, .. } =
                &mut invalid.blocks[call_block].terminator.value
            else {
                unreachable!()
            };
            signature.result = reference;
            assert!(
                invalid
                    .validate()
                    .unwrap_err()
                    .contains("invalid noescape reference contract")
            );
            let mut invalid = function.clone();
            let Terminator::Call { signature, .. } =
                &mut invalid.blocks[call_block].terminator.value
            else {
                unreachable!()
            };
            signature.params[0] = Ty::Int(IntTy::I32);
            assert!(
                invalid
                    .validate()
                    .unwrap_err()
                    .contains("operand type mismatch")
            );
        }
    }

    #[test]
    fn reference_formals_share_the_explicit_loan_origin_cap() {
        let reference = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: std::sync::Arc::new(Ty::Int(IntTy::I32)),
        };
        let source = with_root(reference.clone(), vec![], vec![]);
        let mut function = nominal_lowered(&source);
        for index in 1..ir::MAX_LOAN_SITES {
            let local = LocalId(function.locals.len());
            function.locals.push(ir::LocalDecl {
                name: format!("formal{index}"),
                ty: reference.clone(),
                mutable: false,
            });
            function.params.push(local);
        }
        function.validate().unwrap();
        let mut mixed = function.clone();
        mixed.loan_sites.push(LoanSite {
            kind: ir::BorrowKind::Shared,
            place: Place {
                local: LocalId(0),
                projection: vec![Projection::Deref],
            },
            origin: None,
        });
        assert!(
            mixed
                .validate()
                .unwrap_err()
                .contains("combined reference origin limit")
        );
        let local = LocalId(function.locals.len());
        function.locals.push(ir::LocalDecl {
            name: "overflow".into(),
            ty: reference,
            mutable: false,
        });
        function.params.push(local);
        assert!(
            function
                .validate()
                .unwrap_err()
                .contains("combined reference origin limit")
        );
        let mut source = source;
        source.locals = function.locals;
        source.params = function.params;
        assert!(
            lower(&source)
                .unwrap_err()
                .contains("combined reference origin limit")
        );
    }

    #[test]
    fn borrowed_place_site_preserves_origin_and_unique_instruction() {
        let reference = Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: std::sync::Arc::new(Ty::Int(IntTy::I32)),
        };
        let mut source = with_root(
            Ty::Int(IntTy::I32),
            vec![
                expr(
                    reference.clone(),
                    ir::ExprKind::Borrow {
                        kind: ir::BorrowKind::Mut,
                        place: place(LocalId(0)),
                    },
                ),
                expr(reference.clone(), ir::ExprKind::Local(LocalId(1))),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Deref { value: ExprId(1) },
                ),
            ],
            vec![
                ir::Stmt::Let {
                    local: LocalId(1),
                    value: ExprId(0),
                },
                ir::Stmt::Expr(ExprId(2)),
            ],
        );
        source.locals.push(ir::LocalDecl {
            name: "carrier".into(),
            ty: reference,
            mutable: false,
        });
        let function = nominal_lowered(&source);
        assert_eq!(function.loan_sites.len(), 1);
        assert_eq!(function.loan_sites[0].origin, Some(ExprId(0)));
        assert_eq!(function.loan_sites[0].place, place(LocalId(0)));
        assert!(function.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value,
            Statement::Assign { value: Rvalue::Use(Operand::Copy(place)), .. } if place.local == LocalId(1) && place.projection == [Projection::Deref]
        )));
        assert!(!function.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value,
            Statement::Assign { value: Rvalue::Use(Operand::Move(place)), .. } if place.local == LocalId(1)
        )));
        let mut repeated = source.clone();
        repeated.body.statements.push(ir::Stmt::Expr(ExprId(0)));
        assert!(lower(&repeated).unwrap_err().contains("repeated borrow"));
        let mut invalid = function.clone();
        invalid.loan_sites.push(invalid.loan_sites[0].clone());
        assert!(invalid.validate().unwrap_err().contains("exactly one"));
        invalid.loan_sites = vec![function.loan_sites[0].clone(); ir::MAX_LOAN_SITES + 1];
        assert!(invalid.validate().unwrap_err().contains("loan site limit"));
        assert!(
            validate_type(
                &Ty::Tuple(std::sync::Arc::from([function.locals[1].ty.clone()])),
                0
            )
            .is_err()
        );
        let mut signature_escape = function.clone();
        signature_escape.params.push(LocalId(1));
        signature_escape.validate().unwrap();
        signature_escape.locals[1].ty = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: std::sync::Arc::new(function.locals[1].ty.clone()),
        };
        assert!(
            signature_escape
                .validate()
                .unwrap_err()
                .contains("nested or aggregate references")
        );
        assert!(
            function
                .rvalue_ty(&Rvalue::Binary {
                    op: BinaryOp::Eq,
                    left: Operand::Move(place(LocalId(1))),
                    right: Operand::Move(place(LocalId(1)))
                })
                .is_err()
        );
        let mut ignored = with_root(
            function.locals[1].ty.clone(),
            vec![
                expr(
                    function.locals[1].ty.clone(),
                    ir::ExprKind::Local(LocalId(0)),
                ),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Deref { value: ExprId(0) },
                ),
            ],
            vec![
                ir::Stmt::Declare { local: LocalId(0) },
                ir::Stmt::LetPattern {
                    value: ExprId(1),
                    ty: Ty::Int(IntTy::I32),
                    bindings: vec![],
                },
            ],
        );
        ignored.params.clear();
        let ignored = nominal_lowered(&ignored);
        assert!(root_reads(&ignored).is_empty());
        assert!(
            !ignored
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .any(|statement| matches!(statement.value, Statement::Inspect(_)))
        );
    }

    fn nominal(fields: Vec<Ty>) -> ir::StructTy {
        ir::StructTy::new(ir::StructDef {
            source_decl: test_decl(),
            type_args: std::sync::Arc::from([]),
            id: ir::NominalAppId {
                snapshot: ir::SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "S".into(),
            module: String::new(),
            public: false,
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, ty)| ir::FieldDef {
                    name: format!("f{index}"),
                    ty,
                    public: false,
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    fn expr(ty: Ty, kind: ir::ExprKind) -> ir::TypedExpr {
        ir::TypedExpr {
            ty,
            completion: ir::Completion::Returns,
            kind,
        }
    }

    fn with_root(
        root: Ty,
        exprs: Vec<ir::TypedExpr>,
        statements: Vec<ir::Stmt>,
    ) -> ir::CheckedFunction {
        ir::CheckedFunction {
            ref_contract: Default::default(),
            name: "nominal_fixture".into(),
            params: vec![LocalId(0)],
            result: Ty::Unit,
            locals: vec![ir::LocalDecl {
                name: "root".into(),
                ty: root,
                mutable: true,
            }],
            exprs,
            body: ir::Block {
                statements,
                tail: None,
            },
        }
    }

    fn nominal_lowered(source: &ir::CheckedFunction) -> Function {
        source.validate().unwrap();
        let function = lower(source).unwrap();
        function.validate().unwrap();
        function
    }

    fn root_reads(function: &Function) -> Vec<&Operand> {
        function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.value {
                Statement::Assign {
                    value: Rvalue::Use(operand),
                    ..
                } => match operand {
                    Operand::Copy(p) | Operand::Move(p) if p.local == LocalId(0) => Some(operand),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn compounds_validate_dead_ops_and_lower_rhs_then_single_index_update() {
        let base = nominal_lowered(&with_root(
            Ty::Array {
                element: std::sync::Arc::new(Ty::Int(IntTy::I32)),
                len: 2,
            },
            vec![],
            vec![],
        ));
        let update = |op, index, value| Statement::IndexAssignOp {
            array: place(LocalId(0)),
            index,
            value,
            op,
        };
        let index = Operand::Constant(Constant::Int(IntValue::usize(0)));
        let value = Operand::Constant(Constant::Int(IntValue::i32(1)));
        for op in [
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Rem,
            BinaryOp::BitAnd,
            BinaryOp::BitOr,
            BinaryOp::BitXor,
            BinaryOp::Shl,
            BinaryOp::Shr,
        ] {
            let mut function = base.clone();
            function.blocks.push(BasicBlock {
                statements: vec![Located {
                    value: update(op, index.clone(), value.clone()),
                    origin: None,
                }],
                terminator: Located {
                    value: Terminator::Unreachable,
                    origin: None,
                },
            });
            function.validate().unwrap();
        }
        for (op, idx, rhs) in [
            (BinaryOp::Eq, index.clone(), value.clone()),
            (BinaryOp::And, index.clone(), value.clone()),
            (
                BinaryOp::Add,
                Operand::Constant(Constant::Int(IntValue::i32(0))),
                value.clone(),
            ),
            (
                BinaryOp::Add,
                index.clone(),
                Operand::Constant(Constant::Bool(true)),
            ),
            (
                BinaryOp::Add,
                index.clone(),
                Operand::Constant(Constant::Int(IntValue {
                    ty: IntTy::U32,
                    bits: 1,
                })),
            ),
            (
                BinaryOp::Shl,
                index.clone(),
                Operand::Constant(Constant::Int(IntValue {
                    ty: IntTy::U64,
                    bits: u64::MAX,
                })),
            ),
            (
                BinaryOp::Shr,
                index.clone(),
                Operand::Constant(Constant::Int(IntValue {
                    ty: IntTy::U8,
                    bits: 256,
                })),
            ),
        ] {
            let mut function = base.clone();
            function.blocks.push(BasicBlock {
                statements: vec![Located {
                    value: update(op, idx, rhs),
                    origin: None,
                }],
                terminator: Located {
                    value: Terminator::Unreachable,
                    origin: None,
                },
            });
            assert!(function.validate().is_err());
        }
        let source = with_root(
            Ty::Array {
                element: std::sync::Arc::new(Ty::Int(IntTy::I32)),
                len: 2,
            },
            vec![
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Call {
                        target: test_target(),
                        name: "rhs".into(),
                        args: vec![],
                        signature: ir::Signature {
                            ref_contract: Default::default(),
                            params: vec![],
                            result: Ty::Int(IntTy::I32),
                        },
                    },
                ),
                expr(
                    Ty::Int(IntTy::Usize),
                    ir::ExprKind::Call {
                        target: test_target(),
                        name: "index".into(),
                        args: vec![],
                        signature: ir::Signature {
                            ref_contract: Default::default(),
                            params: vec![],
                            result: Ty::Int(IntTy::Usize),
                        },
                    },
                ),
            ],
            vec![ir::Stmt::AssignIndexOp {
                array: place(LocalId(0)),
                index: ExprId(1),
                op: BinaryOp::Add,
                value: ExprId(0),
            }],
        );
        let lowered = nominal_lowered(&source);
        let calls: Vec<_> = lowered
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator.value {
                Terminator::Call { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, ["rhs", "index"]);
        let statements: Vec<_> = lowered
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .collect();
        assert_eq!(
            statements
                .iter()
                .filter(|statement| matches!(statement.value, Statement::IndexAssignOp { .. }))
                .count(),
            1
        );
        assert!(!statements.iter().any(|statement| matches!(
            statement.value,
            Statement::IndexAssign { .. }
                | Statement::Assign {
                    value: Rvalue::IndexCopy { .. },
                    ..
                }
        )));
        assert_storage_paths(&lowered);
        assert!(super::super::ownership::analyze(&lowered).passed());
        for (source, passes) in [
            ("fn f(){let mut x:i32;x+=1;}fn main(){}", false),
            ("fn f(){let x=1;x+=1;}fn main(){}", false),
            ("fn f(){let x=1;let p=&x;*p+=1;}fn main(){}", false),
            ("fn f(){let mut x:i32;x+={x=10;1};}fn main(){}", true),
            (
                "fn f(){let mut x=1;let mut y=2;let mut p=&mut x;*p+={p=&mut y;3};}fn main(){}",
                true,
            ),
        ] {
            let function = nominal_lowered(&checked(source, 0));
            assert_eq!(
                super::super::ownership::analyze(&function).passed(),
                passes,
                "{source}"
            );
        }
    }

    #[test]
    fn shifts_validate_all_pairs_full_counts_dead_blocks_and_eager_order() {
        let types = [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::Usize,
        ];
        let function = nominal_lowered(&with_root(Ty::Unit, vec![], vec![]));
        for op in [BinaryOp::Shl, BinaryOp::Shr] {
            for left in types {
                for right in types {
                    let lhs = Operand::Constant(Constant::Int(IntValue {
                        ty: left,
                        bits: left.mask(),
                    }));
                    let count =
                        |bits| Operand::Constant(Constant::Int(IntValue { ty: right, bits }));
                    for bits in [0, u64::from(left.bits() - 1)] {
                        assert_eq!(
                            function
                                .rvalue_ty(&Rvalue::Binary {
                                    op,
                                    left: lhs.clone(),
                                    right: count(bits)
                                })
                                .unwrap(),
                            Ty::Int(left)
                        );
                    }
                    for bits in [u64::from(left.bits()), right.mask()] {
                        assert!(
                            function
                                .rvalue_ty(&Rvalue::Binary {
                                    op,
                                    left: lhs.clone(),
                                    right: count(bits)
                                })
                                .unwrap_err()
                                .contains("shift count")
                        );
                    }
                    if right.bits() == 64 {
                        assert!(
                            function
                                .rvalue_ty(&Rvalue::Binary {
                                    op,
                                    left: lhs.clone(),
                                    right: count(1_u64 << 32)
                                })
                                .is_err()
                        );
                    }
                }
            }
            for endpoint in [
                Operand::Constant(Constant::Bool(false)),
                Operand::Constant(Constant::Unit),
            ] {
                let integer = Operand::Constant(Constant::Int(IntValue::i32(0)));
                assert!(
                    function
                        .rvalue_ty(&Rvalue::Binary {
                            op,
                            left: endpoint.clone(),
                            right: integer.clone()
                        })
                        .is_err()
                );
                assert!(
                    function
                        .rvalue_ty(&Rvalue::Binary {
                            op,
                            left: integer,
                            right: endpoint
                        })
                        .is_err()
                );
            }
            for count in [
                IntValue {
                    ty: IntTy::U64,
                    bits: 64,
                },
                IntValue {
                    ty: IntTy::U64,
                    bits: u64::MAX,
                },
                IntValue {
                    ty: IntTy::U8,
                    bits: 256,
                },
            ] {
                let mut dead = function.clone();
                dead.blocks.push(BasicBlock {
                    statements: vec![Located {
                        value: Statement::Assign {
                            destination: place(LocalId(0)),
                            value: Rvalue::Binary {
                                op,
                                left: Operand::Constant(Constant::Int(IntValue::i32(1))),
                                right: Operand::Constant(Constant::Int(count)),
                            },
                        },
                        origin: None,
                    }],
                    terminator: Located {
                        value: Terminator::Unreachable,
                        origin: None,
                    },
                });
                let error = dead.validate().unwrap_err();
                assert!(error.contains(if count.bits > count.ty.mask() {
                    "noncanonical"
                } else {
                    "shift count"
                }));
            }
            let source = with_root(
                Ty::Unit,
                vec![
                    expr(
                        Ty::Int(IntTy::I8),
                        ir::ExprKind::Call {
                            target: test_target(),
                            name: "left".into(),
                            args: vec![],
                            signature: ir::Signature {
                                ref_contract: Default::default(),
                                params: vec![],
                                result: Ty::Int(IntTy::I8),
                            },
                        },
                    ),
                    expr(
                        Ty::Int(IntTy::U64),
                        ir::ExprKind::Call {
                            target: test_target(),
                            name: "right".into(),
                            args: vec![],
                            signature: ir::Signature {
                                ref_contract: Default::default(),
                                params: vec![],
                                result: Ty::Int(IntTy::U64),
                            },
                        },
                    ),
                    expr(
                        Ty::Int(IntTy::I8),
                        ir::ExprKind::Binary {
                            op,
                            left: ExprId(0),
                            right: ExprId(1),
                        },
                    ),
                ],
                vec![ir::Stmt::Expr(ExprId(2))],
            );
            let lowered = nominal_lowered(&source);
            let calls: Vec<_> = lowered
                .blocks
                .iter()
                .filter_map(|block| match &block.terminator.value {
                    Terminator::Call { name, .. } => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(calls, ["left", "right"]);
            assert_eq!(
                lowered
                    .blocks
                    .iter()
                    .flat_map(|block| &block.statements)
                    .filter(|statement| matches!(
                        statement.value,
                        Statement::Assign {
                            value: Rvalue::Binary {
                                op: BinaryOp::Shl | BinaryOp::Shr,
                                ..
                            },
                            ..
                        }
                    ))
                    .count(),
                1
            );
            assert_storage_paths(&lowered);
            assert!(super::super::ownership::analyze(&lowered).passed());
        }
        // Other operators retain exact identity despite shift heterogeneity.
        assert!(
            function
                .rvalue_ty(&Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: Operand::Constant(Constant::Int(IntValue::i32(1))),
                    right: Operand::Constant(Constant::Int(IntValue {
                        ty: IntTy::U32,
                        bits: 1
                    }))
                })
                .is_err()
        );
    }

    #[test]
    fn bitwise_exact_scalars_dead_constants_and_eager_call_order() {
        let function = nominal_lowered(&with_root(Ty::Unit, vec![], vec![]));
        for op in [BinaryOp::BitAnd, BinaryOp::BitOr, BinaryOp::BitXor] {
            for ty in [
                IntTy::I8,
                IntTy::I16,
                IntTy::I32,
                IntTy::I64,
                IntTy::Isize,
                IntTy::U8,
                IntTy::U16,
                IntTy::U32,
                IntTy::U64,
                IntTy::Usize,
            ] {
                let operand = Operand::Constant(Constant::Int(IntValue {
                    ty,
                    bits: ty.mask(),
                }));
                assert_eq!(
                    function
                        .rvalue_ty(&Rvalue::Binary {
                            op,
                            left: operand.clone(),
                            right: operand.clone()
                        })
                        .unwrap(),
                    Ty::Int(ty)
                );
                assert!(
                    function
                        .rvalue_ty(&Rvalue::Binary {
                            op,
                            left: operand,
                            right: Operand::Constant(Constant::Bool(true))
                        })
                        .is_err()
                );
            }
            assert_eq!(
                function
                    .rvalue_ty(&Rvalue::Binary {
                        op,
                        left: Operand::Constant(Constant::Bool(false)),
                        right: Operand::Constant(Constant::Bool(true))
                    })
                    .unwrap(),
                Ty::Bool
            );
            assert!(
                function
                    .rvalue_ty(&Rvalue::Binary {
                        op,
                        left: Operand::Constant(Constant::Unit),
                        right: Operand::Constant(Constant::Unit)
                    })
                    .is_err()
            );
            assert!(
                function
                    .rvalue_ty(&Rvalue::Binary {
                        op,
                        left: Operand::Constant(Constant::Int(IntValue::i32(1))),
                        right: Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::U32,
                            bits: 1
                        }))
                    })
                    .is_err()
            );
            let mut dead = function.clone();
            dead.blocks.push(BasicBlock {
                statements: vec![Located {
                    value: Statement::Assign {
                        destination: place(LocalId(0)),
                        value: Rvalue::Binary {
                            op,
                            left: Operand::Constant(Constant::Int(IntValue {
                                ty: IntTy::U8,
                                bits: 256,
                            })),
                            right: Operand::Constant(Constant::Int(IntValue {
                                ty: IntTy::U8,
                                bits: 0,
                            })),
                        },
                    },
                    origin: None,
                }],
                terminator: Located {
                    value: Terminator::Unreachable,
                    origin: None,
                },
            });
            assert!(dead.validate().unwrap_err().contains("noncanonical"));
            let source = with_root(
                Ty::Unit,
                vec![
                    expr(
                        Ty::Bool,
                        ir::ExprKind::Call {
                            target: test_target(),
                            name: "left".into(),
                            args: vec![],
                            signature: ir::Signature {
                                ref_contract: Default::default(),
                                params: vec![],
                                result: Ty::Bool,
                            },
                        },
                    ),
                    expr(
                        Ty::Bool,
                        ir::ExprKind::Call {
                            target: test_target(),
                            name: "right".into(),
                            args: vec![],
                            signature: ir::Signature {
                                ref_contract: Default::default(),
                                params: vec![],
                                result: Ty::Bool,
                            },
                        },
                    ),
                    expr(
                        Ty::Bool,
                        ir::ExprKind::Binary {
                            op,
                            left: ExprId(0),
                            right: ExprId(1),
                        },
                    ),
                ],
                vec![ir::Stmt::Expr(ExprId(2))],
            );
            let lowered = nominal_lowered(&source);
            let calls: Vec<_> = lowered
                .blocks
                .iter()
                .filter_map(|block| match &block.terminator.value {
                    Terminator::Call { name, .. } => Some(name.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(calls, ["left", "right"]);
            assert!(
                !lowered
                    .blocks
                    .iter()
                    .any(|block| matches!(block.terminator.value, Terminator::Switch { .. }))
            );
            assert_eq!(lowered.blocks.iter().flat_map(|block| &block.statements).filter(|statement| matches!(statement.value, Statement::Assign { value: Rvalue::Binary { op: actual, .. }, .. } if std::mem::discriminant(&actual) == std::mem::discriminant(&op))).count(), 1);
            assert_storage_paths(&lowered);
            assert!(super::super::ownership::analyze(&lowered).passed());
        }
        for op in [BinaryOp::And, BinaryOp::Or] {
            assert!(
                function
                    .rvalue_ty(&Rvalue::Binary {
                        op,
                        left: Operand::Constant(Constant::Bool(false)),
                        right: Operand::Constant(Constant::Bool(true))
                    })
                    .is_err()
            );
        }
    }

    #[test]
    fn casts_validate_all_integer_pairs_dead_operands_and_evaluate_source_once() {
        let types = [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::Usize,
        ];
        let function = nominal_lowered(&with_root(Ty::Int(IntTy::I32), vec![], vec![]));
        for from in types {
            for to in types {
                let value = Operand::Constant(Constant::Int(IntValue {
                    ty: from,
                    bits: from.mask(),
                }));
                assert_eq!(
                    function.rvalue_ty(&Rvalue::IntCast { value, to }).unwrap(),
                    Ty::Int(to)
                );
            }
        }
        for root in [
            Ty::Bool,
            Ty::Unit,
            Ty::Never,
            Ty::Tuple(std::sync::Arc::from([Ty::Int(IntTy::I32)])),
            Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            },
        ] {
            let function = nominal_lowered(&with_root(root, vec![], vec![]));
            assert!(
                function
                    .rvalue_ty(&Rvalue::IntCast {
                        value: Operand::Copy(place(LocalId(0))),
                        to: IntTy::U8
                    })
                    .is_err()
            );
        }
        let mut invalid = function.clone();
        invalid.blocks.push(BasicBlock {
            statements: vec![Located {
                value: Statement::Assign {
                    destination: place(LocalId(0)),
                    value: Rvalue::IntCast {
                        value: Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::U8,
                            bits: 256,
                        })),
                        to: IntTy::I32,
                    },
                },
                origin: None,
            }],
            terminator: Located {
                value: Terminator::Unreachable,
                origin: None,
            },
        });
        assert!(invalid.validate().unwrap_err().contains("noncanonical"));
        let source = with_root(
            Ty::Unit,
            vec![
                expr(
                    Ty::Int(IntTy::I64),
                    ir::ExprKind::Call {
                        target: test_target(),
                        name: "once".into(),
                        args: vec![],
                        signature: ir::Signature {
                            ref_contract: Default::default(),
                            params: vec![],
                            result: Ty::Int(IntTy::I64),
                        },
                    },
                ),
                expr(
                    Ty::Int(IntTy::U8),
                    ir::ExprKind::IntCast {
                        value: ExprId(0),
                        to: IntTy::U8,
                    },
                ),
            ],
            vec![ir::Stmt::Expr(ExprId(1))],
        );
        let lowered = nominal_lowered(&source);
        assert_eq!(lowered.blocks.iter().filter(|block| matches!(&block.terminator.value, Terminator::Call {name, ..} if name == "once")).count(), 1);
        let casts: Vec<_> = lowered
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|statement| {
                matches!(
                    &statement.value,
                    Statement::Assign {
                        value: Rvalue::IntCast { to: IntTy::U8, .. },
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(casts.len(), 1);
        assert_eq!(casts[0].origin, Some(ExprId(1)));
        assert_storage_paths(&lowered);
        assert!(super::super::ownership::analyze(&lowered).passed());
        let mut repeated = source.clone();
        repeated.body.statements.push(ir::Stmt::Expr(ExprId(1)));
        let repeated = nominal_lowered(&repeated);
        assert_eq!(repeated.blocks.iter().filter(|block| matches!(&block.terminator.value, Terminator::Call {name, ..} if name == "once")).count(), 2);
        let mut cyclic = source.clone();
        cyclic.exprs[0].kind = ir::ExprKind::IntCast {
            value: ExprId(0),
            to: IntTy::I64,
        };
        cyclic.validate().unwrap();
        assert!(
            lower(&cyclic)
                .unwrap_err()
                .contains("lowering depth resource")
        );
        cyclic.exprs[0].kind = ir::ExprKind::Unary {
            op: UnaryOp::Not,
            value: ExprId(1),
        };
        cyclic.exprs[1].ty = Ty::Int(IntTy::I64);
        cyclic.exprs[1].kind = ir::ExprKind::IntCast {
            value: ExprId(0),
            to: IntTy::I64,
        };
        cyclic.validate().unwrap();
        assert!(
            lower(&cyclic)
                .unwrap_err()
                .contains("lowering depth resource")
        );
        let mut budget = Lowerer {
            budget: None,
            resource_error: None,
            lowering_depth: 0,
            lowering_work: ir::MAX_PATTERN_PROJECTION_WORK,
            origins: 0,
            loans: vec![],
            used_loans: Default::default(),
            source: &source,
            result: function,
            expressions: vec![],
            parameters: Default::default(),
            blocks: vec![],
            current: BlockId(0),
            scopes: vec![],
            loops: vec![],
        };
        assert!(
            budget
                .materialize_expression(ExprId(usize::MAX))
                .unwrap_err()
                .contains("lowering work resource")
        );
        assert_eq!(budget.lowering_depth, 0);
        assert!(budget.blocks.is_empty());
        let mut chain = vec![expr(
            Ty::Int(IntTy::I32),
            ir::ExprKind::Int(IntValue::i32(1)),
        )];
        for index in 0..120 {
            chain.push(expr(
                Ty::Int(IntTy::U64),
                ir::ExprKind::IntCast {
                    value: ExprId(index),
                    to: IntTy::U64,
                },
            ));
        }
        let chain = with_root(Ty::Unit, chain, vec![ir::Stmt::Expr(ExprId(120))]);
        let lowered = nominal_lowered(&chain);
        assert_eq!(
            lowered
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .filter(|statement| matches!(
                    statement.value,
                    Statement::Assign {
                        value: Rvalue::IntCast { .. },
                        ..
                    }
                ))
                .count(),
            120
        );
        assert!(super::super::ownership::analyze(&lowered).passed());
        super::super::backend::emit_checked_function(&chain).unwrap();
    }

    #[test]
    fn all_integer_cfg_operators_require_exact_types_and_validate_dead_constants() {
        for ty in [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::Usize,
        ] {
            let mut function = nominal_lowered(&with_root(Ty::Int(ty), vec![], vec![]));
            let operand = Operand::Constant(Constant::Int(IntValue {
                ty,
                bits: ty.mask(),
            }));
            assert_eq!(function.operand_ty(&operand).unwrap(), Ty::Int(ty));
            assert_eq!(
                function
                    .rvalue_ty(&Rvalue::Unary {
                        op: UnaryOp::Not,
                        value: operand.clone()
                    })
                    .unwrap(),
                Ty::Int(ty)
            );
            assert_eq!(
                function
                    .rvalue_ty(&Rvalue::Unary {
                        op: UnaryOp::Neg,
                        value: operand.clone()
                    })
                    .is_ok(),
                ty.is_signed()
            );
            for op in [
                BinaryOp::Add,
                BinaryOp::Sub,
                BinaryOp::Mul,
                BinaryOp::Div,
                BinaryOp::Rem,
                BinaryOp::Eq,
                BinaryOp::Ne,
                BinaryOp::Lt,
                BinaryOp::Le,
                BinaryOp::Gt,
                BinaryOp::Ge,
            ] {
                let result = function
                    .rvalue_ty(&Rvalue::Binary {
                        op,
                        left: operand.clone(),
                        right: operand.clone(),
                    })
                    .unwrap();
                assert_eq!(
                    result,
                    if matches!(
                        op,
                        BinaryOp::Add
                            | BinaryOp::Sub
                            | BinaryOp::Mul
                            | BinaryOp::Div
                            | BinaryOp::Rem
                    ) {
                        Ty::Int(ty)
                    } else {
                        Ty::Bool
                    }
                );
                let other = if ty == IntTy::I8 {
                    IntTy::U8
                } else {
                    IntTy::I8
                };
                assert!(
                    function
                        .rvalue_ty(&Rvalue::Binary {
                            op,
                            left: operand.clone(),
                            right: Operand::Constant(Constant::Int(IntValue {
                                ty: other,
                                bits: 0
                            }))
                        })
                        .is_err()
                );
            }
            if ty.bits() < 64 {
                function.blocks.push(BasicBlock {
                    statements: vec![Located {
                        value: Statement::Assign {
                            destination: place(LocalId(0)),
                            value: Rvalue::Use(Operand::Constant(Constant::Int(IntValue {
                                ty,
                                bits: 1u64 << ty.bits(),
                            }))),
                        },
                        origin: None,
                    }],
                    terminator: Located {
                        value: Terminator::Unreachable,
                        origin: None,
                    },
                });
                assert!(function.validate().unwrap_err().contains("noncanonical"));
            }
        }
    }

    #[test]
    fn usize_cfg_operands_and_unsigned_operators_do_not_coerce_i32() {
        let function = nominal_lowered(&with_root(Ty::Int(IntTy::Usize), vec![], vec![]));
        let max = Operand::Constant(Constant::Int(IntValue::usize(u64::MAX)));
        let one = Operand::Constant(Constant::Int(IntValue::usize(1)));
        assert_eq!(function.operand_ty(&max).unwrap(), Ty::Int(IntTy::Usize));
        assert_eq!(
            function
                .rvalue_ty(&Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: max.clone(),
                    right: one.clone()
                })
                .unwrap(),
            Ty::Int(IntTy::Usize)
        );
        assert_eq!(
            function
                .rvalue_ty(&Rvalue::Binary {
                    op: BinaryOp::Lt,
                    left: max.clone(),
                    right: one.clone()
                })
                .unwrap(),
            Ty::Bool
        );
        assert_eq!(
            function
                .rvalue_ty(&Rvalue::Unary {
                    op: UnaryOp::Not,
                    value: max.clone()
                })
                .unwrap(),
            Ty::Int(IntTy::Usize)
        );
        assert!(
            function
                .rvalue_ty(&Rvalue::Unary {
                    op: UnaryOp::Neg,
                    value: max.clone()
                })
                .is_err()
        );
        for op in [BinaryOp::Add, BinaryOp::Lt, BinaryOp::Eq] {
            assert!(
                function
                    .rvalue_ty(&Rvalue::Binary {
                        op,
                        left: max.clone(),
                        right: Operand::Constant(Constant::Int(IntValue::i32(1)))
                    })
                    .is_err()
            );
        }
        let root = Ty::Array {
            element: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            len: 1,
        };
        let function = nominal_lowered(&with_root(root, vec![], vec![]));
        assert_eq!(
            function
                .rvalue_ty(&Rvalue::IndexCopy {
                    array: place(LocalId(0)),
                    index: max
                })
                .unwrap(),
            Ty::Int(IntTy::I32)
        );
        assert!(
            function
                .rvalue_ty(&Rvalue::IndexCopy {
                    array: place(LocalId(0)),
                    index: Operand::Constant(Constant::Int(IntValue::i32(0)))
                })
                .is_err()
        );
    }

    #[test]
    fn dynamic_index_may_initialize_original_array_before_final_read() {
        let array = Ty::Array {
            element: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            len: 1,
        };
        let mut source = with_root(
            array.clone(),
            vec![
                expr(Ty::Int(IntTy::I32), ir::ExprKind::Int(IntValue::i32(7))),
                expr(array.clone(), ir::ExprKind::Array(vec![ExprId(0)])),
                expr(Ty::Int(IntTy::Usize), ir::ExprKind::Int(IntValue::usize(0))),
                expr(
                    Ty::Int(IntTy::Usize),
                    ir::ExprKind::Block(ir::Block {
                        statements: vec![ir::Stmt::Assign {
                            local: LocalId(0),
                            value: ExprId(1),
                        }],
                        tail: Some(ExprId(2)),
                    }),
                ),
                expr(array, ir::ExprKind::Local(LocalId(0))),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::DynamicIndex {
                        base: ExprId(4),
                        index: ExprId(3),
                    },
                ),
            ],
            vec![
                ir::Stmt::Declare { local: LocalId(0) },
                ir::Stmt::Expr(ExprId(5)),
            ],
        );
        source.params.clear();
        let function = nominal_lowered(&source);
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
        assert!(
            !function
                .blocks
                .iter()
                .flat_map(|b| &b.statements)
                .any(|s| matches!(s.value, Statement::Inspect(_)))
        );
        let dynamic = function
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .find_map(|s| match &s.value {
                Statement::Assign {
                    value: Rvalue::IndexCopy { array, .. },
                    ..
                } => Some(array),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            dynamic,
            &place(LocalId(0)),
            "read must retain original place, not a copied base snapshot"
        );
    }

    #[test]
    fn dynamic_index_static_prefix_read_precedes_current_index_effects() {
        let child = Ty::Array {
            element: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            len: 1,
        };
        let array = Ty::Array {
            element: std::sync::Arc::new(child.clone()),
            len: 1,
        };
        let mut source = with_root(
            array.clone(),
            vec![
                expr(Ty::Int(IntTy::I32), ir::ExprKind::Int(IntValue::i32(7))),
                expr(child.clone(), ir::ExprKind::Array(vec![ExprId(0)])),
                expr(array.clone(), ir::ExprKind::Array(vec![ExprId(1)])),
                expr(Ty::Int(IntTy::Usize), ir::ExprKind::Int(IntValue::usize(0))),
                expr(
                    Ty::Int(IntTy::Usize),
                    ir::ExprKind::Block(ir::Block {
                        statements: vec![ir::Stmt::Assign {
                            local: LocalId(0),
                            value: ExprId(2),
                        }],
                        tail: Some(ExprId(3)),
                    }),
                ),
                expr(array, ir::ExprKind::Local(LocalId(0))),
                expr(
                    child,
                    ir::ExprKind::Index {
                        base: ExprId(5),
                        index: 0,
                    },
                ),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::DynamicIndex {
                        base: ExprId(6),
                        index: ExprId(4),
                    },
                ),
            ],
            vec![
                ir::Stmt::Declare { local: LocalId(0) },
                ir::Stmt::Expr(ExprId(7)),
            ],
        );
        source.params.clear();
        let function = nominal_lowered(&source);
        assert!(!super::super::ownership::analyze(&function).passed());
        let statements: Vec<_> = function.blocks.iter().flat_map(|b| &b.statements).collect();
        let inspect = statements
            .iter()
            .position(|s| matches!(s.value, Statement::Inspect(_)))
            .unwrap();
        let initialize = statements.iter().position(|s| matches!(&s.value, Statement::Assign { destination, .. } if destination.local == LocalId(0))).unwrap();
        assert!(inspect < initialize);
    }

    #[test]
    fn ignored_noncopy_dynamic_index_has_no_fabricated_element_snapshot() {
        let item = Ty::Struct(nominal(vec![Ty::Int(IntTy::I32)]));
        let array = Ty::Array {
            element: std::sync::Arc::new(item.clone()),
            len: 1,
        };
        let mut source = with_root(
            array.clone(),
            vec![
                expr(array, ir::ExprKind::Local(LocalId(0))),
                expr(Ty::Int(IntTy::Usize), ir::ExprKind::Int(IntValue::usize(0))),
                expr(
                    item.clone(),
                    ir::ExprKind::DynamicIndex {
                        base: ExprId(0),
                        index: ExprId(1),
                    },
                ),
            ],
            vec![ir::Stmt::LetPattern {
                value: ExprId(2),
                ty: item.clone(),
                bindings: vec![],
            }],
        );
        let function = nominal_lowered(&source);
        let element_local = LocalId(source.locals.len() + 2);
        assert!(
            !function.blocks.iter().flat_map(|b| &b.statements).any(
                |s| matches!(s.value, Statement::StorageLive(local) if local == element_local)
            )
        );
        assert_eq!(
            function
                .blocks
                .iter()
                .flat_map(|b| &b.statements)
                .filter(|s| matches!(s.value, Statement::IndexInspect { .. }))
                .count(),
            1
        );
        assert!(
            !function
                .blocks
                .iter()
                .flat_map(|b| &b.statements)
                .any(|s| matches!(
                    s.value,
                    Statement::Assign {
                        value: Rvalue::IndexCopy { .. },
                        ..
                    }
                ))
        );
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
        source.locals.push(ir::LocalDecl {
            name: "extracted".into(),
            ty: item,
            mutable: false,
        });
        source.body.statements = vec![ir::Stmt::Let {
            local: LocalId(1),
            value: ExprId(2),
        }];
        assert!(lower(&source).unwrap_err().contains("unsupported"));
    }

    #[test]
    fn dynamic_index_assignment_owns_rhs_before_index_call() {
        let item = Ty::Struct(nominal(vec![Ty::Int(IntTy::I32)]));
        let array = Ty::Array {
            element: std::sync::Arc::new(item.clone()),
            len: 1,
        };
        let mut source = with_root(
            array,
            vec![
                expr(item.clone(), ir::ExprKind::Local(LocalId(1))),
                expr(
                    Ty::Int(IntTy::Usize),
                    ir::ExprKind::Call {
                        target: test_target(),
                        name: "index".into(),
                        args: vec![],
                        signature: ir::Signature {
                            ref_contract: Default::default(),
                            params: vec![],
                            result: Ty::Int(IntTy::Usize),
                        },
                    },
                ),
            ],
            vec![ir::Stmt::AssignIndex {
                array: place(LocalId(0)),
                index: ExprId(1),
                value: ExprId(0),
            }],
        );
        source.locals.push(ir::LocalDecl {
            name: "rhs".into(),
            ty: item,
            mutable: false,
        });
        source.params.push(LocalId(1));
        let function = nominal_lowered(&source);
        let head = &function.blocks[function.entry.0];
        assert!(head.statements.iter().any(|s| matches!(&s.value, Statement::Assign { value: Rvalue::Use(Operand::Move(place)), .. } if place.local == LocalId(1))));
        assert!(matches!(&head.terminator.value, Terminator::Call { name, .. } if name == "index"));
        let write = function
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .find_map(|s| match &s.value {
                Statement::IndexAssign { value, .. } => Some(value),
                _ => None,
            })
            .unwrap();
        assert!(
            matches!(write, Operand::Move(place) if function.locals[place.local.0].name == "$expr0")
        );
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
    }

    #[test]
    fn array_wildcard_index_inspects_base_without_moving_element() {
        let item = Ty::Struct(nominal(vec![Ty::Int(IntTy::I32)]));
        let root = Ty::Array {
            element: std::sync::Arc::new(item.clone()),
            len: 2,
        };
        let mut source = with_root(
            root.clone(),
            vec![
                expr(root, ir::ExprKind::Local(LocalId(0))),
                expr(
                    item.clone(),
                    ir::ExprKind::Index {
                        base: ExprId(0),
                        index: 0,
                    },
                ),
            ],
            vec![ir::Stmt::LetPattern {
                value: ExprId(1),
                ty: item,
                bindings: vec![],
            }],
        );
        let function = nominal_lowered(&source);
        assert!(root_reads(&function).is_empty());
        let inspections: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .filter_map(|statement| match &statement.value {
                Statement::Inspect(place) => Some(place),
                _ => None,
            })
            .collect();
        assert_eq!(inspections, [&place(LocalId(0))]);
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
        source.body.statements.insert(0, ir::Stmt::Expr(ExprId(0)));
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
        source.body.statements[0] = ir::Stmt::Declare { local: LocalId(0) };
        source.params.clear();
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
    }

    #[test]
    fn array_index_copy_subfields_do_not_move_noncopy_parent() {
        let item = Ty::Struct(nominal(vec![Ty::Int(IntTy::I32)]));
        let root = Ty::Array {
            element: std::sync::Arc::new(item.clone()),
            len: 1,
        };
        let source = with_root(
            root.clone(),
            vec![
                expr(root, ir::ExprKind::Local(LocalId(0))),
                expr(
                    item,
                    ir::ExprKind::Index {
                        base: ExprId(0),
                        index: 0,
                    },
                ),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Field {
                        base: ExprId(1),
                        index: 0,
                    },
                ),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Field {
                        base: ExprId(1),
                        index: 0,
                    },
                ),
            ],
            vec![ir::Stmt::Expr(ExprId(2)), ir::Stmt::Expr(ExprId(3))],
        );
        let function = nominal_lowered(&source);
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 2);
        assert!(reads.iter().all(|operand| matches!(operand, Operand::Copy(place) if place.projection == [Projection::ConstantIndex(0), Projection::Field(0)])));
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
        assert_eq!(
            function
                .operand_ty(&Operand::Move(Place {
                    local: LocalId(0),
                    projection: vec![Projection::ConstantIndex(0), Projection::Field(0)]
                }))
                .unwrap(),
            Ty::Int(IntTy::I32)
        );
        assert!(
            function
                .operand_ty(&Operand::Move(Place {
                    local: LocalId(0),
                    projection: vec![Projection::ConstantIndex(0)]
                }))
                .is_err()
        );
    }

    #[test]
    fn zero_and_single_repeats_consume_noncopy_operand_once() {
        for len in [0, 1] {
            let item = Ty::Struct(nominal(vec![Ty::Int(IntTy::I32)]));
            let array = Ty::Array {
                element: std::sync::Arc::new(item.clone()),
                len,
            };
            let source = with_root(
                item.clone(),
                vec![
                    expr(item, ir::ExprKind::Local(LocalId(0))),
                    expr(
                        array,
                        ir::ExprKind::Repeat {
                            value: ExprId(0),
                            len,
                        },
                    ),
                ],
                vec![ir::Stmt::Expr(ExprId(1))],
            );
            let function = nominal_lowered(&source);
            assert_eq!(root_reads(&function).len(), 1);
            assert!(matches!(root_reads(&function)[0], Operand::Move(_)));
            assert_eq!(
                function
                    .blocks
                    .iter()
                    .flat_map(|b| &b.statements)
                    .filter(|s| matches!(
                        s.value,
                        Statement::Assign {
                            value: Rvalue::Repeat { .. },
                            ..
                        }
                    ))
                    .count(),
                1
            );
            assert!(super::super::ownership::analyze(&function).passed());
            assert_storage_paths(&function);
        }
    }

    #[test]
    fn indexed_assignment_inspects_after_rhs_initializes_array() {
        let root = Ty::Array {
            element: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            len: 1,
        };
        let destination = Place {
            local: LocalId(0),
            projection: vec![Projection::ConstantIndex(0)],
        };
        let mut source = with_root(
            root.clone(),
            vec![
                expr(Ty::Int(IntTy::I32), ir::ExprKind::Int(IntValue::i32(1))),
                expr(root, ir::ExprKind::Array(vec![ExprId(0)])),
                expr(Ty::Int(IntTy::I32), ir::ExprKind::Int(IntValue::i32(7))),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Block(ir::Block {
                        statements: vec![ir::Stmt::Assign {
                            local: LocalId(0),
                            value: ExprId(1),
                        }],
                        tail: Some(ExprId(2)),
                    }),
                ),
            ],
            vec![
                ir::Stmt::Declare { local: LocalId(0) },
                ir::Stmt::AssignPlace {
                    place: destination,
                    value: ExprId(3),
                },
            ],
        );
        source.params.clear();
        let function = nominal_lowered(&source);
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
        let statements: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .collect();
        let initialize = statements.iter().position(|s| matches!(&s.value, Statement::Assign { destination, .. } if destination.local == LocalId(0) && destination.projection.is_empty())).unwrap();
        let inspect = statements
            .iter()
            .position(|s| matches!(&s.value, Statement::Inspect(p) if p.local == LocalId(0)))
            .unwrap();
        let write = statements.iter().position(|s| matches!(&s.value, Statement::Assign { destination, .. } if destination.local == LocalId(0) && !destination.projection.is_empty())).unwrap();
        assert!(initialize < inspect && inspect < write);
        let ir::Stmt::AssignPlace { value, .. } = &mut source.body.statements[1] else {
            unreachable!()
        };
        *value = ExprId(2);
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
    }

    #[test]
    fn nested_array_index_inspects_each_exact_base_prefix_once() {
        let child = Ty::Array {
            element: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            len: 1,
        };
        let root = Ty::Array {
            element: std::sync::Arc::new(child.clone()),
            len: 1,
        };
        let source = with_root(
            root.clone(),
            vec![
                expr(root, ir::ExprKind::Local(LocalId(0))),
                expr(
                    child,
                    ir::ExprKind::Index {
                        base: ExprId(0),
                        index: 0,
                    },
                ),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Index {
                        base: ExprId(1),
                        index: 0,
                    },
                ),
            ],
            vec![ir::Stmt::LetPattern {
                value: ExprId(2),
                ty: Ty::Int(IntTy::I32),
                bindings: vec![],
            }],
        );
        let function = nominal_lowered(&source);
        let inspections: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .filter_map(|s| match &s.value {
                Statement::Inspect(place) => Some(place.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            inspections,
            [
                place(LocalId(0)),
                Place {
                    local: LocalId(0),
                    projection: vec![Projection::ConstantIndex(0)]
                }
            ]
        );
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
    }

    #[test]
    fn scalar_field_reads_copy_only_the_projection_before_whole_struct_move() {
        let root = Ty::Struct(nominal(vec![Ty::Int(IntTy::I32)]));
        let source = with_root(
            root.clone(),
            vec![
                expr(root, ir::ExprKind::Local(LocalId(0))),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Field {
                        base: ExprId(0),
                        index: 0,
                    },
                ),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Field {
                        base: ExprId(0),
                        index: 0,
                    },
                ),
            ],
            vec![
                ir::Stmt::Expr(ExprId(1)),
                ir::Stmt::Expr(ExprId(2)),
                ir::Stmt::Expr(ExprId(0)),
            ],
        );
        let function = nominal_lowered(&source);
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 3);
        assert!(matches!(reads[0], Operand::Copy(p) if p.projection == [0].map(Projection::Field)));
        assert!(matches!(reads[1], Operand::Copy(p) if p.projection == [0].map(Projection::Field)));
        assert!(matches!(reads[2], Operand::Move(p) if p.projection.is_empty()));
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn non_copy_field_moves_preserve_siblings_but_not_parent_or_moved_field() {
        let inner = Ty::Struct(nominal(vec![Ty::Int(IntTy::I32)]));
        let root = Ty::Struct(nominal(vec![inner.clone(), inner.clone()]));
        let mut source = with_root(
            root.clone(),
            vec![
                expr(root, ir::ExprKind::Local(LocalId(0))),
                expr(
                    inner.clone(),
                    ir::ExprKind::Field {
                        base: ExprId(0),
                        index: 0,
                    },
                ),
                expr(
                    inner,
                    ir::ExprKind::Field {
                        base: ExprId(0),
                        index: 1,
                    },
                ),
            ],
            vec![ir::Stmt::Expr(ExprId(1)), ir::Stmt::Expr(ExprId(2))],
        );
        let function = nominal_lowered(&source);
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 2);
        assert!(matches!(reads[0], Operand::Move(p) if p.projection == [0].map(Projection::Field)));
        assert!(matches!(reads[1], Operand::Move(p) if p.projection == [1].map(Projection::Field)));
        assert!(super::super::ownership::analyze(&function).passed());
        source.body.statements.push(ir::Stmt::Expr(ExprId(0)));
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
        source.body.statements.pop();
        source.body.statements.push(ir::Stmt::Expr(ExprId(1)));
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
    }

    #[test]
    fn arbitrary_expression_base_moves_whole_source_once_before_projection() {
        let inner = Ty::Struct(nominal(vec![Ty::Int(IntTy::I32)]));
        let root = Ty::Struct(nominal(vec![inner.clone(), Ty::Int(IntTy::I32)]));
        let mut source = with_root(
            root.clone(),
            vec![
                expr(root.clone(), ir::ExprKind::Local(LocalId(0))),
                expr(
                    root,
                    ir::ExprKind::Block(ir::Block {
                        statements: vec![],
                        tail: Some(ExprId(0)),
                    }),
                ),
                expr(
                    inner,
                    ir::ExprKind::Field {
                        base: ExprId(1),
                        index: 0,
                    },
                ),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Field {
                        base: ExprId(0),
                        index: 1,
                    },
                ),
            ],
            vec![ir::Stmt::Expr(ExprId(2))],
        );
        let function = nominal_lowered(&source);
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 1);
        assert!(matches!(reads[0], Operand::Move(p) if p.projection.is_empty()));
        assert!(super::super::ownership::analyze(&function).passed());
        source.body.statements.push(ir::Stmt::Expr(ExprId(3)));
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
    }

    #[test]
    fn struct_initializer_evaluates_source_order_then_permutates_snapshots() {
        let ty = nominal(vec![Ty::Int(IntTy::I32), Ty::Int(IntTy::I32)]);
        let call = |name: &str| {
            expr(
                Ty::Int(IntTy::I32),
                ir::ExprKind::Call {
                    target: test_target(),
                    name: name.into(),
                    args: vec![],
                    signature: ir::Signature {
                        ref_contract: Default::default(),
                        params: vec![],
                        result: Ty::Int(IntTy::I32),
                    },
                },
            )
        };
        let source = with_root(
            Ty::Int(IntTy::I32),
            vec![
                call("first"),
                call("second"),
                expr(
                    Ty::Struct(ty.clone()),
                    ir::ExprKind::Struct {
                        ty,
                        fields: vec![(1, ExprId(0)), (0, ExprId(1))],
                    },
                ),
            ],
            vec![ir::Stmt::Expr(ExprId(2))],
        );
        let function = nominal_lowered(&source);
        let calls: Vec<_> = function
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator.value {
                Terminator::Call { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, ["first", "second"]);
        let aggregate = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match &statement.value {
                Statement::Assign {
                    value: Rvalue::Aggregate { fields, .. },
                    ..
                } => Some(fields),
                _ => None,
            })
            .unwrap();
        assert!(
            matches!(&aggregate[0], Operand::Copy(p) if function.locals[p.local.0].name == "$expr1")
        );
        assert!(
            matches!(&aggregate[1], Operand::Copy(p) if function.locals[p.local.0].name == "$expr0")
        );
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn payload_constructor_evaluates_source_order_then_permutates_snapshots() {
        let ty = ir::EnumTy::new(ir::EnumDef {
            source_decl: test_decl(),
            type_args: std::sync::Arc::from([]),
            id: ir::NominalAppId {
                snapshot: ir::SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "E".into(),
            module: String::new(),
            public: false,
            variants: vec![
                ir::VariantDef {
                    name: "A".into(),
                    shape: ir::VariantShape::Struct,
                    fields: vec![
                        ir::FieldDef {
                            name: "a".into(),
                            ty: Ty::Int(IntTy::I32),
                            public: true,
                        },
                        ir::FieldDef {
                            name: "b".into(),
                            ty: Ty::Int(IntTy::I32),
                            public: true,
                        },
                    ]
                    .into(),
                },
                ir::VariantDef {
                    name: "B".into(),
                    shape: ir::VariantShape::Unit,
                    fields: std::sync::Arc::from([]),
                },
            ]
            .into(),
        });
        let call = |name: &str| {
            expr(
                Ty::Int(IntTy::I32),
                ir::ExprKind::Call {
                    target: test_target(),
                    name: name.into(),
                    args: vec![],
                    signature: ir::Signature {
                        ref_contract: Default::default(),
                        params: vec![],
                        result: Ty::Int(IntTy::I32),
                    },
                },
            )
        };
        let source = with_root(
            Ty::Int(IntTy::I32),
            vec![
                call("first"),
                call("second"),
                expr(
                    Ty::Enum(ty.clone()),
                    ir::ExprKind::EnumVariant {
                        ty: ty.clone(),
                        variant: 0,
                        fields: vec![(1, ExprId(0)), (0, ExprId(1))],
                    },
                ),
            ],
            vec![ir::Stmt::Expr(ExprId(2))],
        );
        source.validate().unwrap();
        let function = nominal_lowered(&source);
        let calls: Vec<_> = function
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator.value {
                Terminator::Call { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls, ["first", "second"]);
        let fields = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match &statement.value {
                Statement::Assign {
                    value: Rvalue::EnumVariant { fields, .. },
                    ..
                } => Some(fields),
                _ => None,
            })
            .unwrap();
        let local_name = |operand: &Operand| match operand {
            Operand::Copy(place) => function.locals[place.local.0].name.as_str(),
            _ => panic!("expected copied scalar snapshot"),
        };
        assert_eq!(local_name(&fields[0]), "$expr1");
        assert_eq!(local_name(&fields[1]), "$expr0");
        assert!(
            function
                .operand_ty(&Operand::Constant(Constant::Enum { ty, variant: 1 }))
                .is_err()
        );
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
    }

    #[test]
    fn payload_matches_use_variant_switches_and_selected_field_moves() {
        let source = checked(
            "struct S{x:i32} enum E{A(S,i32),B} fn f(e:E)->i32{match e{E::A(s,n)=>s.x+n,E::B=>0}} fn main(){}",
            0,
        );
        source.validate().unwrap();
        let function = nominal_lowered(&source);
        assert_eq!(
            function
                .blocks
                .iter()
                .filter(|block| matches!(block.terminator.value, Terminator::SwitchVariant { .. }))
                .count(),
            2
        );
        let reads = root_reads(&function);
        assert!(reads.iter().any(|operand| matches!(operand, Operand::Move(place) if matches!(place.projection.as_slice(), [Projection::Downcast(_), Projection::Field(0)]))));
        assert!(reads.iter().any(|operand| matches!(operand, Operand::Copy(place) if matches!(place.projection.as_slice(), [Projection::Downcast(_), Projection::Field(1)]))));
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
    }

    #[test]
    fn singleton_payload_let_extracts_disjoint_fields_without_tag_read() {
        let source = checked(
            "struct S{x:i32} enum E{A(S,S)} fn f(e:E){let E::A(a,_)=e;let E::A(_,b)=e;let E::A(_,_) = e;} fn main(){}",
            0,
        );
        let function = nominal_lowered(&source);
        assert!(
            !function
                .blocks
                .iter()
                .any(|block| matches!(block.terminator.value, Terminator::SwitchVariant { .. }))
        );
        assert_eq!(root_reads(&function).len(), 2);
        assert!(super::super::ownership::analyze(&function).passed());
        assert_storage_paths(&function);
    }

    #[test]
    fn validator_rejects_non_copy_operands_even_in_disconnected_blocks() {
        let root = Ty::Struct(nominal(vec![Ty::Int(IntTy::I32)]));
        let source = with_root(root.clone(), vec![], vec![]);
        let mut function = nominal_lowered(&source);
        let target = LocalId(function.locals.len());
        function.locals.push(ir::LocalDecl {
            name: "target".into(),
            ty: root,
            mutable: true,
        });
        function.blocks.push(BasicBlock {
            statements: vec![Located {
                value: Statement::Assign {
                    destination: place(target),
                    value: Rvalue::Use(Operand::Copy(place(LocalId(0)))),
                },
                origin: None,
            }],
            terminator: Located {
                value: Terminator::Unreachable,
                origin: None,
            },
        });
        assert!(function.validate().unwrap_err().contains("Copy type"));
        function.locals[0].ty = Ty::Tuple(vec![function.locals[0].ty.clone()].into());
        assert!(!function.locals[0].ty.is_copy());
        assert!(function.validate().unwrap_err().contains("Copy type"));
    }

    fn checked(source: &str, index: usize) -> ir::CheckedFunction {
        let program = super::super::frontend::parse(source).unwrap();
        let signatures = super::super::frontend::signature_table(&program).unwrap();
        super::super::frontend::type_function_with_signatures(&program, index, &signatures).unwrap()
    }

    #[test]
    fn deferred_declarations_start_live_without_initializing_storage() {
        let function = lower(&checked("fn f(){let x:i32;} fn main(){}", 0)).unwrap();
        function.validate().unwrap();
        let local = LocalId(0);
        assert!(
            function
                .blocks
                .iter()
                .flat_map(|b| &b.statements)
                .any(|s| matches!(s.value, Statement::StorageLive(id) if id == local))
        );
        assert!(!function.blocks.iter().flat_map(|b| &b.statements).any(|s| matches!(&s.value, Statement::Assign { destination, .. } if destination.local == local)));
        assert!(
            super::super::ownership::analyze(&function)
                .diagnostics
                .is_empty()
        );
        assert_storage_paths(&function);
    }

    #[test]
    fn empty_never_match_has_no_reachable_result_read() {
        let source = checked("fn f()->i32{match loop{} {}} fn main(){}", 0);
        source.validate().unwrap();
        let function = lower(&source).unwrap();
        function.validate().unwrap();
        assert!(
            super::super::ownership::analyze(&function)
                .diagnostics
                .is_empty()
        );
        assert_storage_paths(&function);
    }

    #[test]
    fn deferred_immutable_first_initialization_joins_both_branches() {
        let function = lower(&checked(
            "fn f(flag:bool)->i32{let x:i32;if flag{x=1;}else{x=2;}x} fn main(){}",
            0,
        ))
        .unwrap();
        function.validate().unwrap();
        assert!(
            super::super::ownership::analyze(&function)
                .diagnostics
                .is_empty()
        );
        assert_storage_paths(&function);
    }

    #[test]
    fn deferred_reads_reassignment_and_loop_backedges_are_rejected() {
        for source in [
            "fn f()->i32{let x:i32;x} fn main(){}",
            "fn f(){let x:i32;x=1;x=2;} fn main(){}",
            "fn f(flag:bool)->i32{let x:i32;if flag{x=1;}x} fn main(){}",
            "fn f(flag:bool){let x:i32;while flag{x=1;}} fn main(){}",
        ] {
            let function = lower(&checked(source, 0)).unwrap();
            function.validate().unwrap();
            assert!(
                !super::super::ownership::analyze(&function)
                    .diagnostics
                    .is_empty(),
                "{source}"
            );
        }
    }

    #[test]
    fn enum_match_wildcard_and_singleton_do_not_read_scrutinee() {
        for source in [
            "enum E{A,B} fn f(e:E){let moved=e;match e{_=>()};} fn main(){}",
            "enum E{A} fn f(e:E){let moved=e;match e{E::A=>()};} fn main(){}",
        ] {
            let function = nominal_lowered(&checked(source, 0));
            assert!(
                !function
                    .blocks
                    .iter()
                    .flat_map(|b| &b.statements)
                    .any(|s| matches!(
                        s.value,
                        Statement::Assign {
                            value: Rvalue::Discriminant(_),
                            ..
                        }
                    ))
            );
            assert_storage_paths(&function);
        }
    }

    #[test]
    fn enum_match_tests_read_tags_but_only_selected_binding_moves() {
        let function = nominal_lowered(&checked(
            "enum E{A,B} fn f(e:E)->E{match e{E::A=>E::B,other=>other}} fn main(){}",
            0,
        ));
        assert_storage_paths(&function);
        let tags = function
            .blocks
            .iter()
            .filter(|block| matches!(block.terminator.value, Terminator::SwitchVariant { .. }))
            .count();
        assert_eq!(tags, 1);
    }

    #[test]
    fn dead_match_arms_preserve_ownership_reads_and_postmatch_joins() {
        for source in [
            "enum E{A,B} fn f(e:E){let moved=e;match e{_=>(),E::A=>()};} fn main(){}",
            "enum E{A,B} fn f(e:E){match true{_=>(),true=>{let moved=e;}};let after=e;} fn main(){}",
        ] {
            let function = lower(&checked(source, 0)).unwrap();
            function.validate().unwrap();
            assert!(
                !super::super::ownership::analyze(&function)
                    .diagnostics
                    .is_empty()
            );
        }
    }

    #[test]
    fn nominal_branch_and_loop_results_transfer_ownership_without_copy() {
        for source in [
            "struct S{x:i32} fn f(p:S,flag:bool)->S{if flag{p}else{p}} fn main(){}",
            "struct S{x:i32} fn f(p:S)->S{loop{break p;}} fn main(){}",
            "struct S{x:i32} fn f(p:S)->(S,i32){(p,7)} fn main(){}",
        ] {
            let function = nominal_lowered(&checked(source, 0));
            assert!(
                super::super::ownership::analyze(&function).passed(),
                "{source}"
            );
            for statement in function.blocks.iter().flat_map(|block| &block.statements) {
                if let Statement::Assign {
                    value: Rvalue::Use(Operand::Copy(place)),
                    ..
                } = &statement.value
                {
                    assert!(function.place_ty(place).unwrap().is_copy());
                }
            }
        }
    }

    #[test]
    fn nested_place_projection_does_not_consume_non_copy_intermediate_field() {
        let source = "struct I{x:i32} struct S{child:I} fn main(){let s=S{child:I{x:1}};let a=s.child.x;let b=s.child.x;let child=s.child;}";
        let function = nominal_lowered(&checked(source, 0));
        assert!(super::super::ownership::analyze(&function).passed());
        let s = LocalId(
            function
                .locals
                .iter()
                .position(|local| local.name == "s")
                .unwrap(),
        );
        let reads: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.value {
                Statement::Assign {
                    value: Rvalue::Use(operand),
                    ..
                } => match operand {
                    Operand::Copy(p) | Operand::Move(p) if p.local == s => Some(operand),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(reads.len(), 3);
        assert!(
            matches!(reads[0], Operand::Copy(p) if p.projection == [0, 0].map(Projection::Field))
        );
        assert!(
            matches!(reads[1], Operand::Copy(p) if p.projection == [0, 0].map(Projection::Field))
        );
        assert!(matches!(reads[2], Operand::Move(p) if p.projection == [0].map(Projection::Field)));
    }

    #[test]
    fn nominal_argument_moves_are_observed_before_later_arguments() {
        let source = "struct S{x:i32} fn take(s:S,n:i32){} fn main(){let s=S{x:1};take(s,s.x);}";
        let function = nominal_lowered(&checked(source, 1));
        assert!(!super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn wildcard_patterns_do_not_read_previously_moved_struct_places() {
        let source =
            "struct S{x:i32} fn f(p:S){let moved=p;let _=p;let _=(p);let S{x:_}=p;} fn main(){}";
        let function = nominal_lowered(&checked(source, 0));
        assert!(super::super::ownership::analyze(&function).passed());
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 1);
        assert!(matches!(reads[0], Operand::Move(p) if p.projection.is_empty()));
    }

    #[test]
    fn partial_move_pattern_reads_only_selected_sibling_in_source_order() {
        let source =
            "struct S{x:i32} fn f(p:(S,S)){let first=p.0;let(_,second)=p;let(_,_) =p;} fn main(){}";
        let function = nominal_lowered(&checked(source, 0));
        assert!(super::super::ownership::analyze(&function).passed());
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 2);
        assert!(matches!(reads[0], Operand::Move(p) if p.projection == [0].map(Projection::Field)));
        assert!(matches!(reads[1], Operand::Move(p) if p.projection == [1].map(Projection::Field)));

        let source = "struct S{x:i32} struct P{first:S,second:S} fn f(p:P){let P{second:b,first:a}=p;} fn main(){}";
        let function = nominal_lowered(&checked(source, 0));
        assert!(super::super::ownership::analyze(&function).passed());
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 2);
        assert!(matches!(reads[0], Operand::Move(p) if p.projection == [1].map(Projection::Field)));
        assert!(matches!(reads[1], Operand::Move(p) if p.projection == [0].map(Projection::Field)));
    }

    #[test]
    fn wildcard_on_temporary_field_calls_once_without_consuming_that_field() {
        let source = "struct S{x:i32} struct O{child:S} fn make()->O{O{child:S{x:1}}} fn main(){let _=make().child;}";
        let function = nominal_lowered(&checked(source, 1));
        assert!(super::super::ownership::analyze(&function).passed());
        assert_eq!(function.blocks.iter().filter(|block| matches!(&block.terminator.value, Terminator::Call {name, ..} if name == "make")).count(), 1);
        assert!(
            !function
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .any(|statement| {
                    matches!(
                        &statement.value,
                        Statement::Assign {
                            value: Rvalue::Use(Operand::Copy(_) | Operand::Move(_)),
                            ..
                        }
                    )
                })
        );
    }

    #[test]
    fn wildcard_rvalue_tuple_still_consumes_its_input_expressions() {
        let source = "struct S{x:i32} fn f(a:S,b:S){let(_,_)=(a,b);} fn main(){}";
        let function = nominal_lowered(&checked(source, 0));
        assert!(super::super::ownership::analyze(&function).passed());
        assert_eq!(root_reads(&function).len(), 1);
        for local in [LocalId(0), LocalId(1)] {
            assert_eq!(function.blocks.iter().flat_map(|block| &block.statements).filter(|statement| {
                matches!(&statement.value, Statement::Assign {value: Rvalue::Use(Operand::Move(p)), ..} if p.local == local && p.projection.is_empty())
            }).count(), 1);
        }
    }

    #[test]
    fn unit_pattern_is_not_a_read_but_unit_binding_is() {
        let function = nominal_lowered(&checked("fn f(u:()){let ()=u;let real=u;} fn main(){}", 0));
        assert!(super::super::ownership::analyze(&function).passed());
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 1);
        assert!(matches!(reads[0], Operand::Copy(p) if p.projection.is_empty()));
    }

    #[test]
    fn divergent_pattern_initializer_has_effects_but_no_impossible_binding_reads() {
        let source = "fn f(){let(a,b):(i32,bool)={return;};} fn main(){}";
        let function = nominal_lowered(&checked(source, 0));
        assert!(super::super::ownership::analyze(&function).passed());
        assert!(!function.blocks.iter().flat_map(|block| &block.statements).any(|statement| {
            matches!(&statement.value, Statement::Assign {destination, ..} if matches!(function.locals[destination.local.0].name.as_str(), "a" | "b"))
        }));
    }

    #[test]
    fn cyclic_pattern_initializer_projection_fails_at_the_depth_bound() {
        let mut source = with_root(
            Ty::Int(IntTy::I32),
            vec![expr(
                Ty::Int(IntTy::I32),
                ir::ExprKind::Field {
                    base: ExprId(0),
                    index: 0,
                },
            )],
            vec![],
        );
        source.body.statements.push(ir::Stmt::LetPattern {
            value: ExprId(0),
            ty: Ty::Int(IntTy::I32),
            bindings: vec![],
        });
        assert!(lower(&source).unwrap_err().contains("depth"));
    }

    fn live_operand(value: &Operand, live: &[bool]) {
        if let Operand::Copy(place) | Operand::Move(place) = value {
            assert!(
                live[place.local.0],
                "operand local {} is not live",
                place.local.0
            );
        }
    }

    // Exhaust all distinct reachable (block, storage-state) combinations for
    // these small fixtures, including loop backedges. This is a test oracle for
    // lifetime markers, not an initialization/move/ownership implementation.
    fn assert_storage_paths(function: &Function) {
        let mut live = vec![false; function.locals.len()];
        for param in &function.params {
            live[param.0] = true;
        }
        let mut pending = vec![(function.entry, live)];
        let mut seen = HashSet::new();
        while let Some((id, mut live)) = pending.pop() {
            if !seen.insert((id, live.clone())) {
                continue;
            }
            assert!(seen.len() < 10_000, "fixture state exploration limit");
            for statement in &function.blocks[id.0].statements {
                match &statement.value {
                    Statement::IndexAssign {
                        array,
                        index,
                        value,
                    }
                    | Statement::IndexAssignOp {
                        array,
                        index,
                        value,
                        ..
                    } => {
                        live_operand(value, &live);
                        live_operand(index, &live);
                        assert!(live[array.local.0], "indexed assignment reads dead array");
                    }
                    Statement::IndexInspect { array, index } => {
                        live_operand(index, &live);
                        assert!(live[array.local.0], "indexed inspection reads dead array");
                    }
                    Statement::Inspect(place) => {
                        assert!(live[place.local.0], "inspection reads dead storage");
                    }
                    Statement::StorageLive(local) => {
                        assert!(
                            !live[local.0],
                            "double StorageLive of {} in block {}",
                            local.0, id.0
                        );
                        live[local.0] = true;
                    }
                    Statement::StorageDead(local) => {
                        assert!(
                            live[local.0],
                            "StorageDead of inactive {} in block {}",
                            local.0, id.0
                        );
                        live[local.0] = false;
                    }
                    Statement::Assign { destination, value } => {
                        assert!(live[destination.local.0], "destination local is not live");
                        match value {
                            Rvalue::Borrow(loan) => {
                                assert!(
                                    live[function.loan_sites[loan.0].place.local.0],
                                    "borrow of dead storage"
                                );
                            }
                            Rvalue::IndexCopy { array, index } => {
                                live_operand(index, &live);
                                assert!(live[array.local.0], "dynamic index reads dead array");
                            }
                            Rvalue::Use(value)
                            | Rvalue::IntCast { value, .. }
                            | Rvalue::Unary { value, .. }
                            | Rvalue::Repeat { value, .. } => live_operand(value, &live),
                            Rvalue::Binary { left, right, .. } => {
                                live_operand(left, &live);
                                live_operand(right, &live);
                            }
                            Rvalue::Aggregate { fields, .. }
                            | Rvalue::EnumVariant { fields, .. } => {
                                for value in fields {
                                    live_operand(value, &live);
                                }
                            }
                            Rvalue::Discriminant(place) => {
                                assert!(live[place.local.0], "discriminant reads dead storage");
                            }
                            Rvalue::Println(_) => {}
                        }
                    }
                }
            }
            match &function.blocks[id.0].terminator.value {
                Terminator::SwitchVariant {
                    place,
                    then_block,
                    else_block,
                    ..
                } => {
                    assert!(live[place.local.0], "variant switch reads dead storage");
                    pending.push((*then_block, live.clone()));
                    pending.push((*else_block, live));
                }
                Terminator::Goto(next) => pending.push((*next, live)),
                Terminator::Switch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    live_operand(condition, &live);
                    pending.push((*then_block, live.clone()));
                    pending.push((*else_block, live));
                }
                Terminator::Call {
                    args,
                    destination,
                    next,
                    ..
                } => {
                    for arg in args {
                        live_operand(arg, &live);
                    }
                    assert!(live[destination.local.0]);
                    pending.push((*next, live));
                }
                Terminator::Return => {
                    assert!(live[function.return_local.0]);
                    assert!(
                        live.iter()
                            .enumerate()
                            .all(|(index, live)| !live || index == function.return_local.0),
                        "storage survives return cleanup"
                    );
                }
                Terminator::Unreachable => {}
            }
        }
    }

    #[test]
    fn lifetime_markers_match_reachable_branches_loops_and_abrupt_exits() {
        for source in [
            "fn f(n:i32)->(i32,bool){let x=(1,true);while n>0 {if n==1 {break;} else {continue;}} if n==2 {return x;} x} fn main(){}",
            "fn f(n:i32){let a=n>0 && {let t=(1,true);t.1};let b=n<1 || {let x=2;x==2};}fn main(){}",
            "fn f()->i32{1+{return 3;0}}fn main(){}",
            "fn f()->i32{loop{break {return 3;0};}}fn main(){}",
            "fn f()->i32{return {return 3;};}fn main(){}",
            "fn f(n:i32)->i32{loop{let a=(1,2);if n>0{continue;}break a.0;}}fn main(){}",
            "fn f()->i32{if {return 3;} {1}else{2}}fn main(){}",
        ] {
            let function = lower(&checked(source, 0)).unwrap();
            function.validate().unwrap();
            assert_storage_paths(&function);
        }
    }
}
