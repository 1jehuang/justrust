//! Per-function control-flow lowering of immutable checked bodies.
//!
//! Explicit value snapshots and edges preserve source evaluation order. This
//! module selects Copy/Move operands from projected semantic types. Ownership
//! dataflow is established separately by the analyzer before native emission.
use super::frontend::{BinaryOp, UnaryOp};
use super::ir::{self, ExprId, LocalId, LoopId, Place, Ty};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

#[derive(Clone, Debug)]
pub struct Located<T> {
    pub value: T,
    pub origin: Option<ExprId>,
}

#[derive(Clone, Debug)]
pub struct Function {
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
    StorageLive(LocalId),
    StorageDead(LocalId),
    Assign { destination: Place, value: Rvalue },
}

#[derive(Clone, Debug)]
pub enum Constant {
    Enum { ty: ir::EnumTy, variant: usize },
    I32(i32),
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
    Goto(BlockId),
    Switch {
        condition: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },
    Call {
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

fn coerces(got: &Ty, want: &Ty) -> bool {
    if got == &Ty::Never || got == want {
        return true;
    }
    match (got, want) {
        (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
            a.iter().zip(b.iter()).all(|(a, b)| coerces(a, b))
        }
        _ => false,
    }
}

fn validate_type(ty: &Ty, depth: usize) -> Result<(), String> {
    if let Ty::Enum(ty) = ty {
        ty.validate()?;
    }
    if depth > 128 {
        return Err("CFG type nesting exceeds limit 128".into());
    }
    if let Some(count) = ty.field_count() {
        if count == 0 {
            return Err("empty CFG aggregates are unsupported; empty tuples must be Unit".into());
        }
        for index in 0..count {
            validate_type(
                ty.field_ty(index).ok_or("invalid CFG aggregate field")?,
                depth + 1,
            )?;
        }
    }
    Ok(())
}

impl Function {
    pub fn place_ty(&self, place: &Place) -> Result<&Ty, String> {
        let mut ty = &self
            .locals
            .get(place.local.0)
            .ok_or("invalid CFG local")?
            .ty;
        for &index in &place.fields {
            if ty.field_count().is_none() {
                return Err("CFG projection requires an aggregate".into());
            }
            ty = ty
                .field_ty(index)
                .ok_or("CFG projection index out of bounds")?;
        }
        Ok(ty)
    }

    pub fn operand_ty(&self, operand: &Operand) -> Result<Ty, String> {
        match operand {
            Operand::Constant(Constant::I32(_)) => Ok(Ty::I32),
            Operand::Constant(Constant::Bool(_)) => Ok(Ty::Bool),
            Operand::Constant(Constant::Unit) => Ok(Ty::Unit),
            Operand::Constant(Constant::Enum { ty, variant }) => {
                ty.validate()?;
                if *variant >= ty.def().variants.len() {
                    return Err("invalid CFG enum variant".into());
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
            Operand::Move(place) => self.place_ty(place).cloned(),
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

    fn rvalue_ty(&self, value: &Rvalue) -> Result<Ty, String> {
        match value {
            Rvalue::Discriminant(place) => {
                let Ty::Enum(ty) = self.place_ty(place)? else {
                    return Err("CFG discriminant requires enum place".into());
                };
                ty.validate()?;
                Ok(Ty::I32)
            }
            Rvalue::Use(operand) => self.operand_ty(operand),
            Rvalue::Println(_) => Ok(Ty::Unit),
            Rvalue::Unary { op, value } => {
                let ty = self.operand_ty(value)?;
                if matches!(op, UnaryOp::Neg) && ty == Ty::Never {
                    return Err("CFG negation of never is unsupported".into());
                }
                let want = match op {
                    UnaryOp::Neg => Ty::I32,
                    UnaryOp::Not => Ty::Bool,
                };
                self.require_operand(value, &want)?;
                Ok(if ty == Ty::Never { Ty::Never } else { want })
            }
            Rvalue::Binary { op, left, right } => {
                let left_ty = self.operand_ty(left)?;
                let right_ty = self.operand_ty(right)?;
                if left_ty == Ty::Never || right_ty == Ty::Never {
                    return Err("CFG binary operator on never is unsupported".into());
                }
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    return Err("CFG short-circuit operator must be control flow".into());
                }
                let comparison = matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                );
                let operand_ty = if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
                    if left_ty == Ty::Never {
                        right_ty
                    } else {
                        left_ty
                    }
                } else {
                    Ty::I32
                };
                if operand_ty.field_count().is_some() || matches!(operand_ty, Ty::Enum(_)) {
                    return Err("CFG aggregate comparison is unsupported".into());
                }
                self.require_operand(left, &operand_ty)?;
                self.require_operand(right, &operand_ty)?;
                Ok(if comparison { Ty::Bool } else { Ty::I32 })
            }
            Rvalue::Aggregate { ty, fields } => {
                validate_type(ty, 0)?;
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
    pub fn validate(&self) -> Result<(), String> {
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
        for item in &self.blocks {
            for statement in &item.statements {
                match &statement.value {
                    Statement::StorageLive(local) | Statement::StorageDead(local) => {
                        self.place_ty(&place(*local))?;
                    }
                    Statement::Assign { destination, value } => {
                        let got = self.rvalue_ty(value)?;
                        let want = self.place_ty(destination)?;
                        if !coerces(&got, want) {
                            return Err("CFG assignment type mismatch".into());
                        }
                    }
                }
            }
            match &item.terminator.value {
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
                    signature,
                    args,
                    destination,
                    next,
                    ..
                } => {
                    if signature.params.len() != args.len() {
                        return Err("CFG call arity mismatch".into());
                    }
                    for ty in &signature.params {
                        validate_type(ty, 0)?;
                    }
                    validate_type(&signature.result, 0)?;
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
        Ok(())
    }
}

fn place(local: LocalId) -> Place {
    Place {
        local,
        fields: Vec::new(),
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
    fn temporary(&mut self, ty: Ty, origin: Option<ExprId>) -> Place {
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
                    Some((Operand::Copy(source.clone()), Constant::I32(*value)))
                }
                ir::MatchPattern::EnumVariant { ty, variant, .. }
                    if completes && ty.def().variants.len() > 1 =>
                {
                    let tag = self.temporary(Ty::I32, Some(id));
                    self.assign(tag.clone(), Rvalue::Discriminant(source.clone()), Some(id));
                    Some((Operand::Copy(tag), Constant::I32(*variant as i32)))
                }
                _ => None,
            };
            if let Some((left, right)) = comparison.filter(|_| completes) {
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
        let id = BlockId(self.blocks.len());
        self.blocks.push(PendingBlock {
            statements: Vec::new(),
            terminator: None,
        });
        id
    }

    fn statement(&mut self, value: Statement, origin: Option<ExprId>) {
        self.blocks[self.current.0]
            .statements
            .push(Located { value, origin });
    }

    fn terminate(&mut self, value: Terminator, origin: Option<ExprId>) -> Result<(), String> {
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

    /// Place context does not read or consume its aggregate base. Only the
    /// selected subtree is read when this place enters a value context.
    fn expression_place(&self, id: ExprId) -> Option<Place> {
        match &self.source.exprs.get(id.0)?.kind {
            ir::ExprKind::Local(local) => Some(place(*local)),
            ir::ExprKind::Field { base, index } => {
                let mut p = self.expression_place(*base)?;
                p.fields.push(*index);
                Some(p)
            }
            _ => None,
        }
    }

    fn consume(&self, place: Place) -> Result<Operand, String> {
        if self.result.place_ty(&place)?.is_copy() {
            Ok(Operand::Copy(place))
        } else {
            Ok(Operand::Move(place))
        }
    }

    /// Evaluate only the non-place root of a pattern initializer. A field chain
    /// remains an address even when rooted in a call/temporary: ignoring that
    /// field must not accidentally load or move it merely to obtain its place.
    fn pattern_source(&mut self, id: ExprId) -> Result<Place, String> {
        let mut root = id;
        let mut fields = Vec::new();
        let mut source = loop {
            let expression = self
                .source
                .exprs
                .get(root.0)
                .ok_or("invalid pattern initializer expression")?;
            match &expression.kind {
                ir::ExprKind::Local(local) => break place(*local),
                ir::ExprKind::Field { base, index } => {
                    if fields.len() >= ir::MAX_PATTERN_PROJECTION_DEPTH {
                        return Err("pattern initializer projection depth exceeds limit 128".into());
                    }
                    fields.push(*index);
                    root = *base;
                }
                _ => break self.materialize_expression(root)?,
            }
        };
        source.fields.extend(fields.into_iter().rev());
        // Static metadata validation is not a value read. Moved and partially
        // initialized places are deliberately addressable in pattern context.
        self.result.place_ty(&source)?;
        Ok(source)
    }

    fn body(&mut self, body: &ir::Block, destination: Option<Place>) -> Result<(), String> {
        self.scopes.push(Vec::new());
        for statement in &body.statements {
            match statement {
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
                            selected.fields.extend_from_slice(&binding.fields);
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
            ir::ExprKind::EnumVariant { ty, variant } => {
                ty.validate()?;
                if expression.ty != Ty::Enum(ty.clone()) || *variant >= ty.def().variants.len() {
                    return Err("invalid checked enum variant expression".into());
                }
                self.assign(
                    destination.clone(),
                    Rvalue::Use(Operand::Constant(Constant::Enum {
                        ty: ty.clone(),
                        variant: *variant,
                    })),
                    origin,
                );
            }
            ir::ExprKind::Match { scrutinee, arms } => {
                self.match_expression(id, *scrutinee, arms, destination.clone())?;
            }
            ir::ExprKind::Int(value) => self.assign(
                destination.clone(),
                Rvalue::Use(Operand::Constant(Constant::I32(*value))),
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
            ir::ExprKind::Field { base, index } => {
                let p = if let Some(p) = self.expression_place(id) {
                    p
                } else {
                    let mut p = self.materialize_expression(*base)?;
                    p.fields.push(*index);
                    p
                };
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
                name,
                args,
                signature,
            } => {
                let args = args
                    .iter()
                    .map(|id| self.expression(*id))
                    .collect::<Result<Vec<_>, _>>()?;
                let next = self.block();
                self.terminate(
                    Terminator::Call {
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
        source,
        result: Function {
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
        blocks: Vec::new(),
        current: BlockId(0),
        scopes: vec![source.params.clone()],
        loops: Vec::new(),
    };
    lowerer.current = lowerer.block();
    lowerer.statement(Statement::StorageLive(return_local), None);
    if source.result == Ty::Unit {
        lowerer.assign(
            place(return_local),
            Rvalue::Use(Operand::Constant(Constant::Unit)),
            None,
        );
    }
    lowerer.body(&source.body, Some(place(return_local)))?;
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
    use super::*;
    use std::collections::HashSet;

    fn nominal(fields: Vec<Ty>) -> ir::StructTy {
        ir::StructTy::new(ir::StructDef {
            id: ir::DefId {
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
    fn scalar_field_reads_copy_only_the_projection_before_whole_struct_move() {
        let root = Ty::Struct(nominal(vec![Ty::I32]));
        let source = with_root(
            root.clone(),
            vec![
                expr(root, ir::ExprKind::Local(LocalId(0))),
                expr(
                    Ty::I32,
                    ir::ExprKind::Field {
                        base: ExprId(0),
                        index: 0,
                    },
                ),
                expr(
                    Ty::I32,
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
        assert!(matches!(reads[0], Operand::Copy(p) if p.fields == [0]));
        assert!(matches!(reads[1], Operand::Copy(p) if p.fields == [0]));
        assert!(matches!(reads[2], Operand::Move(p) if p.fields.is_empty()));
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn non_copy_field_moves_preserve_siblings_but_not_parent_or_moved_field() {
        let inner = Ty::Struct(nominal(vec![Ty::I32]));
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
        assert!(matches!(reads[0], Operand::Move(p) if p.fields == [0]));
        assert!(matches!(reads[1], Operand::Move(p) if p.fields == [1]));
        assert!(super::super::ownership::analyze(&function).passed());
        source.body.statements.push(ir::Stmt::Expr(ExprId(0)));
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
        source.body.statements.pop();
        source.body.statements.push(ir::Stmt::Expr(ExprId(1)));
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
    }

    #[test]
    fn arbitrary_expression_base_moves_whole_source_once_before_projection() {
        let inner = Ty::Struct(nominal(vec![Ty::I32]));
        let root = Ty::Struct(nominal(vec![inner.clone(), Ty::I32]));
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
                    Ty::I32,
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
        assert!(matches!(reads[0], Operand::Move(p) if p.fields.is_empty()));
        assert!(super::super::ownership::analyze(&function).passed());
        source.body.statements.push(ir::Stmt::Expr(ExprId(3)));
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
    }

    #[test]
    fn struct_initializer_evaluates_source_order_then_permutates_snapshots() {
        let ty = nominal(vec![Ty::I32, Ty::I32]);
        let call = |name: &str| {
            expr(
                Ty::I32,
                ir::ExprKind::Call {
                    name: name.into(),
                    args: vec![],
                    signature: ir::Signature {
                        params: vec![],
                        result: Ty::I32,
                    },
                },
            )
        };
        let source = with_root(
            Ty::I32,
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
    fn validator_rejects_non_copy_operands_even_in_disconnected_blocks() {
        let root = Ty::Struct(nominal(vec![Ty::I32]));
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
            .flat_map(|b| &b.statements)
            .filter(|s| {
                matches!(
                    s.value,
                    Statement::Assign {
                        value: Rvalue::Discriminant(_),
                        ..
                    }
                )
            })
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
        assert!(matches!(reads[0], Operand::Copy(p) if p.fields == [0, 0]));
        assert!(matches!(reads[1], Operand::Copy(p) if p.fields == [0, 0]));
        assert!(matches!(reads[2], Operand::Move(p) if p.fields == [0]));
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
        assert!(matches!(reads[0], Operand::Move(p) if p.fields.is_empty()));
    }

    #[test]
    fn partial_move_pattern_reads_only_selected_sibling_in_source_order() {
        let source =
            "struct S{x:i32} fn f(p:(S,S)){let first=p.0;let(_,second)=p;let(_,_) =p;} fn main(){}";
        let function = nominal_lowered(&checked(source, 0));
        assert!(super::super::ownership::analyze(&function).passed());
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 2);
        assert!(matches!(reads[0], Operand::Move(p) if p.fields == [0]));
        assert!(matches!(reads[1], Operand::Move(p) if p.fields == [1]));

        let source = "struct S{x:i32} struct P{first:S,second:S} fn f(p:P){let P{second:b,first:a}=p;} fn main(){}";
        let function = nominal_lowered(&checked(source, 0));
        assert!(super::super::ownership::analyze(&function).passed());
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 2);
        assert!(matches!(reads[0], Operand::Move(p) if p.fields == [1]));
        assert!(matches!(reads[1], Operand::Move(p) if p.fields == [0]));
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
                matches!(&statement.value, Statement::Assign {value: Rvalue::Use(Operand::Move(p)), ..} if p.local == local && p.fields.is_empty())
            }).count(), 1);
        }
    }

    #[test]
    fn unit_pattern_is_not_a_read_but_unit_binding_is() {
        let function = nominal_lowered(&checked("fn f(u:()){let ()=u;let real=u;} fn main(){}", 0));
        assert!(super::super::ownership::analyze(&function).passed());
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 1);
        assert!(matches!(reads[0], Operand::Copy(p) if p.fields.is_empty()));
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
            Ty::I32,
            vec![expr(
                Ty::I32,
                ir::ExprKind::Field {
                    base: ExprId(0),
                    index: 0,
                },
            )],
            vec![],
        );
        source.body.statements.push(ir::Stmt::LetPattern {
            value: ExprId(0),
            ty: Ty::I32,
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
                            Rvalue::Use(value) | Rvalue::Unary { value, .. } => {
                                live_operand(value, &live)
                            }
                            Rvalue::Binary { left, right, .. } => {
                                live_operand(left, &live);
                                live_operand(right, &live);
                            }
                            Rvalue::Aggregate { fields, .. } => {
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
