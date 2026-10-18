//! Per-function control-flow lowering of immutable checked bodies.
//!
//! Explicit value snapshots and edges preserve source evaluation order. This
//! module selects Copy/Move operands from projected semantic types. Ownership
//! dataflow is established separately by the analyzer before native emission.
use super::frontend::{BinaryOp, UnaryOp};
use super::ir::{self, ExprId, IntTy, IntValue, LocalId, LoopId, Place, Projection, Ty};
mod drop_normal;
pub use drop_normal::ValidatedCleanupCall;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoanId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoanTiming {
    #[default]
    Immediate,
    TwoPhase {
        activation: BlockId,
        argument: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoanSite {
    pub kind: ir::BorrowKind,
    pub place: Place,
    pub origin: Option<ExprId>,
    pub timing: LoanTiming,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located<T> {
    pub value: T,
    pub origin: Option<ExprId>,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub lowering_proof: Option<std::sync::Arc<GeneratedCfgShape>>,
    pub body_substitution: Option<std::sync::Arc<ir::BodySubstitutionProof>>,
    pub call_evidence: std::sync::Arc<[std::sync::Arc<ir::ValidatedCallEvidence>]>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BasicBlock {
    pub statements: Vec<Located<Statement>>,
    pub terminator: Located<Terminator>,
}

/// Payload correspondence with the one canonical lowering. Identity guards retain
/// their original allocations, never the produced Function. Body proofs retain
/// only the original verified template, so these guards have no output backedge.
#[derive(Clone, Debug)]
pub struct GeneratedCfgShape {
    name: String,
    params: std::sync::Arc<[LocalId]>,
    locals: std::sync::Arc<[ir::LocalDecl]>,
    blocks: std::sync::Arc<[BasicBlock]>,
    loans: std::sync::Arc<[LoanSite]>,
    result: Ty,
    contract: std::sync::Arc<ir::RefContract>,
    entry: BlockId,
    return_local: LocalId,
    world: std::sync::Arc<super::traits::ValidatedTraitWorld>,
    declaration: ir::DeclId,
    phase: super::traits::FactsPhase,
    target: Option<ir::CallTarget>,
    capsules: std::sync::Arc<[std::sync::Arc<ir::ValidatedCallEvidence>]>,
    body_proof: Option<std::sync::Arc<ir::BodySubstitutionProof>>,
    original_publication: Option<std::sync::Arc<super::source_replay::CanonicalSourcePublication>>,
    budget: ir::PlanBudget,
}
impl GeneratedCfgShape {
    fn capture(
        f: &Function,
        facts: &super::traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        Self::capture_with_publication(f, facts, None, budget)
    }
    fn capture_with_publication(
        f: &Function,
        facts: &super::traits::ValidatedFacts,
        publication: Option<&std::sync::Arc<super::source_replay::CanonicalSourcePublication>>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        f.preflight_plan(ir::FactsCtx::from_validated(facts), budget)?;
        budget.retain_bytes(f.snapshot_storage(budget)?)?;
        budget.retain_bytes(std::mem::size_of::<Self>() + 8 * std::mem::size_of::<usize>())?;
        budget.retain_bytes(
            f.call_evidence
                .len()
                .checked_mul(2 * std::mem::size_of::<std::sync::Arc<ir::ValidatedCallEvidence>>())
                .ok_or("lowering capsule storage overflow")?,
        )?;
        budget.charge_work(f.call_evidence.len())?;
        Ok(Self {
            name: f.name.clone(),
            params: f.params.clone().into(),
            locals: f.locals.clone().into(),
            blocks: f.blocks.clone().into(),
            loans: f.loan_sites.clone().into(),
            result: f.result.clone(),
            contract: f.ref_contract.clone(),
            entry: f.entry,
            return_local: f.return_local,
            body_proof: f.body_substitution.clone(),
            original_publication: publication.cloned(),
            budget: budget.clone(),
            world: facts.world().clone(),
            declaration: facts.declaration(),
            phase: facts.phase(),
            target: facts.target().cloned(),
            capsules: f.call_evidence.clone(),
        })
    }
    pub(super) fn validate(
        &self,
        f: &Function,
        facts: &super::traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        f.preflight_plan(ir::FactsCtx::from_validated(facts), budget)?;
        f.snapshot_storage(budget)?;
        if let Some(publication) = &self.original_publication {
            ir::validate_published_source(publication, budget)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
            publication
                .validate_source(publication.source(), facts, budget)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
            if f.body_substitution.is_some() {
                return Err("original source CFG cannot carry a generated body proof".into());
            }
            for capsule in f.call_evidence.iter() {
                budget.charge_work(1).map_err(|error| {
                    super::traits::FactsError::resource(budget, &error)
                        .into_legacy_diagnostic(budget)
                })?;
                if !std::sync::Arc::ptr_eq(capsule.source(), publication.source())
                    || !capsule.original().is_some_and(|original| {
                        std::sync::Arc::ptr_eq(original, publication.replay())
                    })
                {
                    return Err("source CFG capsule changed canonical publication".into());
                }
            }
        }
        if !self.budget.same_authority(budget)
            || !std::sync::Arc::ptr_eq(&self.world, facts.world())
            || self.declaration != facts.declaration()
            || self.phase != facts.phase()
            || !facts.budget().same_authority(budget)
        {
            return Err("lowering proof facts authority changed".into());
        }
        match (&self.target, facts.target()) {
            (Some(a), Some(b))
                if a.declaration == b.declaration
                    && facts
                        .world()
                        .types_equal(&a.type_args, &b.type_args)
                        .map_err(|e| e.into_legacy_diagnostic(budget))? => {}
            (None, None) => {}
            _ => return Err("lowering proof target changed".into()),
        }
        budget.charge_work(f.call_evidence.len().saturating_add(self.capsules.len()))?;
        match (&self.body_proof, &f.body_substitution) {
            (Some(original), Some(current)) if std::sync::Arc::ptr_eq(original, current) => {}
            (None, None) => {}
            _ => return Err("lowering proof exact body correspondence changed".into()),
        }
        if f.call_evidence.len() != self.capsules.len()
            || f.call_evidence
                .iter()
                .zip(self.capsules.iter())
                .any(|(capsule, original)| !std::sync::Arc::ptr_eq(capsule, original))
        {
            return Err("lowering proof original evidence changed".into());
        }
        budget.charge_work(
            self.name
                .len()
                .saturating_add(self.params.len())
                .saturating_add(self.locals.len())
                .saturating_add(self.blocks.len())
                .saturating_add(self.loans.len()),
        )?;
        if f.name != self.name
            || f.params.as_slice() != self.params.as_ref()
            || f.locals.len() != self.locals.len()
            || f.blocks.len() != self.blocks.len()
            || f.loan_sites.len() != self.loans.len()
            || f.entry != self.entry
            || f.return_local != self.return_local
            || !std::sync::Arc::ptr_eq(&f.ref_contract, &self.contract)
            || !ir::identical_type(&f.result, &self.result, budget)?
        {
            return Err("generated CFG subject changed".into());
        }
        for (a, b) in f.locals.iter().zip(self.locals.iter()) {
            if !ir::identical_type(&a.ty, &b.ty, budget)? {
                return Err("generated CFG local type changed".into());
            }
        }
        for (a, b) in f.blocks.iter().zip(self.blocks.iter()) {
            budget.charge_work(a.statements.len().saturating_add(b.statements.len()))?;
            if a.statements.len() != b.statements.len() {
                return Err("generated CFG statement count changed".into());
            }
            for (a, b) in a.statements.iter().zip(&b.statements) {
                if !same_statement_types(&a.value, &b.value, budget)? {
                    return Err("generated CFG statement metadata changed".into());
                }
            }
            if !same_terminator_types(&a.terminator.value, &b.terminator.value, budget)? {
                return Err("generated CFG call metadata changed".into());
            }
        }
        if f.locals.as_slice() != self.locals.as_ref()
            || f.blocks.as_slice() != self.blocks.as_ref()
            || f.loan_sites.as_slice() != self.loans.as_ref()
        {
            return Err("generated CFG operation, flow, origin or loan timing changed".into());
        }
        Ok(())
    }
}
fn same_operand_types(a: &Operand, b: &Operand, budget: &ir::PlanBudget) -> Result<bool, String> {
    budget.charge_work(1)?;
    if let (
        Operand::Constant(Constant::Enum { ty: a, .. }),
        Operand::Constant(Constant::Enum { ty: b, .. }),
    ) = (a, b)
    {
        return ir::identical_type(&Ty::Enum(a.clone()), &Ty::Enum(b.clone()), budget);
    }
    Ok(true)
}
fn same_operand_list(
    a: &[Operand],
    b: &[Operand],
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    budget.charge_work(a.len().saturating_add(b.len()))?;
    if a.len() != b.len() {
        return Ok(false);
    }
    for (a, b) in a.iter().zip(b) {
        if !same_operand_types(a, b, budget)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn same_rvalue_types(a: &Rvalue, b: &Rvalue, budget: &ir::PlanBudget) -> Result<bool, String> {
    budget.charge_work(1)?;
    if std::mem::discriminant(a) != std::mem::discriminant(b) {
        return Ok(false);
    }
    match (a, b) {
        (Rvalue::Repeat { ty: a, value: aa }, Rvalue::Repeat { ty: b, value: bb }) => {
            Ok(ir::identical_type(a, b, budget)? && same_operand_types(aa, bb, budget)?)
        }
        (Rvalue::Aggregate { ty: a, fields: aa }, Rvalue::Aggregate { ty: b, fields: bb }) => {
            Ok(ir::identical_type(a, b, budget)? && same_operand_list(aa, bb, budget)?)
        }
        (
            Rvalue::EnumVariant {
                ty: a, fields: aa, ..
            },
            Rvalue::EnumVariant {
                ty: b, fields: bb, ..
            },
        ) => Ok(
            ir::identical_type(&Ty::Enum(a.clone()), &Ty::Enum(b.clone()), budget)?
                && same_operand_list(aa, bb, budget)?,
        ),
        (Rvalue::Use(a), Rvalue::Use(b))
        | (Rvalue::ArrayToSlice { reference: a }, Rvalue::ArrayToSlice { reference: b })
        | (Rvalue::SliceLen { slice: a }, Rvalue::SliceLen { slice: b })
        | (Rvalue::LeadingZeros { value: a }, Rvalue::LeadingZeros { value: b })
        | (Rvalue::IntCast { value: a, .. }, Rvalue::IntCast { value: b, .. })
        | (Rvalue::FloatCast { value: a, .. }, Rvalue::FloatCast { value: b, .. })
        | (Rvalue::Unary { value: a, .. }, Rvalue::Unary { value: b, .. })
        | (Rvalue::IndexCopy { index: a, .. }, Rvalue::IndexCopy { index: b, .. }) => {
            same_operand_types(a, b, budget)
        }
        (
            Rvalue::Binary {
                left: a, right: aa, ..
            },
            Rvalue::Binary {
                left: b, right: bb, ..
            },
        )
        | (
            Rvalue::SliceIndex {
                slice: a,
                index: aa,
            },
            Rvalue::SliceIndex {
                slice: b,
                index: bb,
            },
        ) => Ok(same_operand_types(a, b, budget)? && same_operand_types(aa, bb, budget)?),
        _ => Ok(true),
    }
}
fn same_statement_types(
    a: &Statement,
    b: &Statement,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    budget.charge_work(1)?;
    if std::mem::discriminant(a) != std::mem::discriminant(b) {
        return Ok(false);
    }
    match (a, b) {
        (Statement::Assign { value: a, .. }, Statement::Assign { value: b, .. }) => {
            same_rvalue_types(a, b, budget)
        }
        (
            Statement::IndexAssign {
                index: a,
                value: aa,
                ..
            },
            Statement::IndexAssign {
                index: b,
                value: bb,
                ..
            },
        )
        | (
            Statement::IndexAssignOp {
                index: a,
                value: aa,
                ..
            },
            Statement::IndexAssignOp {
                index: b,
                value: bb,
                ..
            },
        ) => Ok(same_operand_types(a, b, budget)? && same_operand_types(aa, bb, budget)?),
        (Statement::IndexInspect { index: a, .. }, Statement::IndexInspect { index: b, .. }) => {
            same_operand_types(a, b, budget)
        }
        _ => Ok(true),
    }
}
fn same_terminator_types(
    a: &Terminator,
    b: &Terminator,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    budget.charge_work(1)?;
    if std::mem::discriminant(a) != std::mem::discriminant(b) {
        return Ok(false);
    }
    match (a, b) {
        (
            Terminator::Call {
                target: a,
                signature: sa,
                args: aa,
                ..
            },
            Terminator::Call {
                target: b,
                signature: sb,
                args: bb,
                ..
            },
        ) => {
            Ok(ir::identical_call_types(a, sa, b, sb, budget)?
                && same_operand_list(aa, bb, budget)?)
        }
        (Terminator::Switch { condition: a, .. }, Terminator::Switch { condition: b, .. }) => {
            same_operand_types(a, b, budget)
        }
        _ => Ok(true),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Constant {
    Enum { ty: ir::EnumTy, variant: usize },
    Int(IntValue),
    Char(ir::CharValue),
    Float(ir::FloatValue),
    Bool(bool),
    Unit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    Constant(Constant),
    Copy(Place),
    Move(Place),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rvalue {
    /// Count leading zero bits in an integer operand, producing u32.
    LeadingZeros {
        value: Operand,
    },
    IntCast {
        value: Operand,
        to: IntTy,
    },
    /// Numeric `as` with at least one float endpoint. Float-to-int saturates
    /// and maps NaN to zero, int/float-to-float rounds to nearest-even.
    FloatCast {
        value: Operand,
        to: ir::NumTy,
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
    Output(ir::LiteralOutput),
    Println(String),
    /// Unsizes exactly shared `&[u8; N]` to shared `&[u8]`. N comes from the
    /// operand's validated type. The operand is evaluated once and its loan
    /// and origin set is the only storage authority of the result.
    ArrayToSlice {
        reference: Operand,
    },
    /// Builtin length of exactly shared `&[u8]`, result `usize`. Grants no
    /// storage authority and performs no read through the reference.
    SliceLen {
        slice: Operand,
    },
    /// Runtime bounds-checked `u8` copy through exactly shared `&[u8]` at a
    /// `usize` index. Requires a live shared-origin read. No slice place or
    /// mutable index exists.
    SliceIndex {
        slice: Operand,
        index: Operand,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminator {
    /// Runtime-guarded, canonically authorized exclusive destructor invocation.
    /// Storage stays live; ownership ends on the normal continuation.
    Drop {
        cleanup: std::sync::Arc<ValidatedCleanupCall>,
        next: BlockId,
    },
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
        target: ir::Callee,
        name: String,
        signature: ir::Signature,
        args: Vec<Operand>,
        destination: Place,
        next: BlockId,
    },
    /// Consumes the value held in Function::return_local.
    Return,
    Panic(Box<ir::PanicMessage>),
    Unreachable,
}

impl Rvalue {
    pub(super) fn visit_operands(
        &self,
        visit: &mut impl FnMut(&Operand) -> Result<(), String>,
    ) -> Result<(), String> {
        match self {
            Self::Use(value)
            | Self::LeadingZeros { value }
            | Self::IntCast { value, .. }
            | Self::FloatCast { value, .. }
            | Self::Unary { value, .. }
            | Self::Repeat { value, .. }
            | Self::ArrayToSlice { reference: value }
            | Self::SliceLen { slice: value } => visit(value)?,
            Self::SliceIndex { slice, index } => {
                visit(slice)?;
                visit(index)?;
            }
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
            Self::Borrow(_) | Self::Discriminant(_) | Self::Println(_) | Self::Output(_) => {}
        }
        Ok(())
    }
}
impl Statement {
    pub(super) fn visit_operands(
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
    pub(super) fn visit_operands(
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
            Self::Drop { .. }
            | Self::SwitchVariant { .. }
            | Self::Goto(_)
            | Self::Return
            | Self::Panic(_)
            | Self::Unreachable => {}
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
    facts: std::sync::Arc<super::traits::ValidatedFacts>,
    function: std::sync::Arc<Function>,
    budget: ir::PlanBudget,
}
impl TemplateCfg {
    pub fn call_evidence(&self) -> &[std::sync::Arc<ir::ValidatedCallEvidence>] {
        &self.function.call_evidence
    }
    pub fn facts(&self) -> &std::sync::Arc<super::traits::ValidatedFacts> {
        &self.facts
    }
    pub fn source(&self) -> &std::sync::Arc<ir::CheckedFunction> {
        &self.source
    }
    pub fn env(&self) -> &std::sync::Arc<ir::GenericEnv> {
        self.facts.env()
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
    facts: std::sync::Arc<super::traits::ValidatedFacts>,
    budget: ir::PlanBudget,
}
impl ClosedCfg {
    pub fn body_substitution(&self) -> Option<&std::sync::Arc<ir::BodySubstitutionProof>> {
        self.function.body_substitution.as_ref()
    }
    pub fn call_evidence(&self) -> &[std::sync::Arc<ir::ValidatedCallEvidence>] {
        &self.function.call_evidence
    }
    pub fn facts(&self) -> &std::sync::Arc<super::traits::ValidatedFacts> {
        &self.facts
    }
    pub fn function(&self) -> &std::sync::Arc<Function> {
        &self.function
    }
    pub fn budget(&self) -> &ir::PlanBudget {
        &self.budget
    }
}
fn require_facts(
    facts: &super::traits::ValidatedFacts,
    phase: super::traits::FactsPhase,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    if facts.phase() != phase || !facts.budget().same_authority(budget) {
        return Err("CFG facts phase or budget authority mismatch".into());
    }
    Ok(())
}
pub fn close(
    function: std::sync::Arc<Function>,
    facts: std::sync::Arc<super::traits::ValidatedFacts>,
    budget: &ir::PlanBudget,
) -> Result<ClosedCfg, String> {
    require_facts(&facts, super::traits::FactsPhase::Closed, budget)?;
    function.validate_provenance(&facts, budget)?;
    function.preflight_plan(ir::FactsCtx::from_validated(&facts), budget)?;
    function.validate_with_facts(ir::FactsCtx::from_validated(&facts))?;
    Ok(ClosedCfg {
        function,
        facts,
        budget: budget.clone(),
    })
}
pub fn lower_template(
    source: std::sync::Arc<ir::CheckedFunction>,
    authority: std::sync::Arc<super::traits::ValidatedFacts>,
    budget: &ir::PlanBudget,
) -> Result<TemplateCfg, String> {
    require_facts(&authority, super::traits::FactsPhase::Universal, budget)?;
    budget.charge_nodes(source.exprs.len().saturating_add(source.locals.len()))?;
    let facts = ir::FactsCtx::from_validated(&authority);
    let mut lowered = lower_internal(&source, facts, Some(budget))?;
    lowered.call_evidence = ir::bind_call_evidence(source.clone(), authority.clone(), budget)?;
    if !lowered.call_evidence.is_empty() {
        lowered.lowering_proof = Some(std::sync::Arc::new(GeneratedCfgShape::capture(
            &lowered, &authority, budget,
        )?));
    }
    let function = std::sync::Arc::new(lowered);
    function.preflight_plan(facts, budget)?;
    function.validate_with_facts(facts)?;
    Ok(TemplateCfg {
        source,
        facts: authority,
        function,
        budget: budget.clone(),
    })
}

/// Canonical Source lowering always retains its publication, including bodies
/// with no calls. Recipe/raw typed IR cannot substitute for this association.
pub(super) fn lower_source_template(
    publication: &std::sync::Arc<super::source_replay::CanonicalSourcePublication>,
    budget: &ir::PlanBudget,
) -> Result<TemplateCfg, super::traits::FactsError> {
    use super::traits::FactsError;
    let authority = publication.facts();
    require_facts(authority, super::traits::FactsPhase::Universal, budget)
        .map_err(|error| FactsError::from_message(budget, &error))?;
    let nodes = publication
        .source()
        .exprs
        .len()
        .checked_add(publication.source().locals.len())
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| FactsError::resource(budget, "source template node overflow"))?;
    budget
        .charge_nodes(nodes)
        .map_err(|error| FactsError::resource(budget, &error))?;
    let lowered = lower_source(publication, budget)?;
    let header = 2usize
        .checked_mul(std::mem::size_of::<usize>())
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Function>()))
        .ok_or_else(|| FactsError::resource(budget, "source template storage overflow"))?;
    budget
        .retain_bytes(header)
        .map_err(|error| FactsError::resource(budget, &error))?;
    Ok(TemplateCfg {
        source: publication.source().clone(),
        facts: authority.clone(),
        function: std::sync::Arc::new(lowered),
        budget: budget.clone(),
    })
}

pub(super) fn lower_source_closed(
    publication: &std::sync::Arc<super::source_replay::CanonicalSourcePublication>,
    budget: &ir::PlanBudget,
) -> Result<Function, super::traits::FactsError> {
    require_facts(
        publication.facts(),
        super::traits::FactsPhase::Closed,
        budget,
    )
    .map_err(|error| super::traits::FactsError::from_message(budget, &error))?;
    lower_source(publication, budget)
}

fn lower_source(
    publication: &std::sync::Arc<super::source_replay::CanonicalSourcePublication>,
    budget: &ir::PlanBudget,
) -> Result<Function, super::traits::FactsError> {
    use super::traits::FactsError;
    let source = publication.source();
    let authority = publication.facts();
    publication.validate_source(source, authority, budget)?;
    let facts = ir::FactsCtx::from_validated(authority);
    let mut function = lower_internal_with_source(
        source,
        facts,
        Some(budget),
        SourceLowering::Original(publication),
    )
    .map_err(|error| FactsError::from_message(budget, &error))?;
    function.call_evidence = ir::validated_call_capsules(publication, budget)?;
    budget
        .charge_nodes(1)
        .map_err(|error| FactsError::resource(budget, &error))?;
    function.lowering_proof = Some(std::sync::Arc::new(
        GeneratedCfgShape::capture_with_publication(
            &function,
            authority,
            Some(publication),
            budget,
        )
        .map_err(|error| FactsError::from_message(budget, &error))?,
    ));
    function
        .preflight_plan(facts, budget)
        .map_err(|error| FactsError::from_message(budget, &error))?;
    function
        .validate_with_facts(facts)
        .map_err(|error| FactsError::from_message(budget, &error))?;
    function
        .validate_provenance(authority, budget)
        .map_err(|error| FactsError::from_message(budget, &error))?;
    Ok(function)
}

fn concrete_carrier_count(ty: &Ty, facts: ir::FactsCtx<'_>) -> Result<usize, String> {
    if let Some(authority) = facts.validated() {
        Ok(ir::carrier_locations(ty, authority.budget())?
            .iter()
            .filter(|location| matches!(location.channel, ir::CarrierChannel::Fixed(_)))
            .count())
    } else {
        ir::reference_layers(ty)
    }
}

fn validate_type(ty: &Ty, depth: usize) -> Result<(), String> {
    let mut pending = vec![(ty, depth)];
    let mut work = 0usize;
    let mut unresolved_associated_type = false;
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
            Ty::AssociatedType(projection) => {
                unresolved_associated_type = true;
                pending.push((&projection.trait_ref.self_ty, depth + 1));
                pending.extend(projection.trait_ref.args.iter().map(|ty| (ty, depth + 1)));
            }
            Ty::Ref { referent, .. } => {
                if depth != 0 {
                    return Err("unsupported: nested or aggregate references".into());
                }
                let layers = ir::reference_layers(ty)?;
                let mut inner = referent.as_ref();
                for _ in 1..layers {
                    let Ty::Ref { referent, .. } = inner else {
                        unreachable!()
                    };
                    inner = referent;
                    work += 1;
                }
                pending.push((inner, depth + layers));
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
                // A zero-field nominal remains an owned, non-Copy nominal.
                // Source shape admission belongs to the checked source schema,
                // not the field count of this runtime carrier.
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
    if unresolved_associated_type {
        return Err("unsupported: unresolved associated type in executable CFG".into());
    }
    Ok(())
}

impl Function {
    /// Same exact payload sizing walk for snapshot construction and comparison.
    /// Only construction retains bytes. Type allocations are shared immutable
    /// Arcs and are validated separately by the authoritative preflight.
    fn snapshot_storage(&self, budget: &ir::PlanBudget) -> Result<usize, String> {
        fn add(
            bytes: &mut usize,
            n: usize,
            width: usize,
            budget: &ir::PlanBudget,
        ) -> Result<(), String> {
            budget.charge_work(n)?;
            *bytes = bytes
                .checked_add(n.checked_mul(width).ok_or("CFG snapshot size overflow")?)
                .ok_or("CFG snapshot size overflow")?;
            Ok(())
        }
        fn path(bytes: &mut usize, p: &Place, budget: &ir::PlanBudget) -> Result<(), String> {
            budget.check_depth(p.projection.len())?;
            add(
                bytes,
                p.projection.len(),
                std::mem::size_of::<Projection>(),
                budget,
            )
        }
        fn operand(bytes: &mut usize, o: &Operand, budget: &ir::PlanBudget) -> Result<(), String> {
            budget.charge_work(1)?;
            if let Operand::Copy(p) | Operand::Move(p) = o {
                path(bytes, p, budget)?;
            }
            Ok(())
        }
        let mut bytes = 0;
        add(&mut bytes, self.name.len(), 1, budget)?;
        add(
            &mut bytes,
            self.params.len(),
            2 * std::mem::size_of::<LocalId>(),
            budget,
        )?;
        add(
            &mut bytes,
            self.locals.len(),
            2 * std::mem::size_of::<ir::LocalDecl>(),
            budget,
        )?;
        add(
            &mut bytes,
            self.blocks.len(),
            2 * std::mem::size_of::<BasicBlock>(),
            budget,
        )?;
        add(
            &mut bytes,
            self.loan_sites.len(),
            2 * std::mem::size_of::<LoanSite>(),
            budget,
        )?;
        for local in &self.locals {
            add(&mut bytes, local.name.len(), 1, budget)?;
        }
        for loan in &self.loan_sites {
            path(&mut bytes, &loan.place, budget)?;
        }
        for block in &self.blocks {
            add(
                &mut bytes,
                block.statements.len(),
                std::mem::size_of::<Located<Statement>>(),
                budget,
            )?;
            for statement in &block.statements {
                match &statement.value {
                    Statement::Assign { destination, value } => {
                        path(&mut bytes, destination, budget)?;
                        match value {
                            Rvalue::Aggregate { fields, .. }
                            | Rvalue::EnumVariant { fields, .. } => add(
                                &mut bytes,
                                fields.len(),
                                std::mem::size_of::<Operand>(),
                                budget,
                            )?,
                            Rvalue::Output(output) => {
                                add(&mut bytes, output.text.len(), 1, budget)?
                            }
                            Rvalue::Println(text) => add(&mut bytes, text.len(), 1, budget)?,
                            Rvalue::IndexCopy { array, .. } | Rvalue::Discriminant(array) => {
                                path(&mut bytes, array, budget)?
                            }
                            _ => {}
                        }
                    }
                    Statement::IndexAssignOp { array, .. }
                    | Statement::IndexAssign { array, .. }
                    | Statement::IndexInspect { array, .. }
                    | Statement::Inspect(array) => path(&mut bytes, array, budget)?,
                    Statement::StorageLive(_) | Statement::StorageDead(_) => {}
                }
                statement
                    .value
                    .visit_operands(&mut |value| operand(&mut bytes, value, budget))?;
            }
            match &block.terminator.value {
                Terminator::Call {
                    name,
                    signature,
                    args,
                    destination,
                    ..
                } => {
                    add(&mut bytes, name.len(), 1, budget)?;
                    add(
                        &mut bytes,
                        signature.params.len(),
                        std::mem::size_of::<Ty>(),
                        budget,
                    )?;
                    add(
                        &mut bytes,
                        args.len(),
                        std::mem::size_of::<Operand>(),
                        budget,
                    )?;
                    path(&mut bytes, destination, budget)?;
                }
                Terminator::Panic(payload) => {
                    add(
                        &mut bytes,
                        1,
                        std::mem::size_of::<ir::PanicMessage>(),
                        budget,
                    )?;
                    add(&mut bytes, payload.message.len(), 1, budget)?;
                    add(&mut bytes, payload.location.len(), 1, budget)?;
                }
                Terminator::Drop { cleanup, .. } => {
                    path(&mut bytes, cleanup.owner(), budget)?;
                    add(
                        &mut bytes,
                        1,
                        std::mem::size_of::<ValidatedCleanupCall>(),
                        budget,
                    )?;
                }
                Terminator::SwitchVariant { place, .. } => path(&mut bytes, place, budget)?,
                _ => {}
            }
            block
                .terminator
                .value
                .visit_operands(&mut |value| operand(&mut bytes, value, budget))?;
        }
        Ok(bytes)
    }
    /// Native Source graph boundary. A legacy/raw capsule, including a body with
    /// no calls, cannot stand in for the required canonical publication.
    pub(super) fn validate_source_publication(
        &self,
        publication: &std::sync::Arc<super::source_replay::CanonicalSourcePublication>,
        budget: &ir::PlanBudget,
    ) -> Result<(), super::traits::FactsError> {
        use super::traits::FactsError;
        let shape = self.lowering_proof.as_ref().ok_or_else(|| {
            FactsError::invalid(budget, "source CFG has no canonical publication shape")
        })?;
        if !shape
            .original_publication
            .as_ref()
            .is_some_and(|original| std::sync::Arc::ptr_eq(original, publication))
        {
            return Err(FactsError::invalid(
                budget,
                "source CFG publication association changed",
            ));
        }
        self.validate_provenance(publication.facts(), budget)
            .map_err(|error| FactsError::from_message(budget, &error))
    }

    pub fn validate_provenance(
        &self,
        facts: &super::traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        if !facts.budget().same_authority(budget) {
            return Err("CFG provenance changed planning authority".into());
        }
        if let Some(target) = facts.target() {
            budget.charge_work(target.type_args.len())?;
        }
        if facts.target().is_some_and(|target| {
            target
                .type_args
                .iter()
                .any(|ty| matches!(ty, Ty::Ref { .. }))
        }) && self.body_substitution.is_none()
        {
            return Err(
                "reference-valued instance requires its exact verified-template body proof".into(),
            );
        }
        if self.lowering_proof.is_none() {
            for local in &self.locals {
                if drop_normal::descriptor(&local.ty, ir::FactsCtx::from_validated(facts))?
                    .is_some()
                {
                    return Err("owning Drop CFG requires exact canonical lowering proof".into());
                }
            }
        }
        if let Some(shape) = &self.lowering_proof {
            shape.validate(self, facts, budget)?;
        } else if self.body_substitution.is_some() || !self.call_evidence.is_empty() {
            return Err("CFG provenance requires its exact canonical lowering snapshot".into());
        }
        if let Some(proof) = &self.body_substitution {
            proof.validate_cfg(self, facts, budget)?;
        }
        for block in &self.blocks {
            budget.charge_work(self.call_evidence.len().saturating_add(1))?;
            let Terminator::Call { target, args, .. } = &block.terminator.value else {
                continue;
            };
            let capsule = self
                .call_evidence
                .iter()
                .find(|evidence| Some(evidence.call()) == block.terminator.origin);
            let ordinary_actuals: &[Ty] = match target {
                ir::Callee::Direct(target) => &target.type_args,
                ir::Callee::TraitItem(call) => &call.method_args,
            };
            budget.charge_work(ordinary_actuals.len())?;
            let requires = ordinary_actuals
                .iter()
                .any(|ty| matches!(ty, Ty::Ref { .. }));
            let Some(capsule) = capsule else {
                if requires {
                    return Err("reference actual CFG call lacks original checked evidence".into());
                }
                continue;
            };
            if !std::sync::Arc::ptr_eq(capsule.facts().world(), facts.world())
                || !capsule.facts().budget().same_authority(budget)
            {
                return Err("CFG call evidence has foreign world or ledger".into());
            }
            if let Some(proof) = &self.body_substitution {
                if !std::sync::Arc::ptr_eq(capsule.source(), proof.template().source())
                    || capsule.facts().declaration() != proof.template().facts().declaration()
                    || capsule.facts().phase() != super::traits::FactsPhase::Universal
                {
                    return Err(
                        "CFG call evidence is not the exact verified template source".into(),
                    );
                }
            } else if capsule.facts().declaration() != facts.declaration()
                || capsule.facts().phase() != facts.phase()
            {
                return Err("CFG call evidence subject or phase mismatch".into());
            }
            let ir::ExprKind::Call {
                args: source_args, ..
            } = &capsule
                .source()
                .exprs
                .get(capsule.call().0)
                .ok_or("CFG call source outside checked arena")?
                .kind
            else {
                return Err("CFG evidence source is not a call".into());
            };
            if args.len() != source_args.len() {
                return Err("CFG call argument provenance arity mismatch".into());
            }
            for (argument, source_arg) in args.iter().zip(source_args) {
                budget.charge_work(1)?;
                let expected = capsule
                    .source()
                    .locals
                    .len()
                    .checked_add(source_arg.0)
                    .ok_or("CFG source argument index overflow")?;
                if !matches!(argument, Operand::Copy(place) | Operand::Move(place) if place.local.0 == expected && place.projection.is_empty())
                {
                    return Err(
                        "CFG call no longer uses its exact checked argument snapshot".into(),
                    );
                }
            }
        }
        Ok(())
    }
    fn validate_loan_timing(&self, facts: ir::FactsCtx<'_>) -> Result<(), String> {
        let mut work = 0usize;
        let mut spend = |amount: usize| -> Result<(), String> {
            work = work
                .checked_add(amount)
                .ok_or("two-phase validation work overflow")?;
            if work > ir::MAX_PATTERN_PROJECTION_WORK {
                return Err("two-phase validation work limit exceeded".into());
            }
            if let Some(authority) = facts.validated() {
                authority.budget().charge_work(amount)?;
            }
            Ok(())
        };
        for (loan_index, site) in self.loan_sites.iter().enumerate() {
            spend(1)?;
            let LoanTiming::TwoPhase {
                activation,
                argument,
            } = site.timing
            else {
                continue;
            };
            if site.kind != ir::BorrowKind::Mut {
                return Err("two-phase loan must be mutable".into());
            }
            let activation_block = self
                .blocks
                .get(activation.0)
                .ok_or("invalid two-phase activation block")?;
            let Terminator::Call {
                target,
                signature,
                args,
                ..
            } = &activation_block.terminator.value
            else {
                return Err("two-phase activation must be a call".into());
            };
            let Some(Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent,
            }) = signature.params.get(argument)
            else {
                return Err("two-phase activation requires mutable reference formal".into());
            };
            let site_ty = self.place_ty(&site.place)?;
            let agrees = if let Some(authority) = facts.validated() {
                authority
                    .world()
                    .types_equal(
                        std::slice::from_ref(site_ty),
                        std::slice::from_ref(referent.as_ref()),
                    )
                    .map_err(|error| error.into_legacy_diagnostic(authority.budget()))?
            } else {
                site_ty == referent.as_ref()
            };
            if !agrees {
                return Err("two-phase referent/formal mismatch".into());
            }
            let Some(Operand::Move(carrier)) = args.get(argument) else {
                return Err("two-phase activation must move its unique carrier".into());
            };
            if !carrier.projection.is_empty()
                || self.params.contains(&carrier.local)
                || carrier.local == self.return_local
            {
                return Err("two-phase carrier must be a dedicated temporary".into());
            }
            let receiver = if let Some(authority) = facts.validated() {
                argument == 0
                    && authority
                        .service()
                        .callee_receiver(target, authority)
                        .map_err(|e| e.into_legacy_diagnostic(authority.budget()))?
                        == (ir::ReceiverMode::Borrowed {
                            kind: ir::BorrowKind::Mut,
                        })
            } else {
                false
            };
            let reborrow = site.place.projection.as_slice() == [Projection::Deref]
                && matches!(
                    self.locals.get(site.place.local.0).map(|local| &local.ty),
                    Some(Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        ..
                    })
                );
            if !receiver && !reborrow {
                return Err(
                    "two-phase loan is neither a mutable receiver nor local mutable reborrow"
                        .into(),
                );
            }
            let mut issue = None;
            let mut uses = 0usize;
            for (block_index, block) in self.blocks.iter().enumerate() {
                spend(1)?;
                for statement in &block.statements {
                    spend(1)?;
                    let check_place = |place: &Place| -> Result<(), String> {
                        if place.local == carrier.local {
                            Err("two-phase carrier has an aliased or extra use".into())
                        } else {
                            Ok(())
                        }
                    };
                    let mut check_operand = |operand: &Operand| -> Result<(), String> {
                        spend(1)?;
                        match operand {
                            Operand::Copy(place) | Operand::Move(place) => check_place(place),
                            Operand::Constant(_) => Ok(()),
                        }
                    };
                    match &statement.value {
                        Statement::StorageLive(_) | Statement::StorageDead(_) => {}
                        Statement::Assign {
                            destination,
                            value: Rvalue::Borrow(id),
                        } if id.0 == loan_index => {
                            if destination != carrier
                                || issue.replace(BlockId(block_index)).is_some()
                            {
                                return Err(
                                    "two-phase loan must have one dedicated issuance".into()
                                );
                            }
                        }
                        Statement::Assign { destination, value } => {
                            check_place(destination)?;
                            value.visit_operands(&mut check_operand)?;
                            match value {
                                Rvalue::IndexCopy { array, .. } | Rvalue::Discriminant(array) => {
                                    check_place(array)?
                                }
                                _ => {}
                            }
                        }
                        Statement::Inspect(place) => check_place(place)?,
                        Statement::IndexInspect { array, index } => {
                            check_place(array)?;
                            check_operand(index)?;
                        }
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
                            check_place(array)?;
                            check_operand(index)?;
                            check_operand(value)?;
                        }
                    }
                }
                let check_place = |place: &Place| -> Result<(), String> {
                    if place.local == carrier.local {
                        Err("two-phase carrier escapes activation".into())
                    } else {
                        Ok(())
                    }
                };
                match &block.terminator.value {
                    Terminator::Call {
                        args, destination, ..
                    } => {
                        check_place(destination)?;
                        for (slot, operand) in args.iter().enumerate() {
                            spend(1)?;
                            if let Operand::Copy(place) | Operand::Move(place) = operand
                                && place.local == carrier.local
                            {
                                if block_index != activation.0
                                    || slot != argument
                                    || !matches!(operand, Operand::Move(_))
                                    || place != carrier
                                {
                                    return Err(
                                        "two-phase carrier used outside exact activation argument"
                                            .into(),
                                    );
                                }
                                uses += 1;
                            }
                        }
                    }
                    Terminator::Switch {
                        condition: Operand::Copy(place) | Operand::Move(place),
                        ..
                    }
                    | Terminator::SwitchVariant { place, .. } => check_place(place)?,
                    Terminator::Drop { cleanup, .. } => check_place(cleanup.owner())?,
                    Terminator::Goto(_)
                    | Terminator::Switch { .. }
                    | Terminator::Return
                    | Terminator::Panic(_)
                    | Terminator::Unreachable => {}
                }
            }
            for other in &self.loan_sites {
                spend(1)?;
                if other.place.local == carrier.local {
                    return Err("two-phase carrier borrowed or aliased".into());
                }
            }
            let issue = issue.ok_or("two-phase reservation has no issuance")?;
            if uses != 1 {
                return Err("two-phase carrier must have exactly one activation use".into());
            }
            let bytes = self
                .blocks
                .len()
                .checked_mul(4 * std::mem::size_of::<usize>() + 2)
                .ok_or("two-phase dominance storage overflow")?;
            let _reservation = facts
                .validated()
                .map(|authority| authority.budget().reserve_bytes(bytes))
                .transpose()?;
            let mut reachable = |excluded: Option<BlockId>| -> Result<bool, String> {
                let mut seen = vec![false; self.blocks.len()];
                let mut pending = Vec::new();
                pending
                    .try_reserve_exact(self.blocks.len())
                    .map_err(|_| "two-phase dominance allocation failed")?;
                if Some(self.entry) != excluded {
                    seen[self.entry.0] = true;
                    pending.push(self.entry);
                }
                while let Some(current) = pending.pop() {
                    spend(1)?;
                    if current == activation {
                        return Ok(true);
                    }
                    let mut push = |next: BlockId| {
                        if Some(next) != excluded && !seen[next.0] {
                            seen[next.0] = true;
                            pending.push(next);
                        }
                    };
                    match self.blocks[current.0].terminator.value {
                        Terminator::Goto(next)
                        | Terminator::Call { next, .. }
                        | Terminator::Drop { next, .. } => push(next),
                        Terminator::Switch {
                            then_block,
                            else_block,
                            ..
                        }
                        | Terminator::SwitchVariant {
                            then_block,
                            else_block,
                            ..
                        } => {
                            push(then_block);
                            push(else_block);
                        }
                        Terminator::Return | Terminator::Panic(_) | Terminator::Unreachable => {}
                    }
                }
                Ok(false)
            };
            if reachable(None)? && reachable(Some(issue))? {
                return Err("two-phase reservation does not dominate reachable activation".into());
            }
        }
        Ok(())
    }
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
                Operand::Constant(
                    Constant::Int(_)
                    | Constant::Char(_)
                    | Constant::Float(_)
                    | Constant::Bool(_)
                    | Constant::Unit,
                ) => {}
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
                            | Rvalue::LeadingZeros { value }
                            | Rvalue::IntCast { value, .. }
                            | Rvalue::FloatCast { value, .. }
                            | Rvalue::Unary { value, .. }
                            | Rvalue::ArrayToSlice { reference: value }
                            | Rvalue::SliceLen { slice: value } => operand(value, budget, facts)?,
                            Rvalue::SliceIndex { slice, index } => {
                                operand(slice, budget, facts)?;
                                operand(index, budget, facts)?;
                            }
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
                            Rvalue::Borrow(_) | Rvalue::Println(_) | Rvalue::Output(_) => {}
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
                    if target.type_arguments().len() > 32 {
                        return Err("generic call argument limit exceeded".into());
                    }
                    for ty in target
                        .metadata_types()
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
                Terminator::Drop { cleanup, .. } => {
                    path(cleanup.owner(), budget)?;
                    let authority = facts
                        .validated()
                        .ok_or("cleanup requires validated canonical facts")?;
                    cleanup.validate(self, authority)?;
                }
                Terminator::Switch { condition, .. } => operand(condition, budget, facts)?,
                Terminator::SwitchVariant { place, .. } => path(place, budget)?,
                Terminator::Goto(_)
                | Terminator::Return
                | Terminator::Panic(_)
                | Terminator::Unreachable => {}
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

    fn validate_operand_facts(
        &self,
        operand: &Operand,
        facts: ir::FactsCtx<'_>,
    ) -> Result<(), String> {
        let ty = self.operand_ty_with_facts(operand, facts)?;
        facts.validate_ty(&ty)
    }
    pub fn operand_ty(&self, operand: &Operand) -> Result<Ty, String> {
        self.operand_ty_with_facts(operand, ir::FactsCtx::Concrete)
    }
    fn operand_ty_with_facts(
        &self,
        operand: &Operand,
        facts: ir::FactsCtx<'_>,
    ) -> Result<Ty, String> {
        match operand {
            Operand::Constant(Constant::Int(value)) => {
                value.validate()?;
                Ok(Ty::Int(value.ty))
            }
            Operand::Constant(Constant::Float(value)) => {
                value.validate()?;
                Ok(Ty::Float(value.ty))
            }
            Operand::Constant(Constant::Char(value)) => {
                value.validate()?;
                Ok(Ty::Char)
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
                if !facts.must_copy(ty)? {
                    return Err("CFG Copy operand requires a Copy type".into());
                }
                Ok(ty.clone())
            }
            Operand::Move(place) => {
                let ty = self.place_ty(place)?;
                if !facts.must_copy(ty)?
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

    fn require_operand_with_facts(
        &self,
        value: &Operand,
        want: &Ty,
        facts: ir::FactsCtx<'_>,
    ) -> Result<(), String> {
        let got = self.operand_ty_with_facts(value, facts)?;
        if coerces(&got, want) {
            Ok(())
        } else {
            Err("CFG operand type mismatch".into())
        }
    }

    /// Slice operations take a whole-local Copy of a shared reference. No slice
    /// place projection, move, constant or never-typed operand is admitted.
    fn slice_operand_ty(&self, value: &Operand, facts: ir::FactsCtx<'_>) -> Result<Ty, String> {
        let Operand::Copy(place) = value else {
            return Err("CFG slice operand must copy a whole local".into());
        };
        if !place.projection.is_empty() {
            return Err("CFG slice operand must copy a whole local".into());
        }
        self.operand_ty_with_facts(value, facts)
    }

    fn indexed_element_ty(
        &self,
        array: &Place,
        index: &Operand,
        facts: ir::FactsCtx<'_>,
    ) -> Result<&Ty, String> {
        self.require_operand_with_facts(index, &Ty::Int(IntTy::Usize), facts)?;
        let array_ty = self.place_ty(array)?;
        if facts.validated().is_none() {
            validate_type(array_ty, 0)?;
        }
        facts.validate_ty(array_ty)?;
        let Ty::Array { element, .. } = array_ty else {
            return Err("dynamic indexing requires array place".into());
        };
        Ok(element)
    }

    fn indexed_mutation_ty(&self, array: &Place) -> Result<&Ty, String> {
        let local = self.locals.get(array.local.0).ok_or("invalid CFG local")?;
        if !array.projection.contains(&Projection::Deref) && !local.mutable {
            return Err("compound indexed mutation requires mutable binding".into());
        }
        ir::projected_mut_ty(&local.ty, &array.projection)
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
                    count.ty.is_signed() && count.bits & (1_u128 << (count.ty.bits() - 1)) != 0;
                if negative || count.bits >= u128::from(integer.bits()) {
                    return Err("CFG constant shift count is negative or exceeds LHS width".into());
                }
            }
            return Ok(left);
        }
        let comparison = matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        );
        if left == Ty::Char || right == Ty::Char {
            return if comparison && left == right {
                Ok(Ty::Bool)
            } else {
                Err("CFG char permits only same-type comparisons".into())
            };
        }
        if matches!(left, Ty::Float(_)) {
            if matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor) {
                return Err("CFG bitwise operator requires integer or bool operands".into());
            }
            if left != right {
                return Err("CFG operand type mismatch".into());
            }
            return Ok(if comparison { Ty::Bool } else { left });
        }
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

    fn compound_ty(
        &self,
        op: BinaryOp,
        target: &Ty,
        value: &Operand,
        facts: ir::FactsCtx<'_>,
    ) -> Result<(), String> {
        let right = self.operand_ty_with_facts(value, facts)?;
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

    #[cfg(test)]
    fn rvalue_ty(&self, value: &Rvalue) -> Result<Ty, String> {
        self.rvalue_ty_with_facts(value, ir::FactsCtx::Concrete)
    }
    fn rvalue_ty_with_facts(&self, value: &Rvalue, facts: ir::FactsCtx<'_>) -> Result<Ty, String> {
        match value {
            Rvalue::LeadingZeros { value } => {
                if !matches!(self.operand_ty_with_facts(value, facts)?, Ty::Int(_)) {
                    return Err("CFG leading zeros requires integer source".into());
                }
                Ok(Ty::Int(IntTy::U32))
            }
            Rvalue::IntCast { value, to } => {
                if !matches!(self.operand_ty_with_facts(value, facts)?, Ty::Int(_)) {
                    return Err("CFG integer cast requires integer source".into());
                }
                Ok(Ty::Int(*to))
            }
            Rvalue::FloatCast { value, to } => {
                let from = ir::NumTy::of(&self.operand_ty_with_facts(value, facts)?)
                    .ok_or("CFG float cast requires numeric source")?;
                if !matches!(
                    (from, to),
                    (ir::NumTy::Float(_), _) | (_, ir::NumTy::Float(_))
                ) {
                    return Err("CFG float cast requires a float endpoint".into());
                }
                Ok(to.ty())
            }
            Rvalue::Borrow(id) => {
                let site = self.loan_sites.get(id.0).ok_or("invalid CFG loan site")?;
                let referent = self.place_ty(&site.place)?;
                let ty = Ty::Ref {
                    kind: site.kind,
                    referent: std::sync::Arc::new(referent.clone()),
                };
                if facts.validated().is_none() {
                    ir::reference_layers(&ty)?;
                }
                facts.validate_ty(&ty)?;
                Ok(ty)
            }
            Rvalue::IndexCopy { array, index } => {
                let element = self.indexed_element_ty(array, index, facts)?;
                if !facts.must_copy(element)? {
                    return Err("unsupported: non-Copy dynamic array element extraction".into());
                }
                Ok(element.clone())
            }
            Rvalue::Repeat { ty, value } => {
                if facts.validated().is_none() {
                    validate_type(ty, 0)?;
                }
                facts.validate_ty(ty)?;
                let Ty::Array { element, len } = ty else {
                    return Err("CFG repeat requires array type".into());
                };
                self.require_operand_with_facts(value, element, facts)?;
                if *len > 1 && !facts.must_copy(element)? {
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
                if facts.validated().is_none() {
                    validate_type(&Ty::Enum(ty.clone()), 0)?;
                }
                facts.validate_ty(&Ty::Enum(ty.clone()))?;
                let def = ty
                    .def()
                    .variants
                    .get(*variant)
                    .ok_or("invalid CFG enum variant")?;
                if fields.len() != def.fields.len() {
                    return Err("CFG enum initializer arity mismatch".into());
                }
                for (field, declaration) in fields.iter().zip(def.fields.iter()) {
                    self.require_operand_with_facts(field, &declaration.ty, facts)?;
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
            Rvalue::Use(operand) => self.operand_ty_with_facts(operand, facts),
            Rvalue::ArrayToSlice { reference } => {
                let source = self.slice_operand_ty(reference, facts)?;
                let (element, _) = source
                    .shared_array_element_len()
                    .ok_or("CFG array-to-slice requires shared array reference")?;
                ir::validate_shared_slice_element_with_facts(element, facts)?;
                let Ty::Ref { referent, .. } = &source else {
                    return Err("CFG array-to-slice requires shared array reference".into());
                };
                let Ty::Array { element, .. } = referent.as_ref() else {
                    return Err("CFG array-to-slice requires shared array reference".into());
                };
                if let Some(authority) = facts.validated() {
                    authority.budget().charge_nodes(1)?;
                    authority.budget().retain_bytes(
                        std::mem::size_of::<Ty>() + 2 * std::mem::size_of::<usize>(),
                    )?;
                }
                Ok(Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: std::sync::Arc::new(Ty::Slice {
                        element: element.clone(),
                    }),
                })
            }
            Rvalue::SliceLen { slice } => {
                let source = self.slice_operand_ty(slice, facts)?;
                let element = source
                    .shared_slice_element()
                    .ok_or("CFG slice length requires shared slice")?;
                ir::validate_shared_slice_element_with_facts(element, facts)?;
                Ok(Ty::Int(IntTy::Usize))
            }
            Rvalue::SliceIndex { slice, index } => {
                let source = self.slice_operand_ty(slice, facts)?;
                let element = source
                    .shared_slice_element()
                    .ok_or("CFG slice index requires shared slice")?;
                ir::validate_shared_slice_element_with_facts(element, facts)?;
                self.require_operand_with_facts(index, &Ty::Int(IntTy::Usize), facts)?;
                if !facts.must_copy(element)? {
                    return Err("CFG slice index cannot copy a non-Copy element".into());
                }
                Ok(element.clone())
            }
            Rvalue::Println(_) | Rvalue::Output(_) => Ok(Ty::Unit),
            Rvalue::Unary { op, value } => {
                let ty = self.operand_ty_with_facts(value, facts)?;
                match (op, &ty) {
                    (UnaryOp::Neg, Ty::Int(integer)) if integer.is_signed() => Ok(ty),
                    (UnaryOp::Neg, Ty::Float(_)) => Ok(ty),
                    (UnaryOp::Not, Ty::Int(_) | Ty::Bool) => Ok(ty),
                    (UnaryOp::Not, Ty::Never) => Ok(Ty::Never),
                    _ => Err("CFG unary operator requires compatible numeric or bool type".into()),
                }
            }
            Rvalue::Binary { op, left, right } => {
                let left_ty = self.operand_ty_with_facts(left, facts)?;
                let right_ty = self.operand_ty_with_facts(right, facts)?;
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
                if facts.validated().is_none() {
                    validate_type(ty, 0)?;
                }
                facts.validate_ty(ty)?;
                if let Ty::Array { element, len } = ty {
                    if fields.len() != *len {
                        return Err("CFG array aggregate length mismatch".into());
                    }
                    for field in fields {
                        self.require_operand_with_facts(field, element, facts)?;
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
                    self.require_operand_with_facts(
                        value,
                        ty.field_ty(index).ok_or("invalid CFG aggregate field")?,
                        facts,
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
        if let Some(authority) = facts.validated()
            && authority.phase() == super::traits::FactsPhase::Schema
        {
            return Err("schema facts cannot validate executable CFG".into());
        }
        for local in &self.locals {
            facts.validate_ty(&local.ty)?;
        }
        facts.validate_ty(&self.result)?;
        for local in &self.locals {
            validate_slice_position(&local.ty, facts)?;
        }
        if self.result.contains_slice() {
            return Err(SLICE_UNSUPPORTED.into());
        }
        let block = |id: BlockId| -> Result<(), String> {
            if id.0 < self.blocks.len() {
                Ok(())
            } else {
                Err("invalid CFG block".into())
            }
        };
        block(self.entry)?;
        if let Some(authority) = facts.validated() {
            self.validate_provenance(authority, authority.budget())?;
        } else {
            for local in &self.locals {
                validate_type(&local.ty, 0)?;
            }
            validate_type(&self.result, 0)?;
        }
        if self.params.len() >= ir::MAX_PATTERN_PROJECTION_WORK {
            return Err("reference contract parameter resource limit exceeded".into());
        }
        let parameter_types = self
            .params
            .iter()
            .map(|param| self.place_ty(&place(*param)).cloned())
            .collect::<Result<Vec<_>, _>>()?;
        if facts.validated().is_none() {
            self.ref_contract.validate(&parameter_types, &self.result)?;
        }
        if let Some(authority) = facts.validated() {
            ir::validate_body_signature(
                &ir::Signature {
                    ref_contract: self.ref_contract.clone(),
                    params: parameter_types.clone(),
                    result: self.result.clone(),
                },
                authority,
            )?;
        }
        drop(parameter_types);
        if self.loan_sites.len() > ir::MAX_LOAN_SITES {
            return Err("CFG loan site limit exceeded".into());
        }
        let mut origins = self.loan_sites.len();
        for param in &self.params {
            origins = origins
                .checked_add(concrete_carrier_count(
                    self.place_ty(&place(*param))?,
                    facts,
                )?)
                .ok_or("reference origin overflow")?;
            if origins > ir::MAX_LOAN_SITES {
                return Err("CFG combined reference origin limit exceeded".into());
            }
        }
        for block in &self.blocks {
            if let Terminator::Call { signature, .. } = &block.terminator.value {
                origins = origins
                    .checked_add(concrete_carrier_count(&signature.result, facts)?)
                    .ok_or("reference origin overflow")?;
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
                    self.validate_operand_facts(operand, facts)?;
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
                        self.indexed_mutation_ty(array)?;
                        let element = self.indexed_element_ty(array, index, facts)?;
                        self.compound_ty(*op, element, value, facts)?;
                    }
                    Statement::IndexAssign {
                        array,
                        index,
                        value,
                    } => {
                        let element = self.indexed_element_ty(array, index, facts)?;
                        self.require_operand_with_facts(value, element, facts)?;
                    }
                    Statement::IndexInspect { array, index } => {
                        self.indexed_element_ty(array, index, facts)?;
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
                        let got = self.rvalue_ty_with_facts(value, facts)?;
                        facts.validate_ty(&got)?;
                        let want = self.place_ty(destination)?;
                        if !coerces(&got, want) {
                            return Err("CFG assignment type mismatch".into());
                        }
                    }
                }
            }
            item.terminator.value.visit_operands(&mut |operand| {
                self.validate_operand_facts(operand, facts)?;
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
                Terminator::Drop { cleanup, next } => {
                    let authority = facts
                        .validated()
                        .ok_or("cleanup requires validated canonical facts")?;
                    if self.lowering_proof.is_none() {
                        return Err("cleanup requires exact canonical lowering proof".into());
                    }
                    cleanup.validate(self, authority)?;
                    block(*next)?;
                }
                Terminator::Goto(next) => block(*next)?,
                Terminator::Switch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    self.require_operand_with_facts(condition, &Ty::Bool, facts)?;
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
                    if let Some(authority) = facts.validated()
                        && authority
                            .is_canonical_drop_call(target)
                            .map_err(|e| e.into_legacy_diagnostic(authority.budget()))?
                    {
                        return Err("canonical destructor invocation requires an authorized cleanup operation".into());
                    }
                    ir::validate_callee(target, signature, facts)?;
                    if target.type_arguments().len() > 32 {
                        return Err("generic call argument limit exceeded".into());
                    }
                    for ty in target
                        .metadata_types()
                        .chain(signature.params.iter())
                        .chain(std::iter::once(&signature.result))
                    {
                        facts.validate_ty(ty)?;
                    }
                    if signature.params.len() != args.len() {
                        return Err("CFG call arity mismatch".into());
                    }
                    for ty in &signature.params {
                        if facts.validated().is_none() {
                            validate_type(ty, 0)?;
                        }
                        facts.validate_ty(ty)?;
                    }
                    if facts.validated().is_none() {
                        validate_type(&signature.result, 0)?;
                    }
                    facts.validate_ty(&signature.result)?;
                    if facts.validated().is_none() {
                        signature
                            .ref_contract
                            .validate(&signature.params, &signature.result)?;
                    }
                    for (arg, ty) in args.iter().zip(&signature.params) {
                        self.require_operand_with_facts(arg, ty, facts)?;
                    }
                    let want = self.place_ty(destination)?;
                    if !coerces(&signature.result, want) {
                        return Err("CFG call destination type mismatch".into());
                    }
                    block(*next)?;
                }
                Terminator::Return | Terminator::Panic(_) | Terminator::Unreachable => {}
            }
        }
        if loan_uses.iter().any(|count| *count != 1) {
            return Err("CFG loan site must have exactly one borrow instruction".into());
        }
        self.validate_loan_timing(facts)
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

struct PatternTest {
    selected: BlockId,
    rejected: BlockId,
    temporaries: Vec<LocalId>,
}

// Fields drop in declaration order: scratch allocations are released before
// either reservation, including allocator-rounded excess capacity.
struct DivergenceScratch {
    seen: Vec<u8>,
    pending: Vec<BlockId>,
    _requested: Option<ir::PlanReservation>,
    _rounded: Option<ir::PlanReservation>,
}

struct LoopTarget {
    id: LoopId,
    head: Option<BlockId>,
    exit: BlockId,
    destination: Option<Place>,
    scope_depth: usize,
}

struct Lowerer<'a> {
    facts: ir::FactsCtx<'a>,
    budget: Option<&'a ir::PlanBudget>,
    resource_error: Option<String>,
    lowering_depth: usize,
    lowering_work: usize,
    origins: usize,
    loans: Vec<Option<LoanId>>,
    call_blocks: Vec<Option<BlockId>>,
    used_loans: std::collections::HashSet<LoanId>,
    source: &'a ir::CheckedFunction,
    result: Function,
    expressions: Vec<LocalId>,
    parameters: std::collections::HashSet<LocalId>,
    blocks: Vec<PendingBlock>,
    current: BlockId,
    drop_enabled: bool,
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

    fn pattern_test(
        &mut self,
        id: ExprId,
        source: &Place,
        pattern: &ir::MatchPattern,
        completes: bool,
    ) -> Result<PatternTest, String> {
        if let ir::MatchPattern::Decision { tests, .. } = pattern {
            return self.decision_pattern_test(id, source, tests, completes);
        }
        let selected = self.block();
        let rejected = self.block();
        self.scopes.push(Vec::new());
        let comparison = match pattern {
            ir::MatchPattern::Bool(value) => {
                Some((Operand::Copy(source.clone()), Constant::Bool(*value)))
            }
            ir::MatchPattern::Int(value) => {
                Some((Operand::Copy(source.clone()), Constant::Int(*value)))
            }
            _ => None,
        };
        if let ir::MatchPattern::EnumVariant { ty, variant, .. } = pattern
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
                    else_block: rejected,
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
                    else_block: rejected,
                },
                Some(id),
            )?;
        } else {
            // Keep dead syntax visible without a runtime tag or value read.
            self.terminate(
                Terminator::Switch {
                    condition: Operand::Constant(Constant::Bool(true)),
                    then_block: selected,
                    else_block: rejected,
                },
                Some(id),
            )?;
        }
        Ok(PatternTest {
            selected,
            rejected,
            temporaries: self.scopes.pop().unwrap(),
        })
    }

    /// All comparison storage is live at entry, even on an early rejection.
    /// The shared success/failure cleanup therefore never ends storage that
    /// was only conditionally created by a later predicate.
    #[inline(never)]
    fn decision_pattern_test(
        &mut self,
        id: ExprId,
        source: &Place,
        tests: &[ir::PatternTest],
        completes: bool,
    ) -> Result<PatternTest, String> {
        self.divergence_work(tests.len())?;
        let selected = self.block();
        let rejected = self.block();
        self.scopes.push(Vec::new());
        if !completes {
            // IR validation still checks every dead predicate and binding.
            // No runtime read or fabricated value is needed for a Never input.
            self.terminate(
                Terminator::Switch {
                    condition: Operand::Constant(Constant::Bool(true)),
                    then_block: selected,
                    else_block: rejected,
                },
                Some(id),
            )?;
        } else {
            for test in tests {
                if matches!(
                    test.kind,
                    ir::PatternTestKind::Bool(_) | ir::PatternTestKind::Int(_)
                ) {
                    self.temporary(Ty::Bool, Some(id));
                }
            }
            let mut scalar = 0;
            for (index, test) in tests.iter().enumerate() {
                let target = if index + 1 == tests.len() {
                    selected
                } else {
                    self.block()
                };
                let inspected = self.decision_place(source, &test.projection)?;
                let right = match &test.kind {
                    ir::PatternTestKind::Bool(value) => Constant::Bool(*value),
                    ir::PatternTestKind::Int(value) => Constant::Int(*value),
                    ir::PatternTestKind::EnumVariant { ty, variant } => {
                        // Retain singleton tag edges too: the true edge proves
                        // the exact downcast prerequisite for subsequent tests.
                        self.terminate(
                            Terminator::SwitchVariant {
                                place: inspected,
                                variant: ir::VariantId {
                                    owner: ty.def().id,
                                    index: *variant,
                                },
                                then_block: target,
                                else_block: rejected,
                            },
                            Some(id),
                        )?;
                        self.current = target;
                        continue;
                    }
                };
                let local = self.scopes.last().unwrap()[scalar];
                scalar += 1;
                self.assign(
                    place(local),
                    Rvalue::Binary {
                        op: BinaryOp::Eq,
                        left: Operand::Copy(inspected),
                        right: Operand::Constant(right),
                    },
                    Some(id),
                );
                self.terminate(
                    Terminator::Switch {
                        condition: Operand::Copy(place(local)),
                        then_block: target,
                        else_block: rejected,
                    },
                    Some(id),
                )?;
                self.current = target;
            }
        }
        Ok(PatternTest {
            selected,
            rejected,
            temporaries: self.scopes.pop().unwrap(),
        })
    }

    fn decision_place(&mut self, source: &Place, suffix: &[Projection]) -> Result<Place, String> {
        let count = source
            .projection
            .len()
            .checked_add(suffix.len())
            .ok_or_else(|| self.divergence_resource("CFG predicate projection overflow"))?;
        let work = count
            .checked_add(1)
            .ok_or_else(|| self.divergence_resource("CFG predicate work overflow"))?;
        self.divergence_work(work)?;
        let requested = count
            .checked_mul(std::mem::size_of::<Projection>())
            .ok_or_else(|| self.divergence_resource("CFG predicate allocation overflow"))?;
        if !self.allocate(0, requested) {
            return Err(self.resource_error.clone().unwrap());
        }
        let mut projection = Vec::new();
        projection
            .try_reserve_exact(count)
            .map_err(|_| self.divergence_resource("CFG predicate allocation failure"))?;
        let actual = projection
            .capacity()
            .checked_mul(std::mem::size_of::<Projection>())
            .ok_or_else(|| self.divergence_resource("CFG predicate allocation overflow"))?;
        let excess = actual
            .checked_sub(requested)
            .ok_or_else(|| self.divergence_resource("CFG predicate allocation underflow"))?;
        if !self.allocate(0, excess) {
            return Err(self.resource_error.clone().unwrap());
        }
        projection.extend_from_slice(&source.projection);
        projection.extend_from_slice(suffix);
        Ok(Place {
            local: source.local,
            projection,
        })
    }

    /// Storage belongs to the caller's active scope. Match arms open a fresh
    /// arm scope; let-else success deliberately keeps the enclosing scope.
    fn initialize_pattern_bindings(
        &mut self,
        id: ExprId,
        source: &Place,
        pattern: &ir::MatchPattern,
        completes: bool,
    ) -> Result<(), String> {
        if let ir::MatchPattern::Binding(local) = pattern {
            if *local == source.local {
                return Err("match binding aliases scrutinee storage".into());
            }
            self.live(*local, Some(id));
            if completes {
                self.assign(
                    place(*local),
                    Rvalue::Use(self.consume(source.clone())?),
                    Some(id),
                );
            }
        }
        if let ir::MatchPattern::EnumVariant { bindings, .. }
        | ir::MatchPattern::Aggregate { bindings, .. }
        | ir::MatchPattern::Decision { bindings, .. } = pattern
        {
            for binding in bindings {
                if binding.local == source.local {
                    return Err("match field binding aliases scrutinee storage".into());
                }
                self.live(binding.local, Some(id));
                if completes {
                    let selected = if matches!(pattern, ir::MatchPattern::Decision { .. }) {
                        self.decision_place(source, &binding.projection)?
                    } else {
                        let mut selected = source.clone();
                        selected.projection.extend_from_slice(&binding.projection);
                        selected
                    };
                    self.assign(
                        place(binding.local),
                        Rvalue::Use(self.consume(selected)?),
                        Some(id),
                    );
                }
            }
        }
        Ok(())
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
            && arms.iter().any(|arm| {
                matches!(
                    arm.pattern,
                    ir::MatchPattern::EnumVariant { .. } | ir::MatchPattern::Decision { .. }
                )
            })
        {
            return Err("unsupported: enum variant matching through dereference".into());
        }
        let completes = self.source.exprs[scrutinee.0].completion == ir::Completion::Returns;
        let join = self.block();
        for arm in arms {
            let PatternTest {
                selected,
                rejected: next,
                temporaries: test_scope,
            } = self.pattern_test(id, &source, &arm.pattern, completes)?;
            self.current = selected;
            for local in test_scope.iter().rev() {
                self.end_storage(*local, Some(id));
            }
            self.scopes.push(Vec::new());
            self.initialize_pattern_bindings(id, &source, &arm.pattern, completes)?;
            let value = self.expression(arm.body)?;
            if completes {
                self.assign(destination.clone(), Rvalue::Use(value), Some(id));
            }
            self.cleanup(self.scopes.len() - 1, Some(id));
            self.scopes.pop();
            self.terminate(Terminator::Goto(join), Some(id))?;
            self.current = next;
            for local in test_scope.iter().rev() {
                self.end_storage(*local, Some(id));
            }
        }
        self.terminate(Terminator::Unreachable, Some(id))?;
        self.current = join;
        Ok(())
    }

    fn divergence_resource(&mut self, message: &str) -> String {
        self.resource_error
            .get_or_insert_with(|| message.to_owned())
            .clone()
    }

    fn divergence_work(&mut self, amount: usize) -> Result<(), String> {
        if let Some(error) = &self.resource_error {
            return Err(error.clone());
        }
        let next = self
            .lowering_work
            .checked_add(amount)
            .filter(|next| *next <= ir::MAX_PATTERN_PROJECTION_WORK)
            .ok_or_else(|| self.divergence_resource("CFG lowering work resource limit exceeded"))?;
        if let Some(budget) = self.budget {
            budget
                .charge_work(amount)
                .map_err(|error| self.divergence_resource(&error))?;
        }
        self.lowering_work = next;
        Ok(())
    }

    /// Never metadata alone cannot certify a divergent failure branch. Prove
    /// that its actual pending CFG cannot reach the lexical continuation before
    /// sealing that disconnected continuation with Unreachable.
    fn prove_failure_diverges(
        &mut self,
        entry: BlockId,
        continuation: BlockId,
    ) -> Result<(), String> {
        self.divergence_work(1)?;
        let count = self.blocks.len();
        if entry.0 >= count || continuation.0 >= count {
            return Err("invalid let-else divergence block".into());
        }
        if entry == continuation {
            return Err("let-else else branch can fall through".into());
        }
        self.divergence_work(count)?;
        let requested = count
            .checked_mul(std::mem::size_of::<BlockId>() + 1)
            .ok_or_else(|| {
                self.divergence_resource("let-else divergence storage resource overflow")
            })?;
        let reservation = self
            .budget
            .map(|budget| budget.reserve_bytes(requested))
            .transpose()
            .map_err(|error| self.divergence_resource(&error))?;
        let mut seen = Vec::new();
        seen.try_reserve_exact(count).map_err(|_| {
            self.divergence_resource("let-else divergence allocation resource failure")
        })?;
        let mut pending = Vec::new();
        pending.try_reserve_exact(count).map_err(|_| {
            self.divergence_resource("let-else divergence allocation resource failure")
        })?;
        let actual = pending
            .capacity()
            .checked_mul(std::mem::size_of::<BlockId>())
            .and_then(|bytes| bytes.checked_add(seen.capacity()))
            .ok_or_else(|| {
                self.divergence_resource("let-else divergence storage resource overflow")
            })?;
        let excess = actual.checked_sub(requested).ok_or_else(|| {
            self.divergence_resource("let-else divergence storage resource overflow")
        })?;
        let rounded = self
            .budget
            .map(|budget| budget.reserve_bytes(excess))
            .transpose()
            .map_err(|error| self.divergence_resource(&error))?;
        seen.resize(count, 0);
        let mut scratch = DivergenceScratch {
            seen,
            pending,
            _requested: reservation,
            _rounded: rounded,
        };
        scratch.seen[entry.0] = 1;
        scratch.pending.push(entry);
        while let Some(current) = scratch.pending.pop() {
            self.divergence_work(1)?;
            if current == continuation {
                return Err("let-else else branch can fall through".into());
            }
            let successors = match self.blocks[current.0]
                .terminator
                .as_ref()
                .map(|term| &term.value)
            {
                Some(
                    Terminator::Goto(next)
                    | Terminator::Call { next, .. }
                    | Terminator::Drop { next, .. },
                ) => [Some(*next), None],
                Some(Terminator::Switch {
                    condition: Operand::Constant(Constant::Bool(value)),
                    then_block,
                    else_block,
                }) => [Some(if *value { *then_block } else { *else_block }), None],
                Some(
                    Terminator::Switch {
                        then_block,
                        else_block,
                        ..
                    }
                    | Terminator::SwitchVariant {
                        then_block,
                        else_block,
                        ..
                    },
                ) => [Some(*then_block), Some(*else_block)],
                // An open outer break/continue target is a sink in this finite
                // snapshot, not a proof that the local continuation diverges.
                None
                | Some(Terminator::Return | Terminator::Panic(_) | Terminator::Unreachable) => {
                    [None, None]
                }
            };
            for next in successors.into_iter().flatten() {
                self.divergence_work(1)?;
                let visited = scratch
                    .seen
                    .get_mut(next.0)
                    .ok_or("invalid let-else divergence successor")?;
                if *visited == 0 {
                    *visited = 1;
                    // Mark-on-enqueue bounds pending storage by block count.
                    scratch.pending.push(next);
                }
            }
        }
        Ok(())
    }

    fn let_else(
        &mut self,
        value: ExprId,
        ty: &Ty,
        pattern: &ir::MatchPattern,
        else_branch: ExprId,
    ) -> Result<(), String> {
        self.source
            .validate_let_else(value, ty, pattern, else_branch, &self.parameters)?;
        let source = self.pattern_source(value)?;
        if source.projection.contains(&Projection::Deref)
            && matches!(
                pattern,
                ir::MatchPattern::EnumVariant { .. } | ir::MatchPattern::Decision { .. }
            )
        {
            return Err("unsupported: enum variant matching through dereference".into());
        }
        let completes = self.source.exprs[value.0].completion == ir::Completion::Returns;
        let PatternTest {
            selected,
            rejected,
            temporaries,
        } = self.pattern_test(value, &source, pattern, completes)?;
        self.current = rejected;
        for local in temporaries.iter().rev() {
            self.end_storage(*local, Some(value));
        }
        self.scopes.push(Vec::new());
        self.expression(else_branch)?;
        self.cleanup(self.scopes.len() - 1, Some(else_branch));
        self.scopes.pop();
        self.prove_failure_diverges(rejected, self.current)?;
        self.terminate(Terminator::Unreachable, Some(else_branch))?;
        self.current = selected;
        for local in temporaries.iter().rev() {
            self.end_storage(*local, Some(value));
        }
        self.initialize_pattern_bindings(value, &source, pattern, completes)
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
            self.end_storage(local, origin);
        }
    }

    fn assign(&mut self, destination: Place, value: Rvalue, origin: Option<ExprId>) {
        self.statement(Statement::Assign { destination, value }, origin);
    }

    fn consume(&self, place: Place) -> Result<Operand, String> {
        if self.facts.must_copy(self.result.place_ty(&place)?)? {
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
            .require_operand_with_facts(&index, &Ty::Int(IntTy::Usize), self.facts)?;
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
            if self.drop_enabled {
                self.scopes.push(Vec::new());
            }
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
                    self.live_binding(*local, None);
                }
                ir::Stmt::Let { local, value } => {
                    let operand = self.expression(*value)?;
                    self.live_binding(*local, Some(*value));
                    self.assign(place(*local), Rvalue::Use(operand), Some(*value));
                }
                ir::Stmt::LetPattern {
                    value,
                    ty,
                    bindings,
                } => {
                    if drop_normal::descriptor(ty, self.facts)?.is_some() && !bindings.is_empty() {
                        return Err("unsupported: destructuring a normal Drop owner".into());
                    }
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
                        if self.drop_enabled {
                            self.cleanup(self.scopes.len() - 1, None);
                            self.scopes.pop();
                        }
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
                        self.live_binding(binding.local, Some(*value));
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
                ir::Stmt::LetElse {
                    value,
                    ty,
                    pattern,
                    else_branch,
                } => {
                    if drop_normal::descriptor(ty, self.facts)?.is_some() {
                        return Err("unsupported: let-else with a normal Drop owner".into());
                    }
                    self.let_else(*value, ty, pattern, *else_branch)?;
                }
                ir::Stmt::Assign { local, value } => {
                    let operand = self.expression(*value)?;
                    self.drop_value(*local, Some(*value));
                    self.assign(place(*local), Rvalue::Use(operand), Some(*value));
                }
                ir::Stmt::AssignPlace { place, value } => {
                    if !place.projection.is_empty()
                        && drop_normal::descriptor(self.result.place_ty(place)?, self.facts)?
                            .is_some()
                    {
                        return Err("unsupported: indirect normal Drop replacement".into());
                    }
                    let operand = self.expression(*value)?;
                    self.inspect_index_prefixes(place, Some(*value))?;
                    if place.projection.is_empty() {
                        self.drop_value(place.local, Some(*value));
                    }
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
                        head: Some(head),
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
                        self.end_storage(*local, Some(*condition));
                    }
                    self.body(body, None)?;
                    self.terminate(Terminator::Goto(head), None)?;
                    self.current = condition_false;
                    for local in condition_scope.iter().rev() {
                        self.end_storage(*local, Some(*condition));
                    }
                    self.terminate(Terminator::Goto(exit), Some(*condition))?;
                    self.current = exit;
                    self.loops.pop();
                }
            }
            if self.drop_enabled {
                self.cleanup(self.scopes.len() - 1, None);
                self.scopes.pop();
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
            Some(ir::ExprKind::LeadingZeros { value }) => {
                self.materialize_cast(id, *value, |value| Rvalue::LeadingZeros { value })
            }
            Some(ir::ExprKind::IntCast { value, to }) => {
                self.materialize_cast(id, *value, |value| Rvalue::IntCast { value, to: *to })
            }
            Some(ir::ExprKind::FloatCast { value, to }) => {
                self.materialize_cast(id, *value, |value| Rvalue::FloatCast { value, to: *to })
            }
            _ => self.materialize_expression_inner(id),
        };
        self.lowering_depth -= 1;
        result
    }

    fn materialize_cast(
        &mut self,
        id: ExprId,
        value: ExprId,
        cast: impl FnOnce(Operand) -> Rvalue,
    ) -> Result<Place, String> {
        let destination = place(
            *self
                .expressions
                .get(id.0)
                .ok_or("missing expression local")?,
        );
        self.live(destination.local, Some(id));
        let value = self.expression(value)?;
        self.assign(destination.clone(), cast(value), Some(id));
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
            // The operand is evaluated exactly once into its own snapshot. The
            // slice result is a Copy of that shared reference plus metadata.
            ir::ExprKind::ArrayToSlice { reference } => {
                let reference = self.expression(*reference)?;
                self.assign(
                    destination.clone(),
                    Rvalue::ArrayToSlice { reference },
                    origin,
                );
            }
            ir::ExprKind::SliceLen { slice } => {
                let slice = self.expression(*slice)?;
                self.assign(destination.clone(), Rvalue::SliceLen { slice }, origin);
            }
            // Receiver first, then index, each once into a distinct snapshot.
            ir::ExprKind::SliceIndex { slice, index } => {
                let slice = self.expression(*slice)?;
                let index = self.expression(*index)?;
                self.assign(
                    destination.clone(),
                    Rvalue::SliceIndex { slice, index },
                    origin,
                );
            }
            ir::ExprKind::LeadingZeros { value } => {
                let value = self.expression(*value)?;
                self.assign(destination.clone(), Rvalue::LeadingZeros { value }, origin);
            }
            ir::ExprKind::IntCast { value, to } => {
                let value = self.expression(*value)?;
                self.assign(
                    destination.clone(),
                    Rvalue::IntCast { value, to: *to },
                    origin,
                );
            }
            ir::ExprKind::FloatCast { value, to } => {
                let value = self.expression(*value)?;
                self.assign(
                    destination.clone(),
                    Rvalue::FloatCast { value, to: *to },
                    origin,
                );
            }
            ir::ExprKind::Float(value) => self.assign(
                destination.clone(),
                Rvalue::Use(Operand::Constant(Constant::Float(*value))),
                origin,
            ),
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
                self.result.operand_ty_with_facts(&operand, self.facts)?;
                self.assign(destination.clone(), Rvalue::Use(operand), origin);
            }
            ir::ExprKind::DynamicIndex { base, index } => {
                if !self.facts.must_copy(&expression.ty)? {
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
            ir::ExprKind::Char(value) => self.assign(
                destination.clone(),
                Rvalue::Use(Operand::Constant(Constant::Char(*value))),
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
                // Each lazy operand is its own temporary scope: LHS temporaries
                // end before the RHS is evaluated or the shortcut is taken.
                if self.drop_enabled {
                    self.scopes.push(Vec::new());
                }
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
                let left_scope = if self.drop_enabled {
                    self.scopes.pop().expect("lazy operand scope")
                } else {
                    Vec::new()
                };
                self.current = shortcut;
                for local in left_scope.iter().rev() {
                    self.end_storage(*local, origin);
                }
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
                for local in left_scope.iter().rev() {
                    self.end_storage(*local, origin);
                }
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
                self.origins = self
                    .origins
                    .checked_add(concrete_carrier_count(&signature.result, self.facts)?)
                    .ok_or("reference origin overflow")?;
                if self.origins > ir::MAX_LOAN_SITES {
                    return Err("CFG combined reference origin limit exceeded".into());
                }
                let args = args
                    .iter()
                    .map(|id| self.expression(*id))
                    .collect::<Result<Vec<_>, _>>()?;
                let next = self.block();
                if self.call_blocks[id.0].replace(self.current).is_some() {
                    if let Some(budget) = self.budget {
                        budget.charge_work(self.source.exprs.len())?;
                    }
                    if self.source.exprs.iter().any(|expression| matches!(&expression.kind, ir::ExprKind::Borrow { timing: ir::BorrowTiming::TwoPhase { activation, .. }, .. } if *activation == id)) {
                        return Err("two-phase call expression was materialized more than once".into());
                    }
                }
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
                // Edition 2024: the whole if condition is a temporary scope,
                // so its temporaries end before either branch runs.
                if self.drop_enabled {
                    self.scopes.push(Vec::new());
                }
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
                let condition_scope = if self.drop_enabled {
                    self.scopes.pop().expect("if condition scope")
                } else {
                    Vec::new()
                };
                self.current = yes;
                for local in condition_scope.iter().rev() {
                    self.end_storage(*local, origin);
                }
                self.body(then_block, Some(destination.clone()))?;
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = no;
                for local in condition_scope.iter().rev() {
                    self.end_storage(*local, origin);
                }
                self.body(else_block, Some(destination.clone()))?;
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = join;
            }
            ir::ExprKind::Block(body) => self.body(body, Some(destination.clone()))?,
            ir::ExprKind::LabeledBlock { id: block_id, body } => {
                let exit = self.block();
                self.loops.push(LoopTarget {
                    id: *block_id,
                    head: None,
                    exit,
                    destination: Some(destination.clone()),
                    scope_depth: self.scopes.len(),
                });
                self.body(body, Some(destination.clone()))?;
                self.terminate(Terminator::Goto(exit), origin)?;
                self.loops.pop();
                self.current = exit;
            }
            ir::ExprKind::Loop { id: loop_id, body } => {
                let head = self.block();
                let exit = self.block();
                self.terminate(Terminator::Goto(head), origin)?;
                self.loops.push(LoopTarget {
                    id: *loop_id,
                    head: Some(head),
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
                let (head, depth) = (
                    frame.head.ok_or("cannot continue a labeled block")?,
                    frame.scope_depth,
                );
                self.cleanup(depth, origin);
                self.terminate(Terminator::Goto(head), origin)?;
                self.disconnected();
            }
            ir::ExprKind::Panic(payload) => {
                // The source preflight covers the arena entry only. A shared
                // ExprId can be lowered repeatedly, allocating a fresh Box and
                // both diagnostic strings on each materialization.
                let bytes = std::mem::size_of::<ir::PanicMessage>()
                    .checked_add(payload.message.len())
                    .and_then(|bytes| bytes.checked_add(payload.location.len()))
                    .ok_or_else(|| {
                        self.divergence_resource("CFG panic payload allocation overflow")
                    })?;
                if !self.allocate(0, bytes) {
                    return Err(self.resource_error.clone().unwrap());
                }
                self.terminate(Terminator::Panic(payload.clone()), origin)?;
                self.disconnected();
            }
            ir::ExprKind::Output(output) => {
                // Each materialization owns fresh literal storage, including
                // repeated uses of a shared expression arena row.
                if !self.allocate(0, output.text.len()) {
                    return Err(self.resource_error.clone().unwrap());
                }
                self.assign(destination.clone(), Rvalue::Output(output.clone()), origin)
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
    lower_internal(source, ir::FactsCtx::Concrete, None)
}

#[cfg(test)]
mod literal_output_tests {
    use super::*;

    fn fixture(output: ir::LiteralOutput) -> ir::CheckedFunction {
        ir::CheckedFunction {
            call_evidence: std::sync::Arc::from([]),
            body_substitution: None,
            ref_contract: Default::default(),
            name: "literal_output".into(),
            params: vec![],
            result: Ty::Unit,
            locals: vec![],
            exprs: vec![ir::TypedExpr {
                ty: Ty::Unit,
                completion: ir::Completion::Returns,
                kind: ir::ExprKind::Output(output),
            }],
            body: ir::Block {
                statements: vec![ir::Stmt::Expr(ExprId(0))],
                tail: None,
            },
        }
    }

    #[test]
    fn output_preserves_payload_and_has_no_operands() {
        for stream in [ir::OutputStream::Stdout, ir::OutputStream::Stderr] {
            for newline in [false, true] {
                for text in ["", "λ\0{}\n"] {
                    let output = ir::LiteralOutput {
                        text: text.into(),
                        stream,
                        newline,
                    };
                    let source = fixture(output.clone());
                    let cfg = lower(&source).unwrap();
                    cfg.validate().unwrap();
                    let outputs: Vec<_> = cfg
                        .blocks
                        .iter()
                        .flat_map(|b| &b.statements)
                        .filter_map(|statement| match &statement.value {
                            Statement::Assign {
                                value: Rvalue::Output(output),
                                ..
                            } => Some(output),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(outputs, vec![&output]);
                    let mut operands = 0;
                    Rvalue::Output(output)
                        .visit_operands(&mut |_| {
                            operands += 1;
                            Ok(())
                        })
                        .unwrap();
                    assert_eq!(operands, 0);
                }
            }
        }
    }

    #[test]
    fn output_storage_and_each_materialization_are_metered() {
        let mut measurements = Vec::new();
        for text in [String::new(), "x".repeat(4096)] {
            let mut source = fixture(ir::LiteralOutput {
                text,
                stream: ir::OutputStream::Stdout,
                newline: false,
            });
            source.body.statements.push(ir::Stmt::Expr(ExprId(0)));
            let budget = ir::PlanBudget::default();
            let cfg = lower_with_budget(&source, &budget).unwrap();
            let snapshot_bytes = cfg.snapshot_storage(&ir::PlanBudget::default()).unwrap();
            measurements.push((budget.usage_snapshot().bytes, snapshot_bytes));
        }
        // The arena retains one payload, and both evaluations clone it.
        assert_eq!(measurements[1].0 - measurements[0].0, 3 * 4096);
        // A CFG snapshot retains both independently emitted payloads.
        assert_eq!(measurements[1].1 - measurements[0].1, 2 * 4096);
    }

    #[test]
    fn unused_output_requires_exact_unit_and_returns() {
        let mut source = fixture(ir::LiteralOutput {
            text: "unused".into(),
            stream: ir::OutputStream::Stderr,
            newline: false,
        });
        source.body.statements.clear();
        source.validate().unwrap();
        assert!(
            !lower(&source)
                .unwrap()
                .blocks
                .iter()
                .flat_map(|b| &b.statements)
                .any(|statement| matches!(
                    &statement.value,
                    Statement::Assign {
                        value: Rvalue::Output(_),
                        ..
                    }
                ))
        );
        for (ty, completion) in [
            (Ty::Bool, ir::Completion::Returns),
            (Ty::Unit, ir::Completion::Never),
            (Ty::Never, ir::Completion::Never),
        ] {
            source.exprs[0].ty = ty;
            source.exprs[0].completion = completion;
            assert!(
                source
                    .validate()
                    .unwrap_err()
                    .contains("literal output requires exact Unit")
            );
            assert!(lower(&source).is_err());
        }
    }
}
pub fn lower_with_budget(
    source: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<Function, String> {
    lower_internal(source, ir::FactsCtx::Concrete, Some(budget))
}
pub fn lower_with_facts(
    source: &ir::CheckedFunction,
    facts: &super::traits::ValidatedFacts,
    budget: &ir::PlanBudget,
) -> Result<Function, String> {
    require_facts(facts, super::traits::FactsPhase::Closed, budget)?;
    let mut function = lower_internal(source, ir::FactsCtx::from_validated(facts), Some(budget))?;
    if source.body_substitution.is_some() || !source.call_evidence.is_empty() {
        source.preflight_plan(ir::FactsCtx::from_validated(facts), budget)?;
        budget.retain_bytes(
            std::mem::size_of::<ir::CheckedFunction>()
                + std::mem::size_of::<super::traits::ValidatedFacts>()
                + 4 * std::mem::size_of::<usize>(),
        )?;
        function.call_evidence = ir::bind_call_evidence(
            std::sync::Arc::new(source.clone()),
            std::sync::Arc::new(facts.clone()),
            budget,
        )?;
    }
    if source.body_substitution.is_some() || !source.call_evidence.is_empty() {
        function.lowering_proof = Some(std::sync::Arc::new(GeneratedCfgShape::capture(
            &function, facts, budget,
        )?));
    }
    function.validate_provenance(facts, budget)?;
    Ok(function)
}
fn lower_internal(
    source: &ir::CheckedFunction,
    facts: ir::FactsCtx<'_>,
    budget: Option<&ir::PlanBudget>,
) -> Result<Function, String> {
    lower_internal_with_source(source, facts, budget, SourceLowering::Legacy)
}

enum SourceLowering<'a> {
    Legacy,
    Original(&'a std::sync::Arc<super::source_replay::CanonicalSourcePublication>),
}

/// Shared slices are whole root locals/temporaries, never results, aggregates
/// or nested reference positions. Element authority is checked separately.
pub(super) const SLICE_UNSUPPORTED: &str =
    "unsupported: slice type outside root shared slice locals is not supported by CFG lowering";

fn validate_slice_position(ty: &Ty, facts: ir::FactsCtx<'_>) -> Result<(), String> {
    if let Some(element) = ty.shared_slice_element() {
        ir::validate_shared_slice_element_with_facts(element, facts)?;
    } else if ty.contains_slice() {
        return Err(SLICE_UNSUPPORTED.into());
    }
    Ok(())
}

fn reject_slices(
    source: &ir::CheckedFunction,
    facts: ir::FactsCtx<'_>,
    budget: Option<&ir::PlanBudget>,
) -> Result<(), String> {
    if let Some(budget) = budget {
        budget.charge_work(
            source
                .exprs
                .len()
                .checked_add(source.locals.len())
                .and_then(|count| count.checked_add(1))
                .ok_or("CFG slice type work overflow")?,
        )?;
    }
    if source.result.contains_slice() {
        return Err(SLICE_UNSUPPORTED.into());
    }
    for local in &source.locals {
        validate_slice_position(&local.ty, facts)?;
    }
    for expression in &source.exprs {
        validate_slice_position(&expression.ty, facts)?;
    }
    Ok(())
}

fn lower_internal_with_source(
    source: &ir::CheckedFunction,
    facts: ir::FactsCtx<'_>,
    budget: Option<&ir::PlanBudget>,
    authority: SourceLowering<'_>,
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
    match authority {
        SourceLowering::Legacy => source.validate_with_facts(facts)?,
        SourceLowering::Original(publication) => {
            let budget = budget.ok_or("original source lowering requires planning authority")?;
            let facts = facts
                .validated()
                .ok_or("original source lowering requires validated facts")?;
            if !std::ptr::eq(source, publication.source().as_ref()) {
                return Err("source lowering changed exact checked payload".into());
            }
            publication
                .validate_source(publication.source(), facts, budget)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
            ir::validate_published_source(publication, budget)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
        }
    }
    // Metadata and exact publication authority precede unsupported-position
    // classification, including for unused and concretely substituted types.
    reject_slices(source, facts, budget)?;
    let drop_enabled = drop_normal::admit(source, facts)?;
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
        formal_origins = formal_origins
            .checked_add(concrete_carrier_count(ty, facts)?)
            .ok_or("reference origin overflow")?;
        if formal_origins > ir::MAX_LOAN_SITES {
            return Err("CFG combined reference origin limit exceeded".into());
        }
    }
    let formal_parameter_origins = formal_origins;
    for expression in &source.exprs {
        if matches!(expression.kind, ir::ExprKind::Call { .. }) {
            formal_origins = formal_origins
                .checked_add(concrete_carrier_count(&expression.ty, facts)?)
                .ok_or("reference origin overflow")?;
            if formal_origins > ir::MAX_LOAN_SITES {
                return Err("CFG combined reference origin limit exceeded".into());
            }
        }
    }
    let mut loan_sites = Vec::new();
    if let Some(budget) = budget {
        budget.charge_work(source.exprs.len())?;
        budget.retain_bytes(
            source
                .exprs
                .len()
                .checked_mul(std::mem::size_of::<Option<BlockId>>())
                .ok_or("activation map storage overflow")?,
        )?;
    }
    let mut loans = vec![None; source.exprs.len()];
    for (index, expression) in source.exprs.iter().enumerate() {
        if let ir::ExprKind::Borrow { kind, place, .. } = &expression.kind {
            if loan_sites.len() >= ir::MAX_LOAN_SITES - formal_origins {
                return Err("CFG loan site limit exceeded".into());
            }
            loans[index] = Some(LoanId(loan_sites.len()));
            loan_sites.push(LoanSite {
                kind: *kind,
                place: place.clone(),
                origin: Some(ExprId(index)),
                timing: LoanTiming::Immediate,
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
        facts,
        budget,
        resource_error: None,
        lowering_depth: 0,
        lowering_work: 0,
        origins: formal_parameter_origins + loan_sites.len(),
        loans,
        call_blocks: vec![None; source.exprs.len()],
        used_loans: std::collections::HashSet::new(),
        source,
        result: Function {
            lowering_proof: None,
            body_substitution: source.body_substitution.clone(),
            call_evidence: std::sync::Arc::from([]),
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
        drop_enabled,
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
    if let Some(error) = lowerer.resource_error.take() {
        return Err(error);
    }
    lowerer.terminate(Terminator::Return, None)?;
    for (index, expression) in source.exprs.iter().enumerate() {
        if let ir::ExprKind::Borrow {
            timing:
                ir::BorrowTiming::TwoPhase {
                    activation,
                    argument,
                    ..
                },
            ..
        } = &expression.kind
        {
            let loan = lowerer.loans[index].ok_or("missing two-phase loan site")?;
            let activation = lowerer
                .call_blocks
                .get(activation.0)
                .copied()
                .flatten()
                .ok_or("missing typed call activation block")?;
            lowerer.result.loan_sites[loan.0].timing = LoanTiming::TwoPhase {
                activation,
                argument: *argument,
            };
        }
    }
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
mod panic_terminal_tests {
    use super::*;
    use crate::compiler::{frontend, ownership};
    use std::sync::Arc;

    fn publish(
        text: &str,
        budget: &ir::PlanBudget,
    ) -> Arc<crate::compiler::source_replay::CanonicalSourcePublication> {
        let program = frontend::parse(text).unwrap();
        let plan = frontend::PlanCtx::from_program(&program, budget).unwrap();
        let frontend::PublishedCallableBody::Source(publication) = plan
            .publish_callable_body(plan.function_decl(0).unwrap(), budget)
            .unwrap()
        else {
            panic!("canonical source publication required")
        };
        publication
    }

    #[test]
    fn leading_zeros_source_and_cfg_reject_replayed_operand_or_operation() {
        let budget = ir::PlanBudget::default();
        let publication = publish(
            "fn f(x:u64,y:u64)->u32{x.leading_zeros()}fn main(){}",
            &budget,
        );
        let source = publication.source();
        let index = source
            .exprs
            .iter()
            .position(|expression| matches!(expression.kind, ir::ExprKind::LeadingZeros { .. }))
            .unwrap();
        let mut forged = source.as_ref().clone();
        let ir::ExprKind::LeadingZeros { value } = forged.exprs[index].kind else {
            unreachable!()
        };
        // Change the receiver read without changing either endpoint type.
        let ir::ExprKind::Local(local) = &mut forged.exprs[value.0].kind else {
            unreachable!()
        };
        *local = forged.params[1];
        assert!(
            publication
                .validate_source(&Arc::new(forged), publication.facts(), &budget)
                .is_err()
        );

        let function = lower_source_closed(&publication, &budget).unwrap();
        function
            .validate_provenance(publication.facts(), &budget)
            .unwrap();
        for change in 0..3 {
            let mut forged = function.clone();
            let statement = forged
                .blocks
                .iter_mut()
                .flat_map(|block| &mut block.statements)
                .find(|statement| {
                    matches!(
                        statement.value,
                        Statement::Assign {
                            value: Rvalue::LeadingZeros { .. },
                            ..
                        }
                    )
                })
                .unwrap();
            if change == 0 {
                let Statement::Assign {
                    value: Rvalue::LeadingZeros { value },
                    ..
                } = &mut statement.value
                else {
                    unreachable!()
                };
                *value = Operand::Copy(place(function.params[1]));
            } else if change == 1 {
                let Statement::Assign { value, .. } = &mut statement.value else {
                    unreachable!()
                };
                *value = Rvalue::Use(Operand::Constant(Constant::Int(IntValue {
                    ty: IntTy::U32,
                    bits: 0,
                })));
            } else {
                statement.origin = None;
            }
            assert!(
                forged
                    .validate_provenance(publication.facts(), &budget)
                    .is_err()
            );
        }
    }

    #[test]
    fn source_panic_requires_exact_never_even_in_unused_arena_entries() {
        let budget = ir::PlanBudget::default();
        let publication = publish("fn f()->i32{panic!(\"cold\")}fn main(){}", &budget);
        let source = publication.source();
        let index = source
            .exprs
            .iter()
            .position(|expression| matches!(expression.kind, ir::ExprKind::Panic(_)))
            .unwrap();
        assert_eq!(source.exprs[index].ty, Ty::Never);
        assert_eq!(source.exprs[index].completion, ir::Completion::Never);
        for mutate_location in [false, true] {
            let mut forged = source.as_ref().clone();
            let ir::ExprKind::Panic(payload) = &mut forged.exprs[index].kind else {
                unreachable!()
            };
            if mutate_location {
                payload.location.push('!');
            } else {
                payload.message.push('!');
            }
            assert!(
                publication
                    .validate_source(&Arc::new(forged), publication.facts(), &budget)
                    .is_err()
            );
        }
        let mut forged = source.as_ref().clone();
        forged.exprs[index].ty = Ty::Unit;
        assert!(
            forged
                .validate()
                .unwrap_err()
                .contains("panic requires exact Never")
        );
        forged = source.as_ref().clone();
        forged.exprs[index].completion = ir::Completion::Returns;
        assert!(
            forged
                .validate()
                .unwrap_err()
                .contains("panic requires exact Never")
        );
        forged = source.as_ref().clone();
        let mut unused = source.exprs[index].clone();
        unused.completion = ir::Completion::Returns;
        forged.exprs.push(unused);
        assert!(
            forged
                .validate()
                .unwrap_err()
                .contains("panic requires exact Never")
        );
    }

    #[test]
    fn source_panic_is_terminal_without_a_result_write_and_has_sealed_payload() {
        let budget = ir::PlanBudget::default();
        let publication = publish("fn f()->i32{panic!(\"cold\")}fn main(){}", &budget);
        let function = lower_source_closed(&publication, &budget).unwrap();
        function.validate().unwrap();
        assert!(ownership::analyze(&function).passed());
        let (block_index, block) = function
            .blocks
            .iter()
            .enumerate()
            .find(|(_, block)| matches!(block.terminator.value, Terminator::Panic(_)))
            .unwrap();
        let origin = block.terminator.origin.unwrap();
        let ir::ExprKind::Panic(expected) = &publication.source().exprs[origin.0].kind else {
            panic!("panic terminator must retain its panic source")
        };
        let Terminator::Panic(actual) = &block.terminator.value else {
            unreachable!()
        };
        assert_eq!(actual, expected);
        assert_eq!(actual.message, "cold");
        assert!(!actual.location.is_empty());
        assert!(!block.statements.iter().any(|statement| {
            statement.origin == Some(origin) && matches!(statement.value, Statement::Assign { .. })
        }));
        let mut visited_operands = 0;
        block
            .terminator
            .value
            .visit_operands(&mut |_| {
                visited_operands += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(visited_operands, 0);
        for change in 0..4 {
            let mut forged = function.clone();
            match change {
                0 => {
                    let Terminator::Panic(payload) =
                        &mut forged.blocks[block_index].terminator.value
                    else {
                        unreachable!()
                    };
                    payload.message.push('!');
                }
                1 => {
                    let Terminator::Panic(payload) =
                        &mut forged.blocks[block_index].terminator.value
                    else {
                        unreachable!()
                    };
                    payload.location.push('!');
                }
                2 => forged.blocks[block_index].terminator.origin = None,
                _ => forged.blocks[block_index].terminator.value = Terminator::Return,
            }
            assert!(
                forged
                    .validate_source_publication(&publication, &budget)
                    .is_err()
            );
        }
    }

    #[test]
    fn source_let_else_panic_proves_real_divergence_and_generic_bodies_remain_checkable() {
        let budget = ir::PlanBudget::default();
        let publication = publish(
            "enum E{A(i32),B}fn f(x:E)->i32{let E::A(value)=x else {panic!(\"no\")};value}fn main(){}",
            &budget,
        );
        let function = lower_source_closed(&publication, &budget).unwrap();
        assert!(ownership::analyze(&function).passed());
        let mut returning_else = publication.source().as_ref().clone();
        let panic_expression = returning_else
            .exprs
            .iter_mut()
            .find(|expression| matches!(expression.kind, ir::ExprKind::Panic(_)))
            .unwrap();
        // Preserve forged Never metadata while changing the actual operation to
        // a returning one. CFG reachability, not metadata, must reject this.
        panic_expression.kind = ir::ExprKind::Println("not divergent".into());
        assert!(
            lower_with_facts(&returning_else, publication.facts(), &budget)
                .unwrap_err()
                .contains("else branch can fall through")
        );
        assert_eq!(
            function
                .blocks
                .iter()
                .filter(|block| matches!(block.terminator.value, Terminator::Panic(_)))
                .count(),
            1
        );
        let budget = ir::PlanBudget::default();
        let publication = publish("fn f<T>(x:T)->T{panic!(\"generic\")}fn main(){}", &budget);
        let template = lower_source_template(&publication, &budget).unwrap();
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }

    #[test]
    fn repeated_source_panic_materialization_charges_full_payload_before_cloning() {
        let budget = ir::PlanBudget::default();
        let text = format!("fn f(){{panic!(\"{}\")}}fn main(){{}}", "x".repeat(4096));
        let publication = publish(&text, &budget);
        let function = lower_source_closed(&publication, &budget).unwrap();
        let source = publication.source();
        let id = ExprId(
            source
                .exprs
                .iter()
                .position(|expression| matches!(expression.kind, ir::ExprKind::Panic(_)))
                .unwrap(),
        );
        let ir::ExprKind::Panic(payload) = &source.exprs[id.0].kind else {
            unreachable!()
        };
        assert!(!payload.location.is_empty());
        let payload_bytes = std::mem::size_of::<ir::PanicMessage>()
            + payload.message.len()
            + payload.location.len();
        let statement_bytes = std::mem::size_of::<Located<Statement>>();
        let materialization_bytes =
            statement_bytes + payload_bytes + std::mem::size_of::<PendingBlock>();
        let mut lowerer = Lowerer {
            facts: ir::FactsCtx::from_validated(publication.facts()),
            budget: Some(&budget),
            resource_error: None,
            lowering_depth: 0,
            lowering_work: 0,
            origins: 0,
            loans: vec![None; source.exprs.len()],
            call_blocks: vec![None; source.exprs.len()],
            used_loans: Default::default(),
            source,
            result: function,
            expressions: (0..source.exprs.len())
                .map(|index| LocalId(source.locals.len() + index))
                .collect(),
            parameters: Default::default(),
            blocks: vec![PendingBlock {
                statements: Vec::new(),
                terminator: None,
            }],
            current: BlockId(0),
            drop_enabled: false,
            scopes: vec![Vec::new()],
            loops: Vec::new(),
        };
        assert!(publication.facts().budget().same_authority(&budget));
        assert!(lowerer.budget.unwrap().same_authority(&budget));
        // Tighten the original publication ledger, not a fresh replacement.
        // Two complete copies fit. The third falls one byte short of the full
        // boxed payload, after its StorageLive but before any payload clone.
        let remaining = 2 * materialization_bytes + statement_bytes + payload_bytes - 1;
        budget
            .retain_bytes(budget.limits().bytes - budget.usage_snapshot().bytes - remaining)
            .unwrap();
        for index in 0..2 {
            let before = budget.usage_snapshot().bytes;
            lowerer.materialize_expression(id).unwrap();
            assert_eq!(
                budget.usage_snapshot().bytes - before,
                materialization_bytes
            );
            let terminator = lowerer.blocks[index].terminator.as_ref().unwrap();
            assert_eq!(terminator.origin, Some(id));
            let Terminator::Panic(actual) = &terminator.value else {
                panic!("each occurrence must materialize its own panic terminator")
            };
            assert_eq!(actual, payload);
            assert!(!std::ptr::eq(actual.as_ref(), payload.as_ref()));
        }
        let Terminator::Panic(first) = &lowerer.blocks[0].terminator.as_ref().unwrap().value else {
            unreachable!()
        };
        let Terminator::Panic(second) = &lowerer.blocks[1].terminator.as_ref().unwrap().value
        else {
            unreachable!()
        };
        assert!(!std::ptr::eq(first.as_ref(), second.as_ref()));
        let error = lowerer.materialize_expression(id).unwrap_err();
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Bytes);
        assert_eq!(failure.requested, payload_bytes);
        assert_eq!(failure.used, budget.usage_snapshot().bytes);
        assert_eq!(failure.limit - failure.used, payload_bytes - 1);
        assert!(lowerer.blocks[2].terminator.is_none());
        assert_eq!(lowerer.blocks.len(), 3);
        let after_failure = budget.usage_snapshot().bytes;
        assert_eq!(lowerer.materialize_expression(id).unwrap_err(), error);
        assert_eq!(budget.usage_snapshot().bytes, after_failure);
        assert_eq!(budget.first_resource_failure(), Some(failure));
    }

    #[test]
    fn panic_payload_storage_is_budgeted_in_ir_and_cfg_clones() {
        let budget = ir::PlanBudget::default();
        let publication = publish("fn f(){panic!(\"cold\")}fn main(){}", &budget);
        let source = publication.source();
        let mut larger = source.as_ref().clone();
        let ir::ExprKind::Panic(payload) = &mut larger
            .exprs
            .iter_mut()
            .find(|expression| matches!(expression.kind, ir::ExprKind::Panic(_)))
            .unwrap()
            .kind
        else {
            unreachable!()
        };
        payload.message.push_str(&"x".repeat(4096));
        payload.location.push_str(&"y".repeat(2048));
        let small_budget = ir::PlanBudget::default();
        source
            .preflight_plan(ir::FactsCtx::Concrete, &small_budget)
            .unwrap();
        let large_budget = ir::PlanBudget::default();
        larger
            .preflight_plan(ir::FactsCtx::Concrete, &large_budget)
            .unwrap();
        assert_eq!(
            large_budget.usage_snapshot().bytes - small_budget.usage_snapshot().bytes,
            6144
        );
        let function = lower_source_closed(&publication, &budget).unwrap();
        let mut larger = function.clone();
        let Terminator::Panic(payload) = &mut larger
            .blocks
            .iter_mut()
            .find(|block| matches!(block.terminator.value, Terminator::Panic(_)))
            .unwrap()
            .terminator
            .value
        else {
            unreachable!()
        };
        payload.message.push_str(&"x".repeat(4096));
        payload.location.push_str(&"y".repeat(2048));
        let original_bytes = function
            .snapshot_storage(&ir::PlanBudget::default())
            .unwrap();
        let enlarged_bytes = larger.snapshot_storage(&ir::PlanBudget::default()).unwrap();
        assert_eq!(enlarged_bytes - original_bytes, 6144);
    }
}

#[cfg(test)]
mod original_replay_transport_tests {
    use super::*;
    use crate::compiler::{frontend, ownership};
    use std::sync::Arc;

    fn published(
        plan: &frontend::PlanCtx,
        index: usize,
        budget: &ir::PlanBudget,
    ) -> Arc<crate::compiler::source_replay::CanonicalSourcePublication> {
        let declaration = plan.function_decl(index).unwrap();
        let frontend::PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, budget).unwrap()
        else {
            panic!("canonical source publication required")
        };
        publication
    }

    #[test]
    fn original_source_zero_call_missing_and_transplanted_publication_fences() {
        let budget = ir::PlanBudget::default();
        let program = frontend::parse("fn empty(){} fn other(){} fn main(){}").unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let publication = published(&plan, 0, &budget);
        let other = published(&plan, 1, &budget);
        let function = lower_source_closed(&publication, &budget).unwrap();
        assert!(function.call_evidence.is_empty());
        assert!(function.lowering_proof.is_some());
        function
            .validate_source_publication(&publication, &budget)
            .unwrap();
        assert!(
            function
                .validate_source_publication(&other, &budget)
                .is_err()
        );
        let raw = lower_with_facts(publication.source(), publication.facts(), &budget).unwrap();
        assert!(raw.lowering_proof.is_none());
        assert!(
            raw.validate_source_publication(&publication, &budget)
                .is_err()
        );
        let mut detached = function.clone();
        detached.lowering_proof = None;
        assert!(
            detached
                .validate_source_publication(&publication, &budget)
                .is_err()
        );
        let other_function = lower_source_closed(&other, &budget).unwrap();
        let mut transplanted = function.clone();
        transplanted.lowering_proof = other_function.lowering_proof.clone();
        assert!(
            transplanted
                .validate_source_publication(&publication, &budget)
                .is_err()
        );
        assert!(
            transplanted
                .validate_source_publication(&other, &budget)
                .is_err()
        );
        let republished = published(&plan, 0, &budget);
        assert!(!Arc::ptr_eq(&publication, &republished));
        assert!(
            function
                .validate_source_publication(&republished, &budget)
                .is_err()
        );
        assert!(lower_source_template(&publication, &budget).is_err());
    }

    #[test]
    fn original_source_generic_exact_template_phase_bridge() {
        let budget = ir::PlanBudget::default();
        let program = frontend::parse(
            "fn id<T:Copy>(x:T)->T{x} fn forward<T:Copy>(x:T)->T{id::<T>(x)} fn main(){}",
        )
        .unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let publication = published(&plan, 1, &budget);
        assert_eq!(
            publication.facts().phase(),
            crate::compiler::traits::FactsPhase::Universal
        );
        assert!(lower_source_closed(&publication, &budget).is_err());
        let lowered = lower_source_template(&publication, &budget).unwrap();
        assert!(Arc::ptr_eq(lowered.source(), publication.source()));
        lowered
            .function()
            .validate_source_publication(&publication, &budget)
            .unwrap();
        let verified = ownership::verify_template(lowered).unwrap();
        let target = ir::CallTarget {
            declaration: plan.function_decl(1).unwrap(),
            type_args: Arc::from([Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(Ty::Int(IntTy::I32)),
            }]),
        };
        let facts = plan.instance_facts(&target).unwrap();
        assert!(
            publication
                .validate_source(publication.source(), &facts, &budget)
                .is_err()
        );
        let instance = ir::instantiate_checked(
            &verified,
            &target.type_args,
            &facts,
            &mut plan.clone(),
            &budget,
        )
        .unwrap();
        let function = lower_with_facts(&instance, &facts, &budget).unwrap();
        function.validate_provenance(&facts, &budget).unwrap();
        assert!(
            function
                .lowering_proof
                .as_ref()
                .unwrap()
                .original_publication
                .is_none()
        );
        let proof = function.body_substitution.as_ref().unwrap();
        assert!(Arc::ptr_eq(proof.template().source(), publication.source()));
        proof
            .template()
            .function()
            .validate_source_publication(&publication, &budget)
            .unwrap();
        assert_eq!(function.call_evidence.len(), 1);
        assert!(Arc::ptr_eq(
            &function.call_evidence[0],
            &verified.function().call_evidence[0]
        ));
        assert!(Arc::ptr_eq(
            function.call_evidence[0].original().unwrap(),
            publication.replay()
        ));
        // A closed specialization is not reclassified as a fresh original call.
        assert_eq!(
            function.call_evidence[0].trace().arguments[0].expectation,
            ir::CallExpectation::NoFirmReference
        );
        assert!(
            function
                .validate_source_publication(&publication, &budget)
                .is_err()
        );
        let mut detached = function;
        detached.body_substitution = None;
        assert!(detached.validate_provenance(&facts, &budget).is_err());
    }
}

#[cfg(test)]
mod tests {

    // Retain the real checked source-name owner alongside its module arena.
    fn fixture_modules() -> (
        std::sync::Arc<super::super::semantic_name::NameStore>,
        std::sync::Arc<super::super::module_identity::ModuleArena>,
    ) {
        use super::super::expansion_budget::ExpansionBudget;
        use super::super::module_identity::ModuleArenaBuilder;
        use super::super::semantic_name::NameStoreBuilder;
        use super::super::token::Edition;
        use super::super::token_session::{SourceSnapshot, TokenSession};
        let budget = ExpansionBudget::default();
        let sources =
            SourceSnapshot::from_text(std::path::Path::new("module-fixture.rs"), "", &budget)
                .unwrap();
        let session = TokenSession::new(sources, budget).unwrap();
        let names = NameStoreBuilder::new(session, Edition::E2024).unwrap();
        let modules = ModuleArenaBuilder::for_source(&names)
            .unwrap()
            .freeze()
            .unwrap();
        (names.freeze().unwrap(), modules)
    }
    #[test]
    fn leading_zeros_survives_substitution_and_rejects_generated_replay() {
        use crate::compiler::{frontend, ownership};
        let budget = ir::PlanBudget::default();
        let program =
            frontend::parse("fn count<T>(unused:T,x:u64)->u32{x.leading_zeros()}fn main(){}")
                .unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let source =
            frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
        let template = ownership::verify_template(
            lower_template(
                std::sync::Arc::new(source),
                plan.callable_facts(declaration).unwrap(),
                &budget,
            )
            .unwrap(),
        )
        .unwrap();
        let target = ir::CallTarget {
            declaration,
            type_args: std::sync::Arc::from([Ty::Int(IntTy::I32)]),
        };
        let facts = plan.instance_facts(&target).unwrap();
        let instance = ir::instantiate_checked(
            &template,
            &target.type_args,
            &facts,
            &mut plan.clone(),
            &budget,
        )
        .unwrap();
        instance
            .validate_with_facts(ir::FactsCtx::from_validated(&facts))
            .unwrap();
        let function = lower_with_facts(&instance, &facts, &budget).unwrap();
        function.validate_provenance(&facts, &budget).unwrap();
        let mut forged = instance;
        let operation = forged
            .exprs
            .iter_mut()
            .find(|expression| matches!(expression.kind, ir::ExprKind::LeadingZeros { .. }))
            .unwrap();
        operation.kind = ir::ExprKind::Int(IntValue {
            ty: IntTy::U32,
            bits: 0,
        });
        assert!(
            forged
                .validate_with_facts(ir::FactsCtx::from_validated(&facts))
                .is_err()
        );
    }

    #[test]
    fn generic_carrier_generated_body_and_lowering_proofs_reject_metadata_replay() {
        use crate::compiler::{frontend, ownership};
        let budget = ir::PlanBudget::default();
        let program = frontend::parse(
            "fn id<T>(x:T,y:T)->T{x} fn forward<T>(x:T,y:T)->T{id::<T>(x,y)} fn main(){}",
        )
        .unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let source =
            frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
        let universal = plan.callable_facts(declaration).unwrap();
        let template = ownership::verify_template(
            lower_template(std::sync::Arc::new(source), universal, &budget).unwrap(),
        )
        .unwrap();
        let target = ir::CallTarget {
            declaration,
            type_args: std::sync::Arc::from([Ty::Int(IntTy::I32)]),
        };
        let facts = plan.instance_facts(&target).unwrap();
        let instance = ir::instantiate_checked(
            &template,
            &target.type_args,
            &facts,
            &mut plan.clone(),
            &budget,
        )
        .unwrap();
        instance
            .validate_with_facts(ir::FactsCtx::from_validated(&facts))
            .unwrap();
        let mut replay = instance.clone();
        let call = replay
            .exprs
            .iter_mut()
            .find(|expression| matches!(expression.kind, ir::ExprKind::Call { .. }))
            .unwrap();
        let ir::ExprKind::Call {
            target: ir::Callee::Direct(target),
            args,
            ..
        } = &mut call.kind
        else {
            panic!("direct call fixture");
        };
        target.declaration = declaration;
        args.swap(0, 1);
        assert!(
            replay
                .validate_with_facts(ir::FactsCtx::from_validated(&facts))
                .is_err()
        );
        assert!(lower_with_facts(&replay, &facts, &budget).is_err());
        let function = lower_with_facts(&instance, &facts, &budget).unwrap();
        function.validate_provenance(&facts, &budget).unwrap();
        let shape = function.lowering_proof.as_ref().unwrap();
        assert!(std::sync::Arc::ptr_eq(&shape.world, facts.world()));
        assert!(std::sync::Arc::ptr_eq(
            shape.body_proof.as_ref().unwrap(),
            function.body_substitution.as_ref().unwrap()
        ));
        assert!(
            shape
                .capsules
                .iter()
                .zip(function.call_evidence.iter())
                .all(|(original, current)| std::sync::Arc::ptr_eq(original, current))
        );
        let mut replay = function.clone();
        let call = replay
            .blocks
            .iter_mut()
            .find(|block| matches!(block.terminator.value, Terminator::Call { .. }))
            .unwrap();
        let Terminator::Call {
            target: ir::Callee::Direct(target),
            args,
            ..
        } = &mut call.terminator.value
        else {
            panic!("direct lowered fixture");
        };
        target.declaration = declaration;
        args.swap(0, 1);
        assert!(replay.validate_provenance(&facts, &budget).is_err());
        assert!(close(std::sync::Arc::new(replay), facts.clone(), &budget).is_err());
        let mut detached = function;
        detached.lowering_proof = None;
        assert!(detached.validate_provenance(&facts, &budget).is_err());
    }

    #[test]
    fn nested_shared_carriers_lower_typed_dereferences_and_bound_formal_origins() {
        let scalar = Ty::Int(IntTy::I32);
        let shared = |referent: Ty| Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: std::sync::Arc::new(referent),
        };
        let inner = shared(scalar.clone());
        let outer = shared(inner.clone());
        let mut source = with_root(
            outer.clone(),
            vec![
                expr(outer, ir::ExprKind::Local(LocalId(0))),
                expr(inner.clone(), ir::ExprKind::Deref { value: ExprId(0) }),
                expr(scalar.clone(), ir::ExprKind::Deref { value: ExprId(1) }),
            ],
            vec![ir::Stmt::Expr(ExprId(1)), ir::Stmt::Expr(ExprId(2))],
        );
        source.ref_contract = ir::normalize_ref_contract(
            &ir::RefContract {
                region_count: 2,
                input_regions: (0..2)
                    .map(|depth| ir::InputRegion {
                        input: ir::InputReference {
                            param: 0,
                            position: ir::ReferencePosition { depth },
                        },
                        region: ir::RegionParamId(depth.into()),
                    })
                    .collect::<Vec<_>>()
                    .into(),
                results: std::sync::Arc::from([]),
                outlives: std::sync::Arc::from([]),
            },
            &ir::PlanBudget::default(),
        )
        .unwrap();
        let function = nominal_lowered(&source);
        assert_eq!(
            function
                .place_ty(&Place {
                    local: LocalId(0),
                    projection: vec![Projection::Deref]
                })
                .unwrap(),
            &inner
        );
        assert_eq!(
            function
                .place_ty(&Place {
                    local: LocalId(0),
                    projection: vec![Projection::Deref, Projection::Deref]
                })
                .unwrap(),
            &scalar
        );
        let mut oversized = source;
        oversized.params = vec![LocalId(0); ir::MAX_LOAN_SITES / 2 + 1];
        assert!(lower(&oversized).is_err());
        assert!(validate_type(&shared(shared(shared(scalar))), 0).is_err());
    }

    fn template_facts(
        modules: &std::sync::Arc<super::super::module_identity::ModuleArena>,
        source: &ir::CheckedFunction,
        env: std::sync::Arc<ir::GenericEnv>,
        extra: &[super::super::traits::CallableHeader],
        budget: &ir::PlanBudget,
    ) -> std::sync::Arc<super::super::traits::ValidatedFacts> {
        let declaration = env.scopes().last().unwrap().binder().declaration;
        let mut headers = extra.to_vec();
        headers.push(super::super::traits::CallableHeader {
            declaration,
            schema: super::super::traits::GenericSchema {
                env,
                signature: ir::Signature {
                    ref_contract: source.ref_contract.clone(),
                    params: source
                        .params
                        .iter()
                        .map(|id| source.locals[id.0].ty.clone())
                        .collect(),
                    result: source.result.clone(),
                },
                predicates: std::sync::Arc::from([]),
            },
        });
        super::super::traits::test_support::world(modules.clone(), &headers, budget)
            .unwrap()
            .callable_facts(declaration)
            .unwrap()
    }
    fn instance_facts(
        facts: &super::super::traits::ValidatedFacts,
        args: &[Ty],
    ) -> std::sync::Arc<super::super::traits::ValidatedFacts> {
        facts
            .world()
            .instance_facts(&ir::CallTarget {
                declaration: facts.declaration(),
                type_args: args.into(),
            })
            .unwrap()
    }
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
        let (_module_names, modules) = fixture_modules();
        let budget = ir::PlanBudget::default();
        let scopes = [test_decl(), test_decl()].map(|declaration| {
            std::sync::Arc::new(ir::ParamEnv::new(ir::BinderId { declaration }, 1).unwrap())
        });
        let env = std::sync::Arc::new(ir::GenericEnv::new(&scopes, &budget).unwrap());
        let types = env.ordered_params().map(Ty::Param).collect::<Vec<_>>();
        let result = Ty::Tuple(types.clone().into());
        let source = ir::CheckedFunction {
            call_evidence: std::sync::Arc::from([]),
            body_substitution: None,
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
        let authority = template_facts(&modules, &source, env.clone(), &[], &budget);
        let template = lower_template(
            std::sync::Arc::new(source.clone()),
            authority.clone(),
            &budget,
        )
        .unwrap();
        assert!(std::sync::Arc::ptr_eq(template.env(), &env));
        let verified = super::super::ownership::verify_template(template).unwrap();
        let closed_facts = instance_facts(&authority, &[Ty::Int(IntTy::I32), Ty::Bool]);
        let instance = ir::instantiate_checked(
            &verified,
            &[Ty::Int(IntTy::I32), Ty::Bool],
            &closed_facts,
            &mut NoNominals,
            &budget,
        )
        .unwrap();
        assert_eq!(
            instance.result,
            Ty::Tuple(vec![Ty::Int(IntTy::I32), Ty::Bool].into())
        );
        let function = lower_with_facts(&instance, &closed_facts, &budget).unwrap();
        assert!(
            super::super::ownership::analyze_closed(
                close(std::sync::Arc::new(function), closed_facts.clone(), &budget).unwrap()
            )
            .is_ok()
        );
        let incomplete = std::sync::Arc::new(ir::GenericEnv::new(&scopes[..1], &budget).unwrap());
        assert!(
            source
                .validate_with_facts(ir::FactsCtx::Universal(&incomplete))
                .is_err()
        );
        let mut duplicate = source;
        duplicate.exprs[1] = expr(types[0].clone(), ir::ExprKind::Local(LocalId(0)));
        duplicate.result = Ty::Tuple(vec![types[0].clone(), types[0].clone()].into());
        duplicate.exprs[2].ty = duplicate.result.clone();
        let duplicate_facts = template_facts(&modules, &duplicate, env, &[], &budget);
        assert!(
            super::super::ownership::verify_template(
                lower_template(std::sync::Arc::new(duplicate), duplicate_facts, &budget).unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn generic_templates_prove_moves_before_relowering_concrete_copy_instances() {
        let (_module_names, modules) = fixture_modules();
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
            call_evidence: std::sync::Arc::from([]),
            body_substitution: None,
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
        let authority = template_facts(&modules, &source, env.clone(), &[], &budget);
        let template = lower_template(
            std::sync::Arc::new(source.clone()),
            authority.clone(),
            &budget,
        )
        .unwrap();
        assert!(template.function().blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value,Statement::Assign { value:Rvalue::Use(Operand::Move(place)),.. } if place.local==LocalId(0))));
        let verified = super::super::ownership::verify_template(template).unwrap();
        let closed_facts = instance_facts(&authority, &[Ty::Int(IntTy::I32)]);
        assert!(
            ir::instantiate_checked(
                &verified,
                &[Ty::Int(IntTy::I32)],
                &closed_facts,
                &mut NoNominals,
                &ir::PlanBudget::default()
            )
            .is_err()
        );
        let instance = ir::instantiate_checked(
            &verified,
            &[Ty::Int(IntTy::I32)],
            &closed_facts,
            &mut NoNominals,
            &budget,
        )
        .unwrap();
        let concrete = lower_with_facts(&instance, &closed_facts, &budget).unwrap();
        assert!(concrete.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value,Statement::Assign { value:Rvalue::Use(Operand::Copy(place)),.. } if place.local==LocalId(0))));
        assert!(
            super::super::ownership::analyze_closed(
                close(std::sync::Arc::new(concrete), closed_facts.clone(), &budget).unwrap()
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
        let duplicate_facts = template_facts(&modules, &duplicate, env, &[], &budget);
        let invalid =
            lower_template(std::sync::Arc::new(duplicate), duplicate_facts, &budget).unwrap();
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
                module: modules.source_root().unwrap(),
                visibility: crate::compiler::ir::Visibility::Restricted(
                    modules.source_root().unwrap(),
                ),
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
            lowering_proof: None,
            body_substitution: None,
            call_evidence: std::sync::Arc::from([]),
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
        assert!(close(std::sync::Arc::new(malformed), closed_facts, &budget).is_err());
    }
    #[test]
    fn generic_instantiation_reinterns_variant_owners_and_resolved_call_metadata() {
        use super::super::frontend;
        let budget = ir::PlanBudget::default();
        let program = frontend::parse("enum E<T>{A(T)} fn identity<T>(x:T)->T{x} fn extract<T>(e:E<T>)->T{match e{E::A(x)=>identity::<T>(x)}} fn main(){}").unwrap();
        let mut plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = (0..plan.callable_count())
            .map(|i| plan.callable_decl(i).unwrap())
            .find(|decl| {
                let path = &plan.callable_body(&program, *decl).unwrap().name;
                path.anchor() == super::super::semantic_path::PathAnchor::Crate
                    && path.len() == 1
                    && program.names.spelling(path.segments()[0]).unwrap() == "extract"
            })
            .unwrap();
        let source =
            frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
        let Ty::Enum(symbolic) = source.locals[source.params[0].0].ty.clone() else {
            panic!("enum parameter");
        };
        let authority = plan.callable_facts(declaration).unwrap();
        let closed = plan
            .instance_facts(&ir::CallTarget {
                declaration,
                type_args: std::sync::Arc::from([Ty::Int(IntTy::I32)]),
            })
            .unwrap();
        let verified = super::super::ownership::verify_template(
            lower_template(std::sync::Arc::new(source), authority, &budget).unwrap(),
        )
        .unwrap();
        let mut instance = ir::instantiate_checked(
            &verified,
            &[Ty::Int(IntTy::I32)],
            &closed,
            &mut plan,
            &budget,
        )
        .unwrap();
        let Ty::Enum(concrete) = instance.locals[instance.params[0].0].ty.clone() else {
            panic!("enum parameter");
        };
        assert_ne!(symbolic.def().id, concrete.def().id);
        let (target, signature) = instance
            .exprs
            .iter()
            .find_map(|e| {
                if let ir::ExprKind::Call {
                    target, signature, ..
                } = &e.kind
                {
                    Some((target, signature))
                } else {
                    None
                }
            })
            .unwrap();
        let ir::Callee::Direct(target) = target else {
            panic!("closed direct call");
        };
        assert_eq!(target.type_args.as_ref(), [Ty::Int(IntTy::I32)]);
        assert_eq!(signature.result, Ty::Int(IntTy::I32));
        let (ty, bindings) = instance
            .exprs
            .iter()
            .find_map(|e| {
                if let ir::ExprKind::Match { arms, .. } = &e.kind {
                    if let ir::MatchPattern::EnumVariant { ty, bindings, .. } = &arms[0].pattern {
                        Some((ty, bindings))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(ty.def().id, concrete.def().id);
        assert_eq!(
            bindings[0].projection[0],
            Projection::Downcast(ir::VariantId {
                owner: concrete.def().id,
                index: 0
            })
        );
        assert!(
            super::super::ownership::analyze_closed(
                close(
                    std::sync::Arc::new(lower_with_facts(&instance, &closed, &budget).unwrap()),
                    closed,
                    &budget
                )
                .unwrap()
            )
            .is_ok()
        );
        for expression in &mut instance.exprs {
            if let ir::ExprKind::Match { arms, .. } = &mut expression.kind
                && let ir::MatchPattern::EnumVariant { bindings, .. } = &mut arms[0].pattern
            {
                bindings[0].projection[0] = Projection::Downcast(ir::VariantId {
                    owner: symbolic.def().id,
                    index: 0,
                });
            }
        }
        assert!(instance.validate().is_err());
    }
    fn test_decl() -> ir::DeclId {
        static SNAPSHOT: std::sync::OnceLock<ir::SnapshotId> = std::sync::OnceLock::new();
        static INDEX: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        super::super::traits::test_support::source_decl(
            *SNAPSHOT.get_or_init(|| ir::SnapshotId::allocate().unwrap()),
            INDEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            &ir::PlanBudget::default(),
        )
        .unwrap()
    }
    fn test_target() -> ir::Callee {
        ir::Callee::Direct(ir::CallTarget {
            declaration: test_decl(),
            type_args: std::sync::Arc::from([]),
        })
    }

    #[test]
    fn canonical_copy_lowering_uses_exact_bounds_and_explicit_moves_still_consume() {
        use super::super::frontend;
        for (text, copy) in [
            (
                "fn dup<T:core::marker::Copy>(x:T)->(T,T){(x,x)} fn main(){}",
                true,
            ),
            (
                "trait Copy{} fn dup<T:Copy>(x:T)->(T,T){(x,x)} fn main(){}",
                false,
            ),
        ] {
            let program = frontend::parse(text).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.callable_decl(0).unwrap();
            let source =
                frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
            let facts = plan.callable_facts(declaration).unwrap();
            let template = lower_template(std::sync::Arc::new(source), facts, &budget).unwrap();
            let parameter = template.function.params[0];
            let copies = template
                .function
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .filter(|statement| {
                    matches!(&statement.value, Statement::Assign {
                    value: Rvalue::Use(Operand::Copy(place)), ..
                } if place.local == parameter && place.projection.is_empty())
                })
                .count();
            assert_eq!(copies, if copy { 2 } else { 0 });
            assert_eq!(
                super::super::ownership::verify_template(template.clone()).is_ok(),
                copy
            );
            if copy {
                let mut consuming = template;
                for block in &mut std::sync::Arc::make_mut(&mut consuming.function).blocks {
                    for statement in &mut block.statements {
                        if let Statement::Assign {
                            value: Rvalue::Use(operand),
                            ..
                        } = &mut statement.value
                            && let Operand::Copy(place) = operand
                            && place.local == parameter
                            && place.projection.is_empty()
                        {
                            *operand = Operand::Move(place.clone());
                        }
                    }
                }
                assert!(super::super::ownership::verify_template(consuming).is_err());
            }
        }
    }
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generic_reference_two_phase_protocol_rejects_aliases_and_keeps_dead_activation() {
        use super::super::frontend;
        for argument in ["*r", "loop{}"] {
            let text = format!(
                "fn sink(x:&mut i32,n:i32){{*x=n;}} fn f(r:&mut i32){{sink(r,{argument});}} fn main(){{}}"
            );
            let program = frontend::parse(&text).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.callable_decl(1).unwrap();
            let source =
                frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
            let facts = plan.callable_facts(declaration).unwrap();
            let function =
                lower_internal(&source, ir::FactsCtx::from_validated(&facts), Some(&budget))
                    .unwrap();
            function
                .validate_with_facts(ir::FactsCtx::from_validated(&facts))
                .unwrap();
            let (site, activation, argument) = function
                .loan_sites
                .iter()
                .enumerate()
                .find_map(|(index, site)| {
                    if let LoanTiming::TwoPhase {
                        activation,
                        argument,
                    } = site.timing
                    {
                        Some((index, activation, argument))
                    } else {
                        None
                    }
                })
                .expect("implicit mutable argument reborrow reserves a loan");
            let Terminator::Call { args, .. } = &function.blocks[activation.0].terminator.value
            else {
                panic!("activation is a real typed call");
            };
            let Operand::Move(carrier) = &args[argument] else {
                panic!("activation moves unique carrier");
            };
            let mut aliased = function.clone();
            aliased.blocks[activation.0].statements.push(Located {
                origin: None,
                value: Statement::Assign {
                    destination: carrier.clone(),
                    value: Rvalue::Use(Operand::Move(carrier.clone())),
                },
            });
            assert!(
                aliased
                    .validate_with_facts(ir::FactsCtx::from_validated(&facts))
                    .is_err()
            );
            let mut invalid_slot = function.clone();
            invalid_slot.loan_sites[site].timing = LoanTiming::TwoPhase {
                activation,
                argument: usize::MAX,
            };
            assert!(
                invalid_slot
                    .validate_with_facts(ir::FactsCtx::from_validated(&facts))
                    .is_err()
            );
        }
    }

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
            callee.ref_contract.results[0].returns,
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
        std::sync::Arc::make_mut(&mut contract.results)[0].returns =
            ir::ReturnContract::SharedInputPaths(std::sync::Arc::from([ir::InputPath {
                input: ir::InputReference {
                    param: 0,
                    position: ir::ReferencePosition { depth: 0 },
                },
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
            lowered.ref_contract.results[0].returns,
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
                            timing: ir::BorrowTiming::Immediate,
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
                    referent: std::sync::Arc::new(Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: std::sync::Arc::new(reference.clone()),
                    }),
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
            timing: LoanTiming::Immediate,
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
                        timing: ir::BorrowTiming::Immediate,
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

    fn nominal(
        modules: &std::sync::Arc<super::super::module_identity::ModuleArena>,
        fields: Vec<Ty>,
    ) -> ir::StructTy {
        ir::StructTy::new(ir::StructDef {
            source_decl: test_decl(),
            type_args: std::sync::Arc::from([]),
            id: ir::NominalAppId {
                snapshot: ir::SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "S".into(),
            module: modules.source_root().unwrap(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.source_root().unwrap()),
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, ty)| ir::FieldDef {
                    name: format!("f{index}"),
                    ty,
                    visibility: crate::compiler::ir::Visibility::Restricted(
                        modules.source_root().unwrap(),
                    ),
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
            call_evidence: std::sync::Arc::from([]),
            body_substitution: None,
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
    fn dead_indexed_compounds_require_every_reference_hop_mutable() {
        let array = Ty::Array {
            element: std::sync::Arc::new(Ty::Int(IntTy::U128)),
            len: 2,
        };
        let reference = |kind, referent| Ty::Ref {
            kind,
            referent: std::sync::Arc::new(referent),
        };
        let mutable = reference(ir::BorrowKind::Mut, array.clone());
        let shared = reference(ir::BorrowKind::Shared, array.clone());
        for (root, projection, accepted) in [
            (mutable.clone(), vec![Projection::Deref], true),
            (shared.clone(), vec![Projection::Deref], false),
            (
                Ty::Tuple(vec![mutable.clone(), Ty::Bool].into()),
                vec![Projection::Field(0), Projection::Deref],
                true,
            ),
            (
                Ty::Tuple(vec![shared, Ty::Bool].into()),
                vec![Projection::Field(0), Projection::Deref],
                false,
            ),
            (
                reference(ir::BorrowKind::Shared, mutable.clone()),
                vec![Projection::Deref, Projection::Deref],
                false,
            ),
            (
                reference(ir::BorrowKind::Mut, mutable),
                vec![Projection::Deref, Projection::Deref],
                true,
            ),
            (array, vec![], false),
        ] {
            let mut source = with_root(root.clone(), vec![], vec![]);
            source.locals[0].mutable = false;
            let mut function = nominal_lowered(&with_root(Ty::Unit, vec![], vec![]));
            function.locals[0].ty = root.clone();
            function.locals[0].mutable = false;
            let target = Place {
                local: LocalId(0),
                projection: projection.clone(),
            };
            assert_eq!(
                function.indexed_mutation_ty(&target).is_ok(),
                accepted,
                "metadata {projection:?}"
            );
            function.blocks.push(BasicBlock {
                statements: vec![Located {
                    value: Statement::IndexAssignOp {
                        array: Place {
                            local: LocalId(0),
                            projection: projection.clone(),
                        },
                        index: Operand::Constant(Constant::Int(IntValue::usize(0))),
                        value: Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::U128,
                            bits: 1,
                        })),
                        op: BinaryOp::Add,
                    },
                    origin: None,
                }],
                terminator: Located {
                    value: Terminator::Unreachable,
                    origin: None,
                },
            });
            // The facts-free legacy verifier rejects aggregate and mutable
            // nested carriers independently. Exercise its all-block path only
            // for supported roots, and the exact mutation guard above for all.
            if validate_type(&root, 0).is_ok() {
                assert_eq!(function.validate().is_ok(), accepted, "{projection:?}");
            }
            source.exprs = vec![
                expr(Ty::Int(IntTy::Usize), ir::ExprKind::Int(IntValue::usize(0))),
                expr(
                    Ty::Int(IntTy::U128),
                    ir::ExprKind::Int(IntValue {
                        ty: IntTy::U128,
                        bits: 1,
                    }),
                ),
            ];
            source.body.statements = vec![ir::Stmt::AssignIndexOp {
                array: Place {
                    local: LocalId(0),
                    projection: projection.clone(),
                },
                index: ExprId(0),
                value: ExprId(1),
                op: BinaryOp::Add,
            }];
            if !matches!(&root, Ty::Ref { referent, .. } if matches!(&**referent, Ty::Ref { .. })) {
                assert_eq!(source.validate().is_ok(), accepted, "IR {projection:?}");
            }
        }
    }

    #[test]
    fn reference_array_compounds_preserve_projection_and_reject_malformed_dead_updates() {
        for integer in [IntTy::U8, IntTy::I128, IntTy::U128] {
            let source = with_root(
                Ty::Ref {
                    kind: ir::BorrowKind::Mut,
                    referent: std::sync::Arc::new(Ty::Array {
                        element: std::sync::Arc::new(Ty::Int(integer)),
                        len: 2,
                    }),
                },
                vec![
                    expr(Ty::Int(IntTy::Usize), ir::ExprKind::Int(IntValue::usize(0))),
                    expr(
                        Ty::Int(IntTy::U128),
                        ir::ExprKind::Int(IntValue {
                            ty: IntTy::U128,
                            bits: 1,
                        }),
                    ),
                ],
                vec![ir::Stmt::AssignIndexOp {
                    array: Place {
                        local: LocalId(0),
                        projection: vec![Projection::Deref],
                    },
                    index: ExprId(0),
                    value: ExprId(1),
                    op: BinaryOp::Shl,
                }],
            );
            let function = nominal_lowered(&source);
            let update = function
                .blocks
                .iter()
                .flat_map(|b| &b.statements)
                .find(|s| matches!(s.value, Statement::IndexAssignOp { .. }))
                .unwrap()
                .clone();
            let Statement::IndexAssignOp { array, .. } = &update.value else {
                unreachable!()
            };
            assert_eq!(array.projection, [Projection::Deref]);
            for case in 0..4 {
                let mut invalid = function.clone();
                let mut bad = update.clone();
                let Statement::IndexAssignOp {
                    array,
                    index,
                    value,
                    op,
                } = &mut bad.value
                else {
                    unreachable!()
                };
                match case {
                    0 => *index = Operand::Constant(Constant::Bool(false)),
                    1 => {
                        *value = Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::U64,
                            bits: 1_u128 << 64,
                        }))
                    }
                    2 => *op = BinaryOp::Eq,
                    _ => array.projection.push(Projection::Field(99)),
                }
                invalid.blocks.push(BasicBlock {
                    statements: vec![bad],
                    terminator: Located {
                        value: Terminator::Unreachable,
                        origin: None,
                    },
                });
                assert!(invalid.validate().is_err(), "case {case} {integer:?}");
            }
        }
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
                    bits: u128::from(u64::MAX),
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
    fn char_constants_and_operators_are_exact_and_validated() {
        let function = nominal_lowered(&with_root(Ty::Unit, vec![], vec![]));
        let scalar = |value| Operand::Constant(Constant::Char(ir::CharValue { value }));
        for value in [0, 0xd7ff, 0xe000, 0x10ffff] {
            assert_eq!(function.operand_ty(&scalar(value)).unwrap(), Ty::Char);
        }
        for value in [0xd800, 0xdfff, 0x110000, u32::MAX] {
            assert!(function.operand_ty(&scalar(value)).is_err());
        }
        for op in [
            BinaryOp::Eq,
            BinaryOp::Ne,
            BinaryOp::Lt,
            BinaryOp::Le,
            BinaryOp::Gt,
            BinaryOp::Ge,
        ] {
            assert_eq!(
                function
                    .rvalue_ty(&Rvalue::Binary {
                        op,
                        left: scalar(0),
                        right: scalar(0x10ffff)
                    })
                    .unwrap(),
                Ty::Bool
            );
            assert!(
                function
                    .rvalue_ty(&Rvalue::Binary {
                        op,
                        left: scalar(65),
                        right: Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::U32,
                            bits: 65
                        }))
                    })
                    .is_err()
            );
        }
        for op in [
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Rem,
            BinaryOp::Shl,
            BinaryOp::Shr,
            BinaryOp::BitAnd,
            BinaryOp::BitOr,
            BinaryOp::BitXor,
        ] {
            assert!(
                function
                    .rvalue_ty(&Rvalue::Binary {
                        op,
                        left: scalar(65),
                        right: scalar(1)
                    })
                    .is_err()
            );
        }
        for op in [UnaryOp::Neg, UnaryOp::Not] {
            assert!(
                function
                    .rvalue_ty(&Rvalue::Unary {
                        op,
                        value: scalar(65)
                    })
                    .is_err()
            );
        }
        assert!(
            function
                .rvalue_ty(&Rvalue::IntCast {
                    value: scalar(65),
                    to: IntTy::U32
                })
                .is_err()
        );
    }

    #[test]
    fn wide_shift_count_boundaries_do_not_truncate_high_limbs() {
        let function = nominal_lowered(&with_root(Ty::Unit, vec![], vec![]));
        for op in [BinaryOp::Shl, BinaryOp::Shr] {
            for lhs in [IntTy::I128, IntTy::U128] {
                for bits in [0, 127, 128, 1_u128 << 64, 1_u128 << 127, u128::MAX] {
                    let result = function.rvalue_ty(&Rvalue::Binary {
                        op,
                        left: Operand::Constant(Constant::Int(IntValue { ty: lhs, bits: 1 })),
                        right: Operand::Constant(Constant::Int(IntValue {
                            ty: IntTy::U128,
                            bits,
                        })),
                    });
                    if bits < 128 {
                        assert_eq!(result.unwrap(), Ty::Int(lhs));
                    } else {
                        assert!(result.unwrap_err().contains("shift count"));
                    }
                }
            }
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
            IntTy::I128,
            IntTy::U128,
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
                    for bits in [0, u128::from(left.bits() - 1)] {
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
                    for bits in [u128::from(left.bits()), right.mask()] {
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
                    if right.bits() >= 64 {
                        assert!(
                            function
                                .rvalue_ty(&Rvalue::Binary {
                                    op,
                                    left: lhs.clone(),
                                    right: count(1_u128 << (right.bits() - 1))
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
                    ty: IntTy::U128,
                    bits: 1_u128 << 64,
                },
                IntValue {
                    ty: IntTy::I128,
                    bits: 1_u128 << 127,
                },
                IntValue {
                    ty: IntTy::U128,
                    bits: 128,
                },
                IntValue {
                    ty: IntTy::U64,
                    bits: 1_u128 << 64,
                },
                IntValue {
                    ty: IntTy::U64,
                    bits: 64,
                },
                IntValue {
                    ty: IntTy::U64,
                    bits: u128::from(u64::MAX),
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
                IntTy::I128,
                IntTy::U128,
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
    fn leading_zeros_validate_actual_operand_and_dead_cfg_rows() {
        let function = nominal_lowered(&with_root(Ty::Int(IntTy::U32), vec![], vec![]));
        for ty in [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::I128,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::U128,
            IntTy::Usize,
        ] {
            assert_eq!(
                function
                    .rvalue_ty(&Rvalue::LeadingZeros {
                        value: Operand::Constant(Constant::Int(IntValue {
                            ty,
                            bits: ty.mask()
                        })),
                    })
                    .unwrap(),
                Ty::Int(IntTy::U32)
            );
        }
        for root in [
            Ty::Bool,
            Ty::Char,
            Ty::Float(ir::FloatTy::F64),
            Ty::Unit,
            Ty::Never,
            Ty::Tuple(std::sync::Arc::from([Ty::Int(IntTy::I32)])),
            Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: std::sync::Arc::new(Ty::Int(IntTy::I32)),
            },
        ] {
            let mut invalid = nominal_lowered(&with_root(root, vec![], vec![]));
            let destination = LocalId(invalid.locals.len());
            invalid.locals.push(ir::LocalDecl {
                name: "count".into(),
                ty: Ty::Int(IntTy::U32),
                mutable: true,
            });
            for value in [
                Operand::Copy(place(LocalId(0))),
                Operand::Move(place(LocalId(0))),
            ] {
                assert!(
                    invalid
                        .rvalue_ty(&Rvalue::LeadingZeros {
                            value: value.clone()
                        })
                        .is_err()
                );
                // A correct u32 destination must not conceal an invalid operand,
                // including in a block unreachable from entry.
                invalid.blocks.push(BasicBlock {
                    statements: vec![Located {
                        value: Statement::Assign {
                            destination: place(destination),
                            value: Rvalue::LeadingZeros { value },
                        },
                        origin: None,
                    }],
                    terminator: Located {
                        value: Terminator::Unreachable,
                        origin: None,
                    },
                });
                assert!(invalid.validate().unwrap_err().contains("integer source"));
                invalid.blocks.pop();
            }
        }
        for (destination, value, diagnostic) in [
            (
                LocalId(0),
                Operand::Constant(Constant::Int(IntValue {
                    ty: IntTy::U8,
                    bits: 256,
                })),
                "noncanonical",
            ),
            (
                function.return_local,
                Operand::Constant(Constant::Int(IntValue::i32(0))),
                "assignment type mismatch",
            ),
            (
                LocalId(0),
                Operand::Copy(place(LocalId(usize::MAX))),
                "invalid CFG local",
            ),
        ] {
            let mut invalid = function.clone();
            invalid.blocks.push(BasicBlock {
                statements: vec![Located {
                    value: Statement::Assign {
                        destination: place(destination),
                        value: Rvalue::LeadingZeros { value },
                    },
                    origin: None,
                }],
                terminator: Located {
                    value: Terminator::Unreachable,
                    origin: None,
                },
            });
            assert!(invalid.validate().unwrap_err().contains(diagnostic));
        }
    }

    #[test]
    fn leading_zeros_lower_once_preserve_origins_and_bound_cycles() {
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
                    Ty::Int(IntTy::U32),
                    ir::ExprKind::LeadingZeros { value: ExprId(0) },
                ),
            ],
            vec![ir::Stmt::Expr(ExprId(1))],
        );
        let function = nominal_lowered(&source);
        assert_eq!(
            function
                .blocks
                .iter()
                .filter(|block| matches!(&block.terminator.value,
            Terminator::Call { name, .. } if name == "once"))
                .count(),
            1
        );
        let operations: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter(|statement| {
                matches!(
                    &statement.value,
                    Statement::Assign {
                        value: Rvalue::LeadingZeros { .. },
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].origin, Some(ExprId(1)));
        assert_storage_paths(&function);
        assert!(super::super::ownership::analyze(&function).passed());
        let mut cyclic = source;
        cyclic.exprs[0] = expr(
            Ty::Int(IntTy::U32),
            ir::ExprKind::LeadingZeros { value: ExprId(0) },
        );
        cyclic.validate().unwrap();
        assert!(
            lower(&cyclic)
                .unwrap_err()
                .contains("lowering depth resource")
        );
    }

    #[test]
    fn casts_validate_all_integer_pairs_dead_operands_and_evaluate_source_once() {
        let (_module_names, modules) = fixture_modules();
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
            IntTy::I128,
            IntTy::U128,
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
            facts: ir::FactsCtx::Concrete,
            budget: None,
            resource_error: None,
            lowering_depth: 0,
            lowering_work: ir::MAX_PATTERN_PROJECTION_WORK,
            origins: 0,
            loans: vec![],
            call_blocks: vec![],
            used_loans: Default::default(),
            source: &source,
            result: function,
            expressions: vec![],
            parameters: Default::default(),
            blocks: vec![],
            current: BlockId(0),
            drop_enabled: false,
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
        let budget = ir::PlanBudget::default();
        let signature = ir::Signature {
            params: chain
                .params
                .iter()
                .map(|id| chain.locals[id.0].ty.clone())
                .collect(),
            result: chain.result.clone(),
            ref_contract: chain.ref_contract.clone(),
        };
        let facts = super::super::traits::test_support::facts_for_signature(
            modules.clone(),
            &signature,
            &chain
                .locals
                .iter()
                .map(|local| local.ty.clone())
                .collect::<Vec<_>>(),
            &budget,
        )
        .unwrap();
        super::super::backend::emit_checked_function(&chain, facts).unwrap();
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
            IntTy::I128,
            IntTy::U128,
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
            if ty.bits() < 128 {
                function.blocks.push(BasicBlock {
                    statements: vec![Located {
                        value: Statement::Assign {
                            destination: place(LocalId(0)),
                            value: Rvalue::Use(Operand::Constant(Constant::Int(IntValue {
                                ty,
                                bits: 1u128 << ty.bits(),
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
        let (_module_names, modules) = fixture_modules();
        let item = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
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
        let (_module_names, modules) = fixture_modules();
        let item = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
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
        let (_module_names, modules) = fixture_modules();
        let item = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
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
        let (_module_names, modules) = fixture_modules();
        let item = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
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
        let (_module_names, modules) = fixture_modules();
        for len in [0, 1] {
            let item = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
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
    fn never_scrutinee_keeps_explicit_domains_and_dead_arm_checks_without_result_assignment() {
        for array in [false, true] {
            let domain = if array {
                Ty::Array {
                    element: std::sync::Arc::new(Ty::Int(IntTy::I32)),
                    len: 3,
                }
            } else {
                Ty::Tuple(vec![Ty::Int(IntTy::I32); 3].into())
            };
            let projection = if array {
                Projection::ConstantIndex(0)
            } else {
                Projection::Field(0)
            };
            let mut source = with_root(
                domain.clone(),
                vec![
                    ir::TypedExpr {
                        ty: Ty::Never,
                        completion: ir::Completion::Never,
                        kind: ir::ExprKind::Block(ir::Block {
                            statements: vec![ir::Stmt::Return(None)],
                            tail: None,
                        }),
                    },
                    expr(
                        Ty::Unit,
                        ir::ExprKind::Block(ir::Block {
                            statements: vec![],
                            tail: None,
                        }),
                    ),
                    ir::TypedExpr {
                        ty: Ty::Never,
                        completion: ir::Completion::Never,
                        kind: ir::ExprKind::Match {
                            scrutinee: ExprId(0),
                            arms: vec![ir::MatchArm {
                                pattern: ir::MatchPattern::Aggregate {
                                    ty: domain,
                                    bindings: vec![ir::PatternBinding {
                                        local: LocalId(1),
                                        projection: vec![projection],
                                    }],
                                },
                                body: ExprId(1),
                            }],
                        },
                    },
                ],
                vec![ir::Stmt::Expr(ExprId(2))],
            );
            source.locals.push(ir::LocalDecl {
                name: "selected".into(),
                ty: Ty::Int(IntTy::I32),
                mutable: false,
            });
            let function = nominal_lowered(&source);
            // The unreachable body is still lowered, but neither the binding
            // nor the Never match result receives a fabricated value.
            let body_local = LocalId(source.locals.len() + 1);
            let match_local = LocalId(source.locals.len() + 2);
            assert!(function.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(
                &statement.value, Statement::Assign { destination, value: Rvalue::Use(Operand::Constant(Constant::Unit)) } if destination.local == body_local
            )));
            assert!(!function.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(
                &statement.value, Statement::Assign { destination, .. } if destination.local == match_local || destination.local == LocalId(1)
            )));
            assert!(super::super::ownership::analyze(&function).passed());
            let ir::ExprKind::Match { arms, .. } = &mut source.exprs[2].kind else {
                unreachable!()
            };
            let ir::MatchPattern::Aggregate { bindings, .. } = &mut arms[0].pattern else {
                unreachable!()
            };
            bindings[0].projection[0] = if array {
                Projection::ConstantIndex(usize::MAX)
            } else {
                Projection::Field(usize::MAX)
            };
            assert!(source.validate().is_err());
            assert!(lower(&source).is_err());
        }
    }

    #[test]
    fn let_else_success_binding_lives_through_following_statements_without_arm_cleanup() {
        let function = nominal_lowered(&checked(
            "struct Cell{n:i32} enum E{A(Cell,i32),B}fn f(p:E)->i32{let E::A(cell,..)=p else{return 0;};let n=1;cell.n+n}fn main(){}",
            0,
        ));
        let root = function.params[0];
        let (selected, rejected) = function
            .blocks
            .iter()
            .find_map(|block| match block.terminator.value {
                Terminator::SwitchVariant {
                    then_block,
                    else_block,
                    ..
                } => Some((then_block, else_block)),
                _ => None,
            })
            .unwrap();
        let statements = &function.blocks[selected.0].statements;
        let (initialize, binding) = statements
            .iter()
            .enumerate()
            .find_map(|(index, statement)| match &statement.value {
                Statement::Assign {
                    destination,
                    value: Rvalue::Use(Operand::Move(place)),
                } if place.local == root
                    && matches!(
                        place.projection.as_slice(),
                        [Projection::Downcast(_), Projection::Field(0)]
                    ) =>
                {
                    Some((index, destination.local))
                }
                _ => None,
            })
            .unwrap();
        let read = statements.iter().position(|statement| matches!(&statement.value, Statement::Assign { value: Rvalue::Use(Operand::Copy(place)), .. } if place.local == binding && place.projection == [Projection::Field(0)])).unwrap();
        let dead = statements.iter().position(|statement| matches!(statement.value, Statement::StorageDead(local) if local == binding)).unwrap();
        assert!(initialize < read && read < dead);
        assert!(!function.blocks[rejected.0].statements.iter().any(
            |statement| matches!(statement.value, Statement::StorageLive(local) if local == binding)
        ));
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn let_else_initializer_runs_once_and_real_outer_control_exits_remain_divergent() {
        let function = nominal_lowered(&checked(
            "enum E{A(i32),B}fn make()->E{E::A(7)}fn f()->i32{let E::A(n)=make() else{return 0;};n}fn main(){}",
            1,
        ));
        assert_eq!(
            function
                .blocks
                .iter()
                .filter(|block| matches!(block.terminator.value, Terminator::Call { .. }))
                .count(),
            1
        );
        assert!(super::super::ownership::analyze(&function).passed());
        for text in [
            "fn f(p:bool)->i32{let true=p else{return 0;};1}fn main(){}",
            "fn f(p:i32)->i32{let 7=p else{return 0;};1}fn main(){}",
            "enum E{A(i32),B}fn f(p:E)->i32{loop{let E::A(n)=p else{break 0;};break n;}}fn main(){}",
            "enum E{A(i32),B}fn f(p:E){loop{let E::A(n)=p else{continue;};break;}}fn main(){}",
        ] {
            let function = nominal_lowered(&checked(text, 0));
            assert!(
                super::super::ownership::analyze(&function).passed(),
                "{text}"
            );
        }
    }

    #[test]
    fn let_else_rejects_empty_block_with_forged_never_metadata() {
        let mut source = checked(
            "fn f(p:bool)->i32{let true=p else{return 0;};1}fn main(){}",
            0,
        );
        nominal_lowered(&source);
        let otherwise = source
            .body
            .statements
            .iter()
            .find_map(|statement| match statement {
                ir::Stmt::LetElse { else_branch, .. } => Some(*else_branch),
                _ => None,
            })
            .unwrap();
        // Preserve both claimed Never fields. They cannot replace evidence of
        // an actual abrupt edge or loop in the lowered failure branch.
        assert_eq!(source.exprs[otherwise.0].ty, Ty::Never);
        assert_eq!(source.exprs[otherwise.0].completion, ir::Completion::Never);
        source.exprs[otherwise.0].kind = ir::ExprKind::Block(ir::Block {
            statements: Vec::new(),
            tail: None,
        });
        assert!(lower(&source).is_err());
        let abrupt = ExprId(source.exprs.len());
        source.exprs.push(ir::TypedExpr {
            ty: Ty::Never,
            completion: ir::Completion::Never,
            kind: ir::ExprKind::Break {
                target: LoopId(0),
                value: None,
            },
        });
        source.exprs[otherwise.0].kind = ir::ExprKind::Loop {
            id: LoopId(0),
            body: ir::Block {
                statements: vec![ir::Stmt::Expr(abrupt)],
                tail: None,
            },
        };
        assert!(lower(&source).is_err());
    }

    fn divergence_probe<'a>(
        source: &'a ir::CheckedFunction,
        budget: &'a ir::PlanBudget,
        terminators: Vec<Option<Terminator>>,
    ) -> Lowerer<'a> {
        Lowerer {
            facts: ir::FactsCtx::Concrete,
            budget: Some(budget),
            resource_error: None,
            lowering_depth: 0,
            lowering_work: 0,
            origins: 0,
            loans: Vec::new(),
            call_blocks: Vec::new(),
            used_loans: Default::default(),
            source,
            result: nominal_lowered(source),
            expressions: Vec::new(),
            parameters: source.params.iter().copied().collect(),
            blocks: terminators
                .into_iter()
                .map(|terminator| PendingBlock {
                    statements: Vec::new(),
                    terminator: terminator.map(|value| Located {
                        value,
                        origin: None,
                    }),
                })
                .collect(),
            current: BlockId(1),
            drop_enabled: false,
            scopes: Vec::new(),
            loops: Vec::new(),
        }
    }

    #[test]
    fn let_else_divergence_proof_uses_real_edges_and_releases_scratch_on_all_outcomes() {
        let source = with_root(Ty::Unit, Vec::new(), Vec::new());
        for (terminators, accepts) in [
            (vec![Some(Terminator::Goto(BlockId(1))), None], false),
            (vec![Some(Terminator::Goto(BlockId(0))), None], true),
            (vec![Some(Terminator::Return), None], true),
            (vec![Some(Terminator::Goto(BlockId(2))), None, None], true),
            (
                vec![
                    Some(Terminator::Switch {
                        condition: Operand::Constant(Constant::Bool(true)),
                        then_block: BlockId(0),
                        else_block: BlockId(1),
                    }),
                    None,
                ],
                true,
            ),
            (
                vec![
                    Some(Terminator::Switch {
                        condition: Operand::Constant(Constant::Bool(false)),
                        then_block: BlockId(0),
                        else_block: BlockId(1),
                    }),
                    None,
                ],
                false,
            ),
        ] {
            let budget = ir::PlanBudget::default();
            let mut probe = divergence_probe(&source, &budget, terminators);
            let before = budget.usage_snapshot().bytes;
            assert_eq!(
                probe.prove_failure_diverges(BlockId(0), BlockId(1)).is_ok(),
                accepts
            );
            let after = budget.usage_snapshot();
            assert_eq!(after.bytes, before);
            assert_eq!(after.reservations.current_reservations, 0);
            assert!(after.reservations.peak_bytes >= 2 * (std::mem::size_of::<BlockId>() + 1));
            assert!(after.first_failure.is_none());
        }
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            work: 4,
            ..Default::default()
        });
        let mut probe = divergence_probe(
            &source,
            &budget,
            vec![Some(Terminator::Goto(BlockId(0))), None],
        );
        let error = probe
            .prove_failure_diverges(BlockId(0), BlockId(1))
            .unwrap_err();
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Work);
        assert_eq!(budget.usage_snapshot().bytes, 0);
        assert_eq!(budget.usage_snapshot().reservations.current_reservations, 0);
        assert!(budget.usage_snapshot().reservations.peak_bytes > 0);
        assert_eq!(
            probe
                .prove_failure_diverges(BlockId(0), BlockId(1))
                .unwrap_err(),
            error
        );
        assert_eq!(budget.first_resource_failure(), Some(failure));
    }

    #[test]
    fn while_let_continue_returns_to_the_single_scrutinee_call_site() {
        let function = nominal_lowered(&checked(
            "fn step()->bool{false} fn f(){while let true=step(){continue;}} fn main(){}",
            1,
        ));
        let calls: Vec<_> = function
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| {
                matches!(block.terminator.value, Terminator::Call { .. }).then_some(BlockId(index))
            })
            .collect();
        assert_eq!(calls.len(), 1);
        // Entry and a real continue backedge both reach the code containing
        // the call. The scrutinee is not snapshotted before entering the loop.
        assert!(function.blocks.iter().filter(|block| matches!(block.terminator.value, Terminator::Goto(target) if target == calls[0])).count() >= 2);
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn while_let_payload_bindings_move_only_on_success_and_die_before_continue() {
        let function = nominal_lowered(&checked(
            "struct Cell{n:i32} enum E{A(Cell,i32),B} fn f(p:E){let mut state=p;while let E::A(cell,..)=state{let n=cell.n;state=E::B;continue;}} fn main(){}",
            0,
        ));
        let root = LocalId(
            function
                .locals
                .iter()
                .position(|local| local.name == "state")
                .unwrap(),
        );
        let Ty::Enum(ty) = &function.locals[root.0].ty else {
            unreachable!()
        };
        let variant = ir::VariantId {
            owner: ty.def().id,
            index: 0,
        };
        let (header, selected) = function
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match &block.terminator.value {
                Terminator::SwitchVariant {
                    place,
                    variant: actual,
                    then_block,
                    ..
                } if place.local == root && *actual == variant => {
                    Some((BlockId(index), *then_block))
                }
                _ => None,
            })
            .unwrap();
        let moves: Vec<_> = function
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(index, block)| {
                block
                    .statements
                    .iter()
                    .filter_map(move |statement| match &statement.value {
                        Statement::Assign {
                            destination,
                            value: Rvalue::Use(Operand::Move(place)),
                        } if place.local == root => {
                            Some((BlockId(index), destination.local, place))
                        }
                        _ => None,
                    })
            })
            .collect();
        assert_eq!(moves.len(), 1);
        let (block, binding, projection) = moves[0];
        assert_eq!(block, selected);
        assert_eq!(
            projection.projection,
            [Projection::Downcast(variant), Projection::Field(0)]
        );
        assert!(function.blocks[selected.0].statements.iter().any(
            |statement| matches!(statement.value, Statement::StorageLive(local) if local == binding)
        ));
        assert!(function.blocks.iter().any(|block| matches!(block.terminator.value, Terminator::Goto(target) if target == header) && block.statements.iter().any(|statement| matches!(statement.value, Statement::StorageDead(local) if local == binding))));
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn while_let_never_scrutinee_keeps_dead_body_without_fabricating_never_values() {
        let source = checked(
            "fn f()->i32{while let true={return 7;}{let x=1;}8} fn main(){}",
            0,
        );
        assert!(source.exprs.iter().any(|expression| matches!(
            expression.kind,
            ir::ExprKind::Loop { .. }
        ) && expression.ty == Ty::Unit
            && expression.completion == ir::Completion::Never));
        assert!(source.exprs.iter().any(|expression| matches!(
            expression.kind,
            ir::ExprKind::Match { .. }
        ) && expression.ty == Ty::Unit
            && expression.completion == ir::Completion::Never));
        let function = nominal_lowered(&source);
        assert!(function.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value, Statement::Assign { value: Rvalue::Use(Operand::Constant(Constant::Int(value))), .. } if *value == IntValue::i32(1))));
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn while_let_dead_body_retains_exact_payload_projection_checks() {
        let mut source = checked(
            "enum E{A(i32),B} fn f(){while let E::A(n)=E::B{let x=n;}} fn main(){}",
            0,
        );
        nominal_lowered(&source);
        let binding = source
            .exprs
            .iter_mut()
            .find_map(|expression| match &mut expression.kind {
                ir::ExprKind::Match { arms, .. } => {
                    arms.iter_mut().find_map(|arm| match &mut arm.pattern {
                        ir::MatchPattern::EnumVariant { bindings, .. } => bindings.first_mut(),
                        _ => None,
                    })
                }
                _ => None,
            })
            .unwrap();
        binding.projection[1] = Projection::Field(usize::MAX);
        assert!(source.validate().is_err());
        assert!(lower(&source).is_err());
    }

    #[test]
    fn if_let_payload_move_is_confined_to_the_actual_selected_variant_edge() {
        let function = nominal_lowered(&checked(
            "struct Cell{n:i32} enum E{A(Cell,i32,Cell),B} fn f(p:E)->i32{if let E::A(first,..)=p{first.n}else{0}} fn main(){}",
            0,
        ));
        let root = function.params[0];
        let Ty::Enum(ty) = &function.locals[root.0].ty else {
            unreachable!()
        };
        let variant = ir::VariantId {
            owner: ty.def().id,
            index: 0,
        };
        let (selected, rejected) = function
            .blocks
            .iter()
            .find_map(|block| match &block.terminator.value {
                Terminator::SwitchVariant {
                    place,
                    variant: actual,
                    then_block,
                    else_block,
                } if place.local == root && *actual == variant => Some((*then_block, *else_block)),
                _ => None,
            })
            .unwrap();
        let moves: Vec<_> = function
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(index, block)| {
                block
                    .statements
                    .iter()
                    .filter_map(move |statement| match &statement.value {
                        Statement::Assign {
                            value: Rvalue::Use(Operand::Move(place)),
                            ..
                        } if place.local == root => Some((index, place)),
                        _ => None,
                    })
            })
            .collect();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].0, selected.0);
        assert_ne!(moves[0].0, rejected.0);
        assert_eq!(
            moves[0].1.projection,
            [Projection::Downcast(variant), Projection::Field(0)]
        );
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn if_let_scrutinee_call_is_evaluated_once_for_refutable_and_irrefutable_patterns() {
        for text in [
            "fn make()->bool{true} fn f()->i32{if let true=make(){1}else{2}} fn main(){}",
            "fn make()->i32{7} fn f()->i32{if let 7=make(){1}else{2}} fn main(){}",
            "enum E{A(i32),B} fn make()->E{E::A(7)} fn f()->i32{if let E::A(n)=make(){n}else{0}} fn main(){}",
            "fn make()->(i32,i32){(1,2)} fn f()->i32{if let (first,last)=make(){first+last}else{0}} fn main(){}",
        ] {
            let function = nominal_lowered(&checked(text, 1));
            assert_eq!(
                function
                    .blocks
                    .iter()
                    .filter(|block| matches!(block.terminator.value, Terminator::Call { .. }))
                    .count(),
                1,
                "{text}"
            );
            assert!(
                super::super::ownership::analyze(&function).passed(),
                "{text}"
            );
        }
    }

    #[test]
    fn if_let_dead_selected_pattern_keeps_full_ir_projection_validation() {
        let mut source = checked(
            "enum E{A(i32),B} fn f()->i32{if let E::A(n)=E::B{n}else{0}} fn main(){}",
            0,
        );
        nominal_lowered(&source);
        let binding = source
            .exprs
            .iter_mut()
            .find_map(|expression| match &mut expression.kind {
                ir::ExprKind::Match { arms, .. } => {
                    arms.iter_mut().find_map(|arm| match &mut arm.pattern {
                        ir::MatchPattern::EnumVariant { bindings, .. } => bindings.first_mut(),
                        _ => None,
                    })
                }
                _ => None,
            })
            .unwrap();
        binding.projection[1] = Projection::Field(usize::MAX);
        assert!(source.validate().is_err());
        assert!(lower(&source).is_err());
    }

    #[test]
    fn array_rest_patterns_copy_only_actual_indices_and_nested_copy_fields() {
        for matching in [false, true] {
            for nested in [false, true] {
                let (element, pattern) = if nested {
                    ("Cell", "[Cell{n:first},..,Cell{n:last}]")
                } else {
                    ("i32", "[first,..,last]")
                };
                let body = if matching {
                    format!("match p{{{pattern}=>first+last}}")
                } else {
                    format!("let {pattern}=p;first+last")
                };
                let text = format!(
                    "struct Cell{{n:i32}} fn f(p:[{element};3])->i32{{{body}}} fn main(){{}}"
                );
                let function = nominal_lowered(&checked(&text, 0));
                let root = function.params[0];
                let reads: Vec<_> = function.blocks.iter().flat_map(|block| &block.statements).filter_map(|statement| match &statement.value {
                    Statement::Assign { value: Rvalue::Use(operand @ (Operand::Copy(place) | Operand::Move(place))), .. } if place.local == root => Some(operand),
                    _ => None,
                }).collect();
                assert_eq!(reads.len(), 2);
                for index in [0, 2] {
                    let mut projection = vec![Projection::ConstantIndex(index)];
                    if nested {
                        projection.push(Projection::Field(0));
                    }
                    assert_eq!(reads.iter().filter(|operand| matches!(operand, Operand::Copy(place) if place.projection == projection)).count(), 1);
                }
                assert!(!function.blocks.iter().any(|block| matches!(
                    block.terminator.value,
                    Terminator::SwitchVariant { .. }
                )));
                assert!(super::super::ownership::analyze(&function).passed());
            }
        }
    }

    #[test]
    fn array_rest_temporary_scrutinee_is_evaluated_once() {
        let function = nominal_lowered(&checked(
            "fn make()->[i32;3]{[1,2,3]} fn f()->i32{match make(){[first,..,last]=>first+last}} fn main(){}",
            1,
        ));
        assert_eq!(
            function
                .blocks
                .iter()
                .filter(|block| matches!(block.terminator.value, Terminator::Call { .. }))
                .count(),
            1
        );
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn enum_tuple_rest_bindings_keep_actual_downcast_and_physical_field_indices() {
        for matching in [false, true] {
            let text = if matching {
                "struct Cell{n:i32} enum E{A(Cell,Cell,Cell),B} fn f(p:E)->i32{match p{E::A(first,..,last)=>first.n+last.n,E::B=>0}} fn main(){}"
            } else {
                "struct Cell{n:i32} enum E{A(Cell,Cell,Cell)} fn f(p:E)->i32{let E::A(first,..,last)=p;first.n+last.n} fn main(){}"
            };
            let function = nominal_lowered(&checked(text, 0));
            let root = function.params[0];
            let Ty::Enum(ty) = &function.locals[root.0].ty else {
                unreachable!()
            };
            let variant = ir::VariantId {
                owner: ty.def().id,
                index: 0,
            };
            let reads: Vec<_> = function
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .filter_map(|statement| match &statement.value {
                    Statement::Assign {
                        value: Rvalue::Use(operand @ (Operand::Copy(place) | Operand::Move(place))),
                        ..
                    } if place.local == root => Some(operand),
                    _ => None,
                })
                .collect();
            assert_eq!(reads.len(), 2);
            for index in [0, 2] {
                assert_eq!(reads.iter().filter(|operand| matches!(operand, Operand::Move(place) if place.projection == [Projection::Downcast(variant), Projection::Field(index)])).count(), 1);
            }
            assert_eq!(
                function.blocks.iter().any(|block| matches!(
                    block.terminator.value,
                    Terminator::SwitchVariant { .. }
                )),
                matching
            );
            assert!(super::super::ownership::analyze(&function).passed());
        }
    }

    #[test]
    fn enum_rest_only_matches_inspect_tags_but_single_variant_let_does_not_move() {
        for matching in [false, true] {
            let text = if matching {
                "struct Cell{n:i32} enum E{A(Cell,Cell,Cell),B} fn f(p:E)->E{match p{E::A(..)=>(),E::B=>()};p} fn main(){}"
            } else {
                "struct Cell{n:i32} enum E{A(Cell,Cell,Cell)} fn f(p:E)->E{let E::A(..)=p;p} fn main(){}"
            };
            let function = nominal_lowered(&checked(text, 0));
            let root = function.params[0];
            let reads: Vec<_> = function
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .filter_map(|statement| match &statement.value {
                    Statement::Assign {
                        value: Rvalue::Use(operand @ (Operand::Copy(place) | Operand::Move(place))),
                        ..
                    } if place.local == root => Some(operand),
                    _ => None,
                })
                .collect();
            assert_eq!(reads.len(), 1);
            assert!(matches!(reads[0], Operand::Move(place) if place.projection.is_empty()));
            assert_eq!(
                function.blocks.iter().any(|block| matches!(
                    block.terminator.value,
                    Terminator::SwitchVariant { .. }
                )),
                matching
            );
            assert!(super::super::ownership::analyze(&function).passed());
        }
    }

    #[test]
    fn dead_enum_rest_arm_still_rejects_invalid_physical_payload_projection() {
        let mut source = checked(
            "enum E{A(i32,i32,i32),B} fn f(p:E)->i32{match p{_=>0,E::A(first,..,last)=>first+last,E::B=>0}} fn main(){}",
            0,
        );
        nominal_lowered(&source);
        let bindings = source
            .exprs
            .iter_mut()
            .find_map(|expression| match &mut expression.kind {
                ir::ExprKind::Match { arms, .. } => {
                    arms.iter_mut().find_map(|arm| match &mut arm.pattern {
                        ir::MatchPattern::EnumVariant { bindings, .. } if !bindings.is_empty() => {
                            Some(bindings)
                        }
                        _ => None,
                    })
                }
                _ => None,
            })
            .unwrap();
        bindings[0].projection[1] = Projection::Field(usize::MAX);
        assert!(source.validate().is_err());
        assert!(lower(&source).is_err());
    }

    #[test]
    fn enum_tuple_rest_temporary_scrutinee_is_evaluated_once() {
        let function = nominal_lowered(&checked(
            "enum E{A(i32,i32,i32),B} fn make()->E{E::A(1,2,3)} fn f()->i32{match make(){E::A(first,..,last)=>first+last,E::B=>0}} fn main(){}",
            1,
        ));
        assert_eq!(
            function
                .blocks
                .iter()
                .filter(|block| matches!(block.terminator.value, Terminator::Call { .. }))
                .count(),
            1
        );
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn divergent_single_variant_rest_let_does_not_load_payload_or_test_tag() {
        let function = nominal_lowered(&checked(
            "enum E{A(i32,i32,i32)} fn f(){let E::A(first,..,last):E={return;};} fn main(){}",
            0,
        ));
        assert!(
            !function
                .blocks
                .iter()
                .any(|block| matches!(block.terminator.value, Terminator::SwitchVariant { .. }))
        );
        assert!(!function.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(&statement.value, Statement::Assign { destination, .. } if matches!(function.locals[destination.local.0].name.as_str(), "first" | "last"))));
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn positional_rest_emits_only_actual_prefix_and_suffix_field_moves() {
        for structural in [false, true] {
            for matching in [false, true] {
                let (ty, pattern) = if structural {
                    ("(Cell,Cell,Cell)", "(first,..,last)")
                } else {
                    ("Trio", "Trio(first,..,last)")
                };
                let body = if matching {
                    format!("match p{{{pattern}=>{{let _=first;last}}}}")
                } else {
                    format!("let {pattern}=p;let _=first;last")
                };
                let text = format!(
                    "struct Cell{{n:i32}} struct Trio(Cell,Cell,Cell); fn f(p:{ty})->Cell{{{body}}} fn main(){{}}"
                );
                let function = nominal_lowered(&checked(&text, 0));
                let root = function.params[0];
                let reads: Vec<_> = function.blocks.iter().flat_map(|block| &block.statements).filter_map(|statement| match &statement.value {
                    Statement::Assign { value: Rvalue::Use(operand @ (Operand::Copy(place) | Operand::Move(place))), .. } if place.local == root => Some(operand),
                    _ => None,
                }).collect();
                assert_eq!(reads.len(), 2);
                for index in [0, 2] {
                    assert_eq!(reads.iter().filter(|operand| matches!(operand, Operand::Move(place) if place.projection == [Projection::Field(index)])).count(), 1);
                }
                assert!(!function.blocks.iter().any(|block| matches!(
                    block.terminator.value,
                    Terminator::SwitchVariant { .. }
                )));
                assert!(super::super::ownership::analyze(&function).passed());
            }
        }
    }

    #[test]
    fn record_rest_match_emits_only_selected_field_move_and_no_tag() {
        let source = checked(
            "struct Cell{n:i32} struct Record{selected:Cell,omitted:Cell} fn f(p:Record)->Cell{match p{Record{selected,..}=>selected}} fn main(){}",
            0,
        );
        let function = nominal_lowered(&source);
        let root = function.params[0];
        let reads: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.value {
                Statement::Assign {
                    value: Rvalue::Use(operand @ (Operand::Copy(place) | Operand::Move(place))),
                    ..
                } if place.local == root => Some(operand),
                _ => None,
            })
            .collect();
        assert_eq!(reads.len(), 1);
        assert!(
            matches!(reads[0], Operand::Move(place) if place.projection == [Projection::Field(0)])
        );
        assert!(
            !function
                .blocks
                .iter()
                .any(|block| matches!(block.terminator.value, Terminator::SwitchVariant { .. }))
        );
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn bindingless_record_rest_match_keeps_whole_nominal_available() {
        for pattern in ["Record{..}", "Record{selected:_,..}"] {
            let text = format!(
                "struct Cell{{n:i32}} struct Record{{selected:Cell,omitted:Cell}} fn f(p:Record)->Record{{match p{{{pattern}=>()}};p}} fn main(){{}}"
            );
            let function = nominal_lowered(&checked(&text, 0));
            let root = function.params[0];
            let moves: Vec<_> = function
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .filter_map(|statement| match &statement.value {
                    Statement::Assign {
                        value: Rvalue::Use(Operand::Move(place)),
                        ..
                    } if place.local == root => Some(place),
                    _ => None,
                })
                .collect();
            assert_eq!(moves.len(), 1);
            assert!(moves[0].projection.is_empty());
            assert!(super::super::ownership::analyze(&function).passed());
        }
    }

    #[test]
    fn aggregate_match_binding_moves_only_selected_field_without_tag_test() {
        let (_module_names, modules) = fixture_modules();
        let child = Ty::Struct(nominal(&modules, vec![]));
        for structural in [false, true] {
            let fields = vec![child.clone(), Ty::Int(IntTy::I32)];
            let ty = if structural {
                Ty::Tuple(fields.into())
            } else {
                Ty::Struct(nominal(&modules, fields))
            };
            let mut source = with_root(
                ty.clone(),
                vec![
                    expr(ty.clone(), ir::ExprKind::Local(LocalId(0))),
                    expr(child.clone(), ir::ExprKind::Local(LocalId(1))),
                    expr(
                        child.clone(),
                        ir::ExprKind::Match {
                            scrutinee: ExprId(0),
                            arms: vec![ir::MatchArm {
                                pattern: ir::MatchPattern::Aggregate {
                                    ty: ty.clone(),
                                    bindings: vec![ir::PatternBinding {
                                        local: LocalId(1),
                                        projection: vec![Projection::Field(0)],
                                    }],
                                },
                                body: ExprId(1),
                            }],
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
                vec![ir::Stmt::Expr(ExprId(2)), ir::Stmt::Expr(ExprId(3))],
            );
            source.locals.push(ir::LocalDecl {
                name: "selected".into(),
                ty: child.clone(),
                mutable: false,
            });
            let function = nominal_lowered(&source);
            let reads = root_reads(&function);
            assert_eq!(reads.len(), 2);
            // The join block is allocated before the selected arm, so flat
            // block storage order is not execution order. Require each exact
            // projection once, without pretending this iterator is a trace.
            assert_eq!(
                reads.iter().filter(|operand| matches!(operand, Operand::Move(place) if place.projection == [Projection::Field(0)])).count(),
                1,
            );
            assert_eq!(
                reads.iter().filter(|operand| matches!(operand, Operand::Copy(place) if place.projection == [Projection::Field(1)])).count(),
                1,
            );
            assert!(function.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(
                &statement.value,
                Statement::Assign { destination, value: Rvalue::Use(Operand::Move(selected)) }
                    if destination.local == LocalId(1)
                        && destination.projection.is_empty()
                        && selected.local == LocalId(0)
                        && selected.projection == [Projection::Field(0)]
            )));
            assert!(
                !function.blocks.iter().any(|block| matches!(
                    block.terminator.value,
                    Terminator::SwitchVariant { .. }
                ))
            );
            assert!(super::super::ownership::analyze(&function).passed());
            source.body.statements.push(ir::Stmt::Expr(ExprId(0)));
            assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
        }
    }

    #[test]
    fn aggregate_match_without_bindings_does_not_consume_scrutinee() {
        let (_module_names, modules) = fixture_modules();
        let ty = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
        let source = with_root(
            ty.clone(),
            vec![
                expr(ty.clone(), ir::ExprKind::Local(LocalId(0))),
                expr(Ty::Bool, ir::ExprKind::Bool(true)),
                expr(
                    Ty::Bool,
                    ir::ExprKind::Match {
                        scrutinee: ExprId(0),
                        arms: vec![ir::MatchArm {
                            pattern: ir::MatchPattern::Aggregate {
                                ty,
                                bindings: vec![],
                            },
                            body: ExprId(1),
                        }],
                    },
                ),
            ],
            vec![ir::Stmt::Expr(ExprId(2)), ir::Stmt::Expr(ExprId(0))],
        );
        let function = nominal_lowered(&source);
        assert_eq!(root_reads(&function).len(), 1);
        assert!(
            matches!(root_reads(&function)[0], Operand::Move(place) if place.projection.is_empty())
        );
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn tuple_struct_pattern_kernel_moves_only_selected_plain_fields() {
        let (_module_names, modules) = fixture_modules();
        let child = Ty::Struct(nominal(&modules, vec![]));
        let ty = Ty::Struct(nominal(&modules, vec![child.clone(), Ty::Int(IntTy::I32)]));
        let mut source = with_root(
            ty.clone(),
            vec![
                expr(ty.clone(), ir::ExprKind::Local(LocalId(0))),
                expr(
                    Ty::Int(IntTy::I32),
                    ir::ExprKind::Field {
                        base: ExprId(0),
                        index: 1,
                    },
                ),
                expr(child.clone(), ir::ExprKind::Local(LocalId(1))),
            ],
            vec![
                ir::Stmt::LetPattern {
                    value: ExprId(0),
                    ty,
                    bindings: vec![ir::PatternBinding {
                        local: LocalId(1),
                        projection: vec![Projection::Field(0)],
                    }],
                },
                ir::Stmt::Expr(ExprId(1)),
                ir::Stmt::Expr(ExprId(2)),
            ],
        );
        source.locals.push(ir::LocalDecl {
            name: "selected".into(),
            ty: child,
            mutable: false,
        });
        let function = nominal_lowered(&source);
        let reads = root_reads(&function);
        assert_eq!(reads.len(), 2);
        assert!(
            matches!(reads[0], Operand::Move(place) if place.projection == [Projection::Field(0)])
        );
        assert!(
            matches!(reads[1], Operand::Copy(place) if place.projection == [Projection::Field(1)])
        );
        assert!(super::super::ownership::analyze(&function).passed());
        source.body.statements.push(ir::Stmt::Expr(ExprId(0)));
        assert!(!super::super::ownership::analyze(&nominal_lowered(&source)).passed());
    }

    #[test]
    fn scalar_field_reads_copy_only_the_projection_before_whole_struct_move() {
        let (_module_names, modules) = fixture_modules();
        let root = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
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
        let (_module_names, modules) = fixture_modules();
        let inner = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
        let root = Ty::Struct(nominal(&modules, vec![inner.clone(), inner.clone()]));
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
        let (_module_names, modules) = fixture_modules();
        let inner = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
        let root = Ty::Struct(nominal(&modules, vec![inner.clone(), Ty::Int(IntTy::I32)]));
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
    fn empty_nominal_aggregate_keeps_identity_and_has_no_projectable_fields() {
        let (_module_names, modules) = fixture_modules();
        let nominal = nominal(&modules, vec![]);
        let ty = Ty::Struct(nominal.clone());
        assert!(!ty.is_copy());
        validate_type(&ty, 0).unwrap();
        assert!(validate_type(&Ty::Tuple(Vec::new().into()), 0).is_err());
        let source = with_root(
            ty.clone(),
            vec![expr(
                ty.clone(),
                ir::ExprKind::Struct {
                    ty: nominal,
                    fields: vec![],
                },
            )],
            vec![ir::Stmt::Expr(ExprId(0))],
        );
        let function = nominal_lowered(&source);
        assert!(
            function
                .blocks
                .iter()
                .flat_map(|block| &block.statements)
                .any(|statement| matches!(&statement.value, Statement::Assign {
                value: Rvalue::Aggregate { ty: aggregate, fields }, ..
            } if aggregate == &ty && fields.is_empty()))
        );
        assert!(
            function
                .place_ty(&Place {
                    local: LocalId(0),
                    projection: vec![Projection::Field(0)],
                })
                .is_err()
        );
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn bindingless_empty_nominal_patterns_do_not_consume_the_place() {
        let (_module_names, modules) = fixture_modules();
        let ty = Ty::Struct(nominal(&modules, vec![]));
        let source = with_root(
            ty.clone(),
            vec![expr(ty.clone(), ir::ExprKind::Local(LocalId(0)))],
            vec![
                ir::Stmt::LetPattern {
                    value: ExprId(0),
                    ty: ty.clone(),
                    bindings: vec![],
                },
                ir::Stmt::LetPattern {
                    value: ExprId(0),
                    ty,
                    bindings: vec![],
                },
                ir::Stmt::Expr(ExprId(0)),
            ],
        );
        let function = nominal_lowered(&source);
        assert_eq!(root_reads(&function).len(), 1);
        assert!(
            matches!(root_reads(&function)[0], Operand::Move(place) if place.projection.is_empty())
        );
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn struct_initializer_evaluates_source_order_then_permutates_snapshots() {
        let (_module_names, modules) = fixture_modules();
        let ty = nominal(&modules, vec![Ty::Int(IntTy::I32), Ty::Int(IntTy::I32)]);
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
        let (_module_names, modules) = fixture_modules();
        let ty = ir::EnumTy::new(ir::EnumDef {
            source_decl: test_decl(),
            type_args: std::sync::Arc::from([]),
            id: ir::NominalAppId {
                snapshot: ir::SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "E".into(),
            module: modules.source_root().unwrap(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.source_root().unwrap()),
            variants: vec![
                ir::VariantDef {
                    name: "A".into(),
                    shape: ir::VariantShape::Struct,
                    fields: vec![
                        ir::FieldDef {
                            name: "a".into(),
                            ty: Ty::Int(IntTy::I32),
                            visibility: crate::compiler::ir::Visibility::Public,
                        },
                        ir::FieldDef {
                            name: "b".into(),
                            ty: Ty::Int(IntTy::I32),
                            visibility: crate::compiler::ir::Visibility::Public,
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
        let (_module_names, modules) = fixture_modules();
        let root = Ty::Struct(nominal(&modules, vec![Ty::Int(IntTy::I32)]));
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

    #[test]
    fn generic_tuple_templates_select_real_sparse_fields_without_enum_tags() {
        use crate::compiler::{frontend, ownership};
        for (source, expected_field) in [
            (
                "struct Wrap<T>(T);fn f<T>(p:Wrap<T>)->T{let Wrap(value)=p;value}fn main(){}",
                0,
            ),
            (
                "struct Pair<T,U>(T,U);fn f<T,U>(p:Pair<T,U>)->U{let Pair(..,value)=p;value}fn main(){}",
                1,
            ),
        ] {
            let budget = ir::PlanBudget::default();
            let program = frontend::parse(source).unwrap();
            let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let frontend::PublishedCallableBody::Source(publication) = plan
                .publish_callable_body(plan.function_decl(0).unwrap(), &budget)
                .unwrap()
            else {
                panic!("canonical source publication required")
            };
            let template = lower_source_template(&publication, &budget).unwrap();
            let function = template.function();
            let parameter = function.params[0];
            assert_eq!(function.blocks.iter().flat_map(|block| &block.statements).filter(|statement| matches!(
                &statement.value,
                Statement::Assign { value: Rvalue::Use(Operand::Move(selected)), .. }
                    if selected.local == parameter && selected.projection == [Projection::Field(expected_field)]
            )).count(), 1, "{source}");
            assert!(
                !function.blocks.iter().any(|block| matches!(
                    block.terminator.value,
                    Terminator::SwitchVariant { .. }
                ))
            );
            assert!(
                ownership::verify_template(template)
                    .unwrap()
                    .analysis()
                    .passed()
            );
        }
    }

    #[test]
    fn decision_predicates_test_nested_tags_and_scalars_before_any_selected_move() {
        let source = checked(
            "struct Cell{n:i32}enum Inner{A(bool,i32),B}enum Outer{A(Inner,Cell),B}fn f(p:Outer)->i32{if let Outer::A(Inner::A(true,7),cell)=p{cell.n}else{0}}fn main(){}",
            0,
        );
        let function = nominal_lowered(&source);
        let root = function.params[0];
        let (entry, first) = function
            .blocks
            .iter()
            .enumerate()
            .find(|(_, block)| {
                matches!(
                    &block.terminator.value, Terminator::SwitchVariant { place, .. }
                        if place.local == root && place.projection.is_empty()
                )
            })
            .unwrap();
        let comparisons: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.value {
                Statement::Assign {
                    destination,
                    value:
                        Rvalue::Binary {
                            op: BinaryOp::Eq,
                            left: Operand::Copy(inspected),
                            ..
                        },
                } if inspected.local == root => Some(destination.local),
                _ => None,
            })
            .collect();
        assert_eq!(comparisons.len(), 2);
        for comparison in comparisons {
            assert!(first.statements.iter().any(|statement| matches!(statement.value, Statement::StorageLive(local) if local == comparison)));
        }
        let mut current = BlockId(entry);
        for index in 0..4 {
            let block = &function.blocks[current.0];
            assert!(!block.statements.iter().any(|statement| matches!(
                &statement.value, Statement::Assign { value: Rvalue::Use(Operand::Move(selected)), .. } if selected.local == root
            )), "predicate {index} moved a binding before all tests succeeded");
            current = match &block.terminator.value {
                Terminator::SwitchVariant { then_block, .. } if index < 2 => *then_block,
                Terminator::Switch { then_block, .. } if index >= 2 => *then_block,
                other => panic!("unexpected predicate {index}: {other:?}"),
            };
        }
        assert!(function.blocks[current.0].statements.iter().any(|statement| matches!(
            &statement.value, Statement::Assign { value: Rvalue::Use(Operand::Move(selected)), .. }
                if selected.local == root && matches!(selected.projection.as_slice(), [Projection::Downcast(_), Projection::Field(1)])
        )));
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn decision_singleton_refinement_is_real_and_never_inputs_do_not_read_storage() {
        let source = checked(
            "enum E{A(bool)}fn f(p:E)->i32{if let E::A(true)=p{1}else{2}}fn main(){}",
            0,
        );
        let function = nominal_lowered(&source);
        assert!(
            function
                .blocks
                .iter()
                .any(|block| matches!(block.terminator.value, Terminator::SwitchVariant { .. }))
        );
        assert!(super::super::ownership::analyze(&function).passed());
        let source = checked(
            "enum E{A(bool),B}fn f()->i32{if let E::A(true)={return 7;}{1}else{2}}fn main(){}",
            0,
        );
        let function = nominal_lowered(&source);
        assert!(
            !function
                .blocks
                .iter()
                .any(|block| matches!(block.terminator.value, Terminator::SwitchVariant { .. }))
        );
        assert!(super::super::ownership::analyze(&function).passed());
    }

    #[test]
    fn product_partition_keeps_exact_selected_moves_and_checks_redundant_arm_types() {
        let source = checked(
            "struct Cell{n:i32}fn f(p:(bool,Cell))->i32{match p{(true,cell)=>cell.n,(false,cell)=>cell.n,_=>42}}fn main(){}",
            0,
        );
        let function = nominal_lowered(&source);
        let root = function.params[0];
        let moves: Vec<_> = function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match &statement.value {
                Statement::Assign {
                    value: Rvalue::Use(Operand::Move(selected)),
                    ..
                } if selected.local == root => Some(selected),
                _ => None,
            })
            .collect();
        assert_eq!(moves.len(), 2);
        assert!(
            moves
                .iter()
                .all(|selected| selected.projection == [Projection::Field(1)])
        );
        assert!(
            !function
                .blocks
                .iter()
                .any(|block| matches!(block.terminator.value, Terminator::SwitchVariant { .. }))
        );
        assert!(super::super::ownership::analyze(&function).passed());
        let program = super::super::frontend::parse("struct Cell{n:i32}fn f(p:(bool,Cell))->i32{match p{(true,cell)=>cell.n,(false,cell)=>cell.n,_=>false}}fn main(){}").unwrap();
        let signatures = super::super::frontend::signature_table(&program).unwrap();
        assert!(
            super::super::frontend::type_function_with_signatures(&program, 0, &signatures)
                .is_err()
        );
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
        assert_eq!(function.blocks.iter().filter(|block| matches!(&block.terminator.value, Terminator::Call {name, ..} if name == "crate::make")).count(), 1);
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
                            | Rvalue::LeadingZeros { value }
                            | Rvalue::IntCast { value, .. }
                            | Rvalue::FloatCast { value, .. }
                            | Rvalue::Unary { value, .. }
                            | Rvalue::Repeat { value, .. }
                            | Rvalue::ArrayToSlice { reference: value }
                            | Rvalue::SliceLen { slice: value } => live_operand(value, &live),
                            Rvalue::Binary { left, right, .. }
                            | Rvalue::SliceIndex {
                                slice: left,
                                index: right,
                            } => {
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
                            Rvalue::Println(_) | Rvalue::Output(_) => {}
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
                Terminator::Drop { cleanup, next } => {
                    assert!(live[cleanup.owner().local.0], "cleanup uses dead storage");
                    pending.push((*next, live));
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
                Terminator::Panic(_) | Terminator::Unreachable => {}
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

#[cfg(test)]
#[path = "cfg/slice_ownership_tests.rs"]
mod slice_ownership_tests;
