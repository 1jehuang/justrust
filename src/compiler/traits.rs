//! A single validated trait world, obligation engine and fallible facts authority.
//!
//! Raw schema callbacks are structural only. Neither raw schemas nor a caller's
//! claimed successful analysis can manufacture body facts or dispatch evidence.
use super::ir::{
    self, CallTarget, Callee, DeclId, DeclKind, GenericEnv, ParamEnv, ParamId, PlanBudget,
    Predicate, ReceiverMode, Signature, SignatureSubject, SnapshotId, TraitCall, TraitCx, TraitRef,
    Ty,
};
use std::sync::{Arc, Mutex};

mod catalog;
pub use catalog::{
    CallableProvider, CatalogDecl, CatalogItem, CoreCatalog, CoreCatalogBuilder, DeclOrigin,
    RecipeDescriptor, RecipeShapeInventory,
};

pub const MAX_PREDICATES: usize = 256;
const DIAGNOSTIC_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactsErrorKind {
    InvalidMetadata,
    Unsupported,
    Unproved,
    Ambiguous,
    Cycle,
    Resource,
}
#[derive(Clone, Debug)]
enum Diagnostic {
    Static(&'static str),
    Owned(Arc<String>),
}
#[derive(Clone, Debug)]
pub struct FactsError {
    kind: FactsErrorKind,
    diagnostic: Diagnostic,
}
impl FactsError {
    fn make(budget: &PlanBudget, kind: FactsErrorKind, message: &str) -> Self {
        let fallback = || Self {
            kind: FactsErrorKind::Resource,
            diagnostic: Diagnostic::Static(
                "trait planning resource boundary (diagnostic allowance exhausted)",
            ),
        };
        let mut end = message.len().min(DIAGNOSTIC_LIMIT.saturating_sub(32));
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = end < message.len();
        let bytes = end + if truncated { 24 } else { 0 };
        if budget.charge_work(bytes).is_err()
            || budget.charge_diagnostic_bytes(bytes).is_err()
            || budget
                .retain_bytes(
                    bytes
                        .saturating_add(2 * std::mem::size_of::<usize>())
                        .saturating_add(std::mem::size_of::<String>()),
                )
                .is_err()
        {
            return fallback();
        }
        let mut text = String::new();
        if text.try_reserve_exact(bytes).is_err() {
            return fallback();
        }
        text.push_str(&message[..end]);
        if truncated {
            text.push_str(" [diagnostic truncated]");
        }
        Self {
            kind,
            diagnostic: Diagnostic::Owned(Arc::new(text)),
        }
    }
    pub fn invalid(b: &PlanBudget, m: &str) -> Self {
        Self::make(b, FactsErrorKind::InvalidMetadata, m)
    }
    pub fn unsupported(b: &PlanBudget, m: &str) -> Self {
        Self::make(b, FactsErrorKind::Unsupported, m)
    }
    pub fn unproved(b: &PlanBudget, m: &str) -> Self {
        Self::make(b, FactsErrorKind::Unproved, m)
    }
    pub fn ambiguous(b: &PlanBudget, m: &str) -> Self {
        Self::make(b, FactsErrorKind::Ambiguous, m)
    }
    pub fn cycle(b: &PlanBudget, m: &str) -> Self {
        Self::make(b, FactsErrorKind::Cycle, m)
    }
    pub fn resource(b: &PlanBudget, m: &str) -> Self {
        Self::make(b, FactsErrorKind::Resource, m)
    }
    pub fn from_message(b: &PlanBudget, m: &str) -> Self {
        if b.charge_work(m.len()).is_err() {
            return Self::resource(b, "trait diagnostic classification resource boundary");
        }
        let kind = if m.contains("limit")
            || m.contains("resource")
            || m.contains("poison")
            || m.contains("overflow")
            || m.contains("exhaust")
        {
            FactsErrorKind::Resource
        } else if m.contains("unsupported") {
            FactsErrorKind::Unsupported
        } else {
            FactsErrorKind::InvalidMetadata
        };
        Self::make(b, kind, m)
    }
    pub fn kind(&self) -> FactsErrorKind {
        self.kind
    }
    pub fn is_resource(&self) -> bool {
        self.kind == FactsErrorKind::Resource
    }
    pub fn into_diagnostic(self, budget: &PlanBudget) -> Result<String> {
        let message = match &self.diagnostic {
            Diagnostic::Static(m) => *m,
            Diagnostic::Owned(m) => m.as_str(),
        };
        charged(budget, budget.charge_diagnostic_bytes(message.len()))?;
        charged(budget, budget.retain_bytes(message.len()))?;
        charged(budget, budget.charge_work(message.len()))?;
        let mut output = String::new();
        output
            .try_reserve_exact(message.len())
            .map_err(|_| Self::resource(budget, "diagnostic allocation failed"))?;
        output.push_str(message);
        Ok(output)
    }
    /// Consumes an already-paid owned message whenever uniquely held. The ONLY
    /// unmetered transport exception is the fixed, source-independent resource
    /// message below (under 80 bytes), approved for legacy String boundaries.
    /// Diagnostic exhaustion must not erase an error or reset semantic budgets.
    pub fn into_legacy_diagnostic(self, budget: &PlanBudget) -> String {
        const EXHAUSTED: &str =
            "trait planning resource limit exceeded (diagnostic transport exhausted)";
        let Self { kind, diagnostic } = self;
        let copied = match diagnostic {
            Diagnostic::Owned(shared) => match Arc::try_unwrap(shared) {
                Ok(text) => return text,
                Err(shared) => Self {
                    kind,
                    diagnostic: Diagnostic::Owned(shared),
                }
                .into_diagnostic(budget),
            },
            Diagnostic::Static(text) => Self {
                kind,
                diagnostic: Diagnostic::Static(text),
            }
            .into_diagnostic(budget),
        };
        match copied {
            Ok(text) => text,
            Err(_) => {
                // Best-effort storage accounting, not a renewed allowance. Rich
                // source/goal text can never enter this constant-size fallback.
                let _ = budget.retain_bytes(EXHAUSTED.len());
                let _ = budget.charge_work(EXHAUSTED.len());
                EXHAUSTED.to_owned()
            }
        }
    }
}
impl std::fmt::Display for FactsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.diagnostic {
            Diagnostic::Static(s) => f.write_str(s),
            Diagnostic::Owned(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for FactsError {}
type Result<T> = std::result::Result<T, FactsError>;
fn charged<T>(b: &PlanBudget, result: std::result::Result<T, String>) -> Result<T> {
    result.map_err(|e| FactsError::from_message(b, &e))
}
fn retain<T>(b: &PlanBudget, count: usize) -> Result<()> {
    let bytes = count
        .checked_mul(std::mem::size_of::<T>())
        .ok_or_else(|| FactsError::resource(b, "trait metadata size overflow"))?;
    let bytes = bytes
        .checked_add(2 * std::mem::size_of::<usize>())
        .ok_or_else(|| FactsError::resource(b, "trait allocation header size overflow"))?;
    charged(b, b.retain_bytes(bytes))
}
fn push<T>(b: &PlanBudget, values: &mut Vec<T>, value: T) -> Result<()> {
    // Retain a conservative old-plus-new relocation peak before Vec growth.
    retain::<T>(b, 2)?;
    if values.len() == values.capacity() {
        charged(b, b.charge_work(values.len()))?;
    }
    values
        .try_reserve_exact(1)
        .map_err(|_| FactsError::resource(b, "trait metadata allocation failed"))?;
    values.push(value);
    Ok(())
}
fn reserve_carriers<T>(b: &PlanBudget, values: &mut Vec<T>, additional: usize) -> Result<()> {
    let capacity = values
        .len()
        .checked_add(additional)
        .ok_or_else(|| FactsError::resource(b, "carrier capacity overflow"))?;
    retain::<T>(
        b,
        capacity
            .checked_mul(2)
            .ok_or_else(|| FactsError::resource(b, "carrier peak overflow"))?,
    )?;
    charged(b, b.charge_work(values.len()))?;
    values
        .try_reserve_exact(additional)
        .map_err(|_| FactsError::resource(b, "carrier allocation failed"))
}

#[derive(Clone, Debug)]
pub struct GenericSchema {
    pub env: Arc<GenericEnv>,
    pub signature: Signature,
    pub predicates: Arc<[Predicate]>,
}
fn clone_schema(schema: &GenericSchema, budget: &PlanBudget) -> Result<GenericSchema> {
    charged(budget, budget.charge_work(schema.signature.params.len()))?;
    retain::<Ty>(budget, schema.signature.params.len())?;
    Ok(schema.clone())
}
#[derive(Clone, Debug, Default)]
pub struct HeaderSet {
    pub traits: Arc<[TraitHeader]>,
    pub items: Arc<[TraitItemHeader]>,
    pub impls: Arc<[ImplHeader]>,
    pub callables: Arc<[CallableHeader]>,
}
#[derive(Clone, Debug)]
pub struct TraitHeader {
    pub declaration: DeclId,
    pub self_param: ParamId,
    pub env: Arc<GenericEnv>,
    pub items: Arc<[DeclId]>,
    pub supertraits: Arc<[TraitRef]>,
}
#[derive(Clone, Debug)]
pub struct TraitItemHeader {
    pub declaration: DeclId,
    pub trait_decl: DeclId,
    pub receiver: ReceiverMode,
    pub schema: GenericSchema,
    pub default: Option<DeclId>,
}
#[derive(Clone, Debug)]
pub struct ImplHeader {
    pub declaration: DeclId,
    pub trait_ref: TraitRef,
    pub schema_env: Arc<GenericEnv>,
    pub predicates: Arc<[Predicate]>,
    pub items: Arc<[(DeclId, DeclId)]>,
}
#[derive(Clone, Debug)]
pub struct CallableHeader {
    pub declaration: DeclId,
    pub schema: GenericSchema,
}
#[derive(Clone, Debug)]
pub struct SchemaDecl {
    pub declaration: DeclId,
    pub kind: DeclKind,
    pub own_env: Arc<ParamEnv>,
    pub env: Arc<GenericEnv>,
    pub owner: Option<DeclId>,
    pub module: Arc<[String]>,
    pub public: bool,
    pub receiver: ReceiverMode,
    pub region_contract_mode: RegionContractMode,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionContractMode {
    Positioned,
    LegacyUnnamedFreeNoReturn,
}
#[derive(Clone, Debug)]
pub struct InstantiatedSchema {
    pub signature: Signature,
    pub predicates: Arc<[Predicate]>,
}
pub trait SchemaCx {
    fn budget(&self) -> &PlanBudget;
    fn registry_snapshot(&self) -> SnapshotId;
    fn core_catalog(&self) -> &Arc<CoreCatalog>;
    fn declaration_info(&self, id: DeclId) -> Result<SchemaDecl>;
    fn declaration_schema(&self, id: DeclId) -> Result<GenericSchema>;
    fn source_region_env(&self, id: DeclId) -> Result<Arc<ir::SourceRegionEnv>>;
    fn instantiate_schema(
        &self,
        schema: &GenericSchema,
        args: &[Ty],
        caller: &GenericEnv,
        budget: &PlanBudget,
    ) -> Result<InstantiatedSchema>;
    fn validate_nominal(&self, ty: &Ty, env: &GenericEnv, budget: &PlanBudget) -> Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WorldId(SnapshotId);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FactsPhase {
    Schema,
    Universal,
    Closed,
}
#[derive(Debug)]
pub struct PredicateEnv {
    world: WorldId,
    index: usize,
    declaration: Option<DeclId>,
    env: Arc<GenericEnv>,
    predicates: Arc<[Predicate]>,
}
impl PredicateEnv {
    pub fn predicates(&self) -> &[Predicate] {
        &self.predicates
    }
    pub fn env(&self) -> &Arc<GenericEnv> {
        &self.env
    }
    pub fn declaration(&self) -> Option<DeclId> {
        self.declaration
    }
}
#[derive(Clone, Debug)]
enum FactsSubject {
    SchemaItem(DeclId),
    UniversalCallable(DeclId),
    ClosedInstance(CallTarget),
}
#[derive(Clone, Debug)]
pub struct ValidatedFacts {
    world: Arc<ValidatedTraitWorld>,
    subject: FactsSubject,
    predicates: Arc<PredicateEnv>,
    requirements: Arc<[ProofEvidence]>,
}
impl ValidatedFacts {
    pub fn world(&self) -> &Arc<ValidatedTraitWorld> {
        &self.world
    }
    pub fn service(&self) -> &dyn TraitCx {
        self.world.as_ref()
    }
    pub fn env(&self) -> &Arc<GenericEnv> {
        self.predicates.env()
    }
    pub fn predicates(&self) -> &Arc<PredicateEnv> {
        &self.predicates
    }
    pub fn phase(&self) -> FactsPhase {
        match self.subject {
            FactsSubject::SchemaItem(_) => FactsPhase::Schema,
            FactsSubject::UniversalCallable(_) => FactsPhase::Universal,
            FactsSubject::ClosedInstance(_) => FactsPhase::Closed,
        }
    }
    pub fn budget(&self) -> &PlanBudget {
        &self.world.budget
    }
    pub fn declaration(&self) -> DeclId {
        match &self.subject {
            FactsSubject::SchemaItem(d) | FactsSubject::UniversalCallable(d) => *d,
            FactsSubject::ClosedInstance(t) => t.declaration,
        }
    }
    pub fn target(&self) -> Option<&CallTarget> {
        if let FactsSubject::ClosedInstance(t) = &self.subject {
            Some(t)
        } else {
            None
        }
    }
    pub fn registry_snapshot(&self) -> SnapshotId {
        self.world.registry
    }
    pub fn validate_signature(
        &self,
        subject: SignatureSubject<'_>,
        signature: &Signature,
    ) -> Result<()> {
        TraitCx::validate_signature(self.world.as_ref(), subject, signature, self)
    }
    pub fn callee_receiver(&self, callee: &Callee) -> Result<ReceiverMode> {
        TraitCx::callee_receiver(self.world.as_ref(), callee, self)
    }
    pub fn carrier_contract(
        &self,
        subject: SignatureSubject<'_>,
    ) -> Result<Arc<CarrierContractView>> {
        self.world.carrier_contract(subject, self)
    }
    pub fn source_region_env(&self, declaration: DeclId) -> Result<Arc<ir::SourceRegionEnv>> {
        self.world.check_facts(self, false)?;
        self.world.info(declaration)?;
        Ok(self
            .world
            .declaration_carriers(declaration)?
            .source_regions
            .clone())
    }
    pub fn validate_ty(&self, ty: &Ty) -> Result<()> {
        self.validate_ty_with_budget(ty, self.budget())
    }
    pub fn projected_ty<'a>(&self, root: &'a Ty, path: &[ir::Projection]) -> Result<&'a Ty> {
        self.validate_ty(root)?;
        charged(self.budget(), self.budget().charge_work(path.len()))?;
        charged(self.budget(), ir::projected_ty(root, path))
    }
    pub fn validate_ty_with_budget(&self, ty: &Ty, budget: &PlanBudget) -> Result<()> {
        if !self.budget().same_authority(budget) {
            return Err(FactsError::invalid(
                budget,
                "facts changed planning budget authority",
            ));
        }
        let maybe_unsized = if let FactsSubject::SchemaItem(item) = self.subject {
            let header = self.world.item_header(item)?;
            Some(self.world.trait_header(header.trait_decl)?.self_param)
        } else {
            None
        };
        self.world
            .validate_ty(ty, self.env(), maybe_unsized, true, 0)
    }
    pub fn must_copy(&self, ty: &Ty) -> Result<bool> {
        if self.phase() == FactsPhase::Schema {
            return Err(FactsError::invalid(
                self.budget(),
                "schema facts cannot grant executable Copy permission",
            ));
        }
        self.validate_ty(ty)?;
        let predicate = Predicate::Implements(TraitRef {
            declaration: self.world.catalog.id(CatalogItem::Copy),
            self_ty: ty.clone(),
            args: Arc::from([]),
        });
        Ok(matches!(
            self.world.prove_goal(&predicate, self)?,
            ProofOutcome::Proved(_)
        ))
    }
}
#[derive(Clone, Debug)]
enum ProofOrigin {
    Assumption,
    Builtin {
        item: CatalogItem,
    },
    Implementation {
        declaration: DeclId,
        arguments: Arc<[Ty]>,
    },
}
#[derive(Clone, Debug)]
struct ProofRecord {
    world: WorldId,
    environment: usize,
    phase: FactsPhase,
    goal: Predicate,
    origin: ProofOrigin,
}
#[derive(Clone, Debug)]
pub struct ProofEvidence(Arc<ProofRecord>);
#[derive(Clone, Debug)]
pub enum ProofOutcome {
    Proved(ProofEvidence),
    NotProven,
}
#[derive(Clone, Debug)]
pub struct DirectCallProof {
    target: CallTarget,
    signature: Signature,
    evidence: Arc<[ProofEvidence]>,
}
impl DirectCallProof {
    pub fn target(&self) -> &CallTarget {
        &self.target
    }
    pub fn signature(&self) -> &Signature {
        &self.signature
    }
}
#[derive(Clone, Debug)]
pub struct ResolvedTraitCall {
    callee: Callee,
    signature: Signature,
    _evidence: ProofEvidence,
    _refinement: Option<Arc<ReturnRefinementProof>>,
    _topology: Option<Arc<SignatureTopologyProof>>,
}
impl ResolvedTraitCall {
    pub fn callee(&self) -> &Callee {
        &self.callee
    }
    pub fn signature(&self) -> &Signature {
        &self.signature
    }
}
#[derive(Clone, Debug)]
struct GoalKey {
    environment: usize,
    phase: FactsPhase,
    goal: Predicate,
}
#[derive(Clone, Debug)]
enum MemoState {
    Active,
    Complete(ProofOutcome),
    Errored(FactsError),
}
#[derive(Clone, Debug)]
struct MemoEntry {
    key: GoalKey,
    state: MemoState,
}

#[derive(Clone, Copy)]
struct ProofContext<'a> {
    predicates: &'a PredicateEnv,
    phase: FactsPhase,
}
impl<'a> ProofContext<'a> {
    fn from_facts(facts: &'a ValidatedFacts) -> Self {
        Self {
            predicates: &facts.predicates,
            phase: facts.phase(),
        }
    }
    fn env(self) -> &'a Arc<GenericEnv> {
        &self.predicates.env
    }
}

#[derive(Clone, Debug)]
struct ReturnRefinementProof {
    world: WorldId,
    item: DeclId,
    implementation: DeclId,
    required: Arc<ir::RefContract>,
    provided: Arc<ir::RefContract>,
    topology: Option<Arc<SignatureTopologyProof>>,
}

#[derive(Clone, Debug)]
struct CarrierRegionBinding {
    carrier: ir::ParameterCarrier,
    region: ir::RegionParamId,
    inputs: Arc<[ir::InputReference]>,
    results: Arc<[ir::ReferencePosition]>,
}
#[derive(Clone, Debug)]
struct SignatureTopologyProof {
    world: WorldId,
    registry: SnapshotId,
    environment: usize,
    phase: FactsPhase,
    declaration: DeclId,
    arguments: Arc<[Ty]>,
    schema: Arc<ir::SignatureOriginSchema>,
    fixed_regions: Arc<[ir::RegionParamId]>,
    carriers: Arc<[CarrierRegionBinding]>,
    realized: Arc<ir::RefContract>,
    budget: PlanBudget,
}

/// Structural declaration transport only. Actual loans and occurrence-local
/// region solutions belong to the borrow plan, never this world-owned view.
#[derive(Debug)]
pub struct CarrierContractView {
    world: WorldId,
    registry: SnapshotId,
    environment: usize,
    phase: FactsPhase,
    declaration: DeclId,
    arguments: Arc<[Ty]>,
    original_env: Arc<GenericEnv>,
    original_contract: ir::CarrierContract,
    contract: ir::CarrierContract,
    origin: Arc<ir::SignatureOriginSchema>,
    fixed: Arc<ir::RefContract>,
    regions: ir::CarrierRegionSchema,
    source_regions: Arc<ir::SourceRegionEnv>,
    components: Arc<[ir::CarrierComponentMapping]>,
    nominals: Arc<[ir::NominalCarrierMapping]>,
    budget: PlanBudget,
}
impl CarrierContractView {
    pub fn original_contract(&self) -> &ir::CarrierContract {
        &self.original_contract
    }
    pub fn contract(&self) -> &ir::CarrierContract {
        &self.contract
    }
    pub fn origin_schema(&self) -> &Arc<ir::SignatureOriginSchema> {
        &self.origin
    }
    pub fn ref_contract(&self) -> &Arc<ir::RefContract> {
        &self.fixed
    }
    pub fn region_schema(&self) -> &ir::CarrierRegionSchema {
        &self.regions
    }
    pub fn source_region_env(&self) -> &Arc<ir::SourceRegionEnv> {
        &self.source_regions
    }
    pub fn component_mappings(&self) -> &[ir::CarrierComponentMapping] {
        &self.components
    }
    pub fn nominal_mappings(&self) -> &[ir::NominalCarrierMapping] {
        &self.nominals
    }
    pub fn declaration(&self) -> DeclId {
        self.declaration
    }
    pub fn arguments(&self) -> &[Ty] {
        &self.arguments
    }
    pub fn parameter_actual(&self, parameter: ParamId) -> Result<Option<(usize, &Ty)>> {
        for (index, candidate) in self.original_env.ordered_params().enumerate() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if candidate == parameter {
                return Ok(self.arguments.get(index).map(|ty| (index, ty)));
            }
        }
        Ok(None)
    }
    pub fn original_env(&self) -> &Arc<GenericEnv> {
        &self.original_env
    }
    pub fn validate_for(&self, facts: &ValidatedFacts) -> Result<()> {
        facts.world.check_facts(facts, false)?;
        facts.world.check_context(ProofContext::from_facts(facts))?;
        if self.world != facts.world.id
            || self.registry != facts.registry_snapshot()
            || self.environment != facts.predicates.index
            || self.phase != facts.phase()
            || !self.budget.same_authority(facts.budget())
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "carrier view has foreign context authority",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct DeclarationCarriers {
    declaration: DeclId,
    contract: ir::CarrierContract,
    regions: ir::CarrierRegionSchema,
    source_regions: Arc<ir::SourceRegionEnv>,
}
struct ExpandedCarrier {
    location: ir::CarrierLocation,
    component: Option<ir::CarrierLocation>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeUse {
    Root,
    Referent { depth: u8, shared: bool },
    Aggregate,
    Actual,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeNode {
    Struct(ir::NominalAppId, usize),
    Enum(ir::NominalAppId, usize),
    Tuple(usize, usize),
    Array(usize, usize),
    Reference(usize, ir::BorrowKind),
}
#[derive(Default)]
struct TypeWalk {
    complete: Vec<((TypeNode, TypeUse), usize)>,
    reservations: Vec<ir::PlanReservation>,
    visits: usize,
    maybe_unsized: Option<ParamId>,
    has_parameters: bool,
}
impl TypeWalk {
    fn key(ty: &Ty) -> Option<TypeNode> {
        match ty {
            Ty::Struct(t) => Some(TypeNode::Struct(
                t.def().id,
                t.def() as *const ir::StructDef as usize,
            )),
            Ty::Enum(t) => Some(TypeNode::Enum(
                t.def().id,
                t.def() as *const ir::EnumDef as usize,
            )),
            Ty::Tuple(t) => Some(TypeNode::Tuple(t.as_ptr() as usize, t.len())),
            Ty::Array { element, len } => {
                Some(TypeNode::Array(Arc::as_ptr(element) as usize, *len))
            }
            Ty::Ref { referent, kind } => {
                Some(TypeNode::Reference(Arc::as_ptr(referent) as usize, *kind))
            }
            _ => None,
        }
    }
    fn lookup(&self, key: (TypeNode, TypeUse), budget: &PlanBudget) -> Result<Option<usize>> {
        for (old, height) in &self.complete {
            charged(budget, budget.charge_work(1))?;
            if *old == key {
                return Ok(Some(*height));
            }
        }
        Ok(None)
    }
    fn insert(
        &mut self,
        key: (TypeNode, TypeUse),
        height: usize,
        budget: &PlanBudget,
    ) -> Result<()> {
        let reservation = charged(
            budget,
            budget.reserve_bytes(
                2 * std::mem::size_of::<((TypeNode, TypeUse), usize)>()
                    + 2 * std::mem::size_of::<ir::PlanReservation>()
                    + 32,
            ),
        )?;
        let relocation = self
            .complete
            .len()
            .checked_add(self.reservations.len())
            .ok_or_else(|| FactsError::resource(budget, "type memo relocation overflow"))?;
        charged(budget, budget.charge_work(relocation))?;
        self.complete
            .try_reserve_exact(1)
            .map_err(|_| FactsError::resource(budget, "type memo allocation failed"))?;
        self.reservations
            .try_reserve_exact(1)
            .map_err(|_| FactsError::resource(budget, "type memo reservation allocation failed"))?;
        self.complete.push((key, height));
        self.reservations.push(reservation);
        Ok(())
    }
}

pub struct ValidatedTraitWorld {
    id: WorldId,
    registry: SnapshotId,
    schema: Arc<dyn SchemaCx + Send + Sync>,
    catalog: Arc<CoreCatalog>,
    budget: PlanBudget,
    headers: HeaderSet,
    environments: Vec<Arc<PredicateEnv>>,
    empty: Arc<PredicateEnv>,
    memo: Mutex<Vec<MemoEntry>>,
    requirements: Vec<(DeclId, Arc<[Predicate]>)>,
    origins: Vec<(DeclId, Arc<ir::SignatureOriginSchema>)>,
    carriers: Vec<DeclarationCarriers>,
}

impl ValidatedTraitWorld {
    pub fn build(
        mut headers: HeaderSet,
        schema: Arc<dyn SchemaCx + Send + Sync>,
        budget: &PlanBudget,
    ) -> Result<Arc<Self>> {
        if !schema.budget().same_authority(budget) {
            return Err(FactsError::invalid(
                budget,
                "trait schema changed planning budget authority",
            ));
        }
        let registry = schema.registry_snapshot();
        let catalog = schema.core_catalog().clone();
        if catalog.snapshot() != registry || !catalog.budget().same_authority(budget) {
            return Err(FactsError::invalid(
                budget,
                "canonical catalog registry or ledger mismatch",
            ));
        }
        // Canonical rows come ONLY from the retained catalog. Supplied rows of
        // the same identity are checked against it, never replaced or remapped.
        let mut traits = Vec::new();
        let mut items = Vec::new();
        let mut callables = Vec::new();
        for h in headers.traits.iter() {
            push(budget, &mut traits, h.clone())?;
        }
        for h in catalog.headers().traits.iter() {
            charged(budget, budget.charge_work(traits.len()))?;
            if !traits.iter().any(|v| v.declaration == h.declaration) {
                push(budget, &mut traits, h.clone())?;
            }
        }
        for h in headers.items.iter() {
            push(
                budget,
                &mut items,
                TraitItemHeader {
                    schema: clone_schema(&h.schema, budget)?,
                    declaration: h.declaration,
                    trait_decl: h.trait_decl,
                    receiver: h.receiver,
                    default: h.default,
                },
            )?;
        }
        for h in catalog.headers().items.iter() {
            charged(budget, budget.charge_work(items.len()))?;
            if !items.iter().any(|v| v.declaration == h.declaration) {
                push(
                    budget,
                    &mut items,
                    TraitItemHeader {
                        schema: clone_schema(&h.schema, budget)?,
                        declaration: h.declaration,
                        trait_decl: h.trait_decl,
                        receiver: h.receiver,
                        default: h.default,
                    },
                )?;
            }
        }
        for h in headers.callables.iter() {
            push(
                budget,
                &mut callables,
                CallableHeader {
                    declaration: h.declaration,
                    schema: clone_schema(&h.schema, budget)?,
                },
            )?;
        }
        for h in catalog.headers().callables.iter() {
            charged(budget, budget.charge_work(callables.len()))?;
            if !callables.iter().any(|v| v.declaration == h.declaration) {
                push(
                    budget,
                    &mut callables,
                    CallableHeader {
                        declaration: h.declaration,
                        schema: clone_schema(&h.schema, budget)?,
                    },
                )?;
            }
        }
        headers.traits = traits.into();
        headers.items = items.into();
        headers.callables = callables.into();
        // Check identities before invoking any potentially expensive raw callback.
        for id in headers
            .traits
            .iter()
            .map(|h| h.declaration)
            .chain(headers.items.iter().map(|h| h.declaration))
            .chain(headers.impls.iter().map(|h| h.declaration))
            .chain(headers.callables.iter().map(|h| h.declaration))
        {
            charged(budget, budget.charge_work(1))?;
            if id.snapshot != registry {
                return Err(FactsError::invalid(
                    budget,
                    "trait header belongs to a foreign registry",
                ));
            }
        }
        retain::<Self>(budget, 1)?;
        let id = WorldId(charged(budget, SnapshotId::allocate())?);
        let empty_env = Arc::new(charged(budget, GenericEnv::new(&[], budget))?);
        retain::<PredicateEnv>(budget, 1)?;
        let empty = Arc::new(PredicateEnv {
            world: id,
            index: usize::MAX,
            declaration: None,
            env: empty_env,
            predicates: Arc::from([]),
        });
        let mut world = Self {
            id,
            registry,
            schema,
            catalog,
            budget: budget.clone(),
            headers,
            environments: Vec::new(),
            empty,
            memo: Mutex::new(Vec::new()),
            requirements: Vec::new(),
            origins: Vec::new(),
            carriers: Vec::new(),
        };
        world.validate_headers()?;
        world.validate_supertrait_graph()?;
        world.build_declared_environments()?;
        world.build_origin_schemas()?;
        for h in world.headers.impls.iter() {
            world.validate_impl(h)?;
        }
        for h in world.headers.impls.iter() {
            let env = world.predicate_env(h.declaration)?;
            let context = ProofContext {
                predicates: &env,
                phase: FactsPhase::Universal,
            };
            world.require_impl_wf(h, &h.trait_ref.self_ty, context, &mut Vec::new())?;
        }
        world.build_environments()?;
        if world
            .headers
            .impls
            .iter()
            .any(|h| h.trait_ref.declaration == world.catalog.id(CatalogItem::Drop))
        {
            return Err(FactsError::unsupported(
                budget,
                "unsupported: canonical Drop destructor execution",
            ));
        }
        Ok(Arc::new(world))
    }
    pub fn registry_snapshot(&self) -> SnapshotId {
        self.registry
    }
    pub fn budget(&self) -> &PlanBudget {
        &self.budget
    }
    pub(super) fn signatures_equal(&self, left: &Signature, right: &Signature) -> Result<bool> {
        self.eq_signature(left, right)
    }
    pub(super) fn types_equal(&self, left: &[Ty], right: &[Ty]) -> Result<bool> {
        self.eq_types(left, right, 0)
    }
    fn info(&self, id: DeclId) -> Result<SchemaDecl> {
        charged(&self.budget, self.budget.charge_work(1))?;
        if id.snapshot != self.registry {
            return Err(FactsError::invalid(
                &self.budget,
                "foreign declaration identity in trait world",
            ));
        }
        let info = self.schema.declaration_info(id)?;
        if let Some(canonical) = self.catalog.declaration(id) {
            let c = &canonical.info;
            if info.kind != c.kind
                || info.owner != c.owner
                || info.receiver != c.receiver
                || info.public != c.public
                || info.region_contract_mode != c.region_contract_mode
                || !self.same_env(&info.env, &c.env)?
                || info.own_env.binder() != c.own_env.binder()
                || info.own_env.arity() != c.own_env.arity()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "catalog declaration metadata spoofed by registry",
                ));
            }
        } else if id.index < self.catalog.source_start() || id.index >= self.catalog.source_end() {
            return Err(FactsError::invalid(
                &self.budget,
                "source declaration overlaps or escapes frozen catalog layout",
            ));
        }
        if info.declaration != id || info.own_env.binder().declaration != id {
            return Err(FactsError::invalid(
                &self.budget,
                "raw registry returned nonauthoritative declaration metadata",
            ));
        }
        self.check_env(&info.env)?;
        Ok(info)
    }
    fn check_env(&self, env: &GenericEnv) -> Result<()> {
        if env.total_arity() > self.budget.limits().params {
            return Err(FactsError::resource(
                &self.budget,
                "generic parameter limit exceeded",
            ));
        }
        for scope in env.scopes() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if scope.binder().declaration.snapshot != self.registry {
                return Err(FactsError::invalid(
                    &self.budget,
                    "foreign binder in trait environment",
                ));
            }
        }
        Ok(())
    }
    fn same_env(&self, left: &GenericEnv, right: &GenericEnv) -> Result<bool> {
        charged(
            &self.budget,
            self.budget
                .charge_work(left.scopes().len().saturating_add(right.scopes().len())),
        )?;
        Ok(left.scopes().len() == right.scopes().len()
            && left
                .scopes()
                .iter()
                .zip(right.scopes())
                .all(|(a, b)| a.binder() == b.binder() && a.arity() == b.arity()))
    }
    fn same_parameters(&self, left: &GenericEnv, right: &GenericEnv) -> Result<bool> {
        charged(
            &self.budget,
            self.budget
                .charge_work(left.total_arity().saturating_add(right.total_arity())),
        )?;
        Ok(left.ordered_params().eq(right.ordered_params()))
    }
    fn trait_header(&self, id: DeclId) -> Result<&TraitHeader> {
        for h in self.headers.traits.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if h.declaration == id {
                return Ok(h);
            }
        }
        Err(FactsError::invalid(
            &self.budget,
            "trait declaration is not in the validated world",
        ))
    }
    fn item_header(&self, id: DeclId) -> Result<&TraitItemHeader> {
        for h in self.headers.items.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if h.declaration == id {
                return Ok(h);
            }
        }
        Err(FactsError::invalid(
            &self.budget,
            "required trait item is not in the validated world",
        ))
    }
    fn callable_header(&self, id: DeclId) -> Result<&CallableHeader> {
        for h in self.headers.callables.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if h.declaration == id {
                return Ok(h);
            }
        }
        Err(FactsError::invalid(
            &self.budget,
            "declaration has no executable callable schema",
        ))
    }
    fn predicate_env(&self, id: DeclId) -> Result<Arc<PredicateEnv>> {
        for env in &self.environments {
            charged(&self.budget, self.budget.charge_work(1))?;
            if env.declaration == Some(id) {
                return Ok(env.clone());
            }
        }
        Err(FactsError::invalid(
            &self.budget,
            "declaration has no authoritative predicate environment",
        ))
    }
    fn validate_ref_in(
        &self,
        goal: &TraitRef,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
    ) -> Result<()> {
        let declaration = self.info(goal.declaration)?;
        if declaration.kind != DeclKind::Trait {
            return Err(FactsError::invalid(
                &self.budget,
                "trait reference does not name a trait",
            ));
        }
        self.trait_header(goal.declaration)?;
        if !goal.args.is_empty() {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: generic trait arguments",
            ));
        }
        self.validate_ty(&goal.self_ty, env, maybe_unsized, true, 0)?;
        Ok(())
    }
    fn validate_predicates(
        &self,
        predicates: &[Predicate],
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
    ) -> Result<()> {
        let mut canonical = Vec::new();
        for predicate in predicates {
            let Predicate::Implements(goal) = predicate;
            self.validate_ref_in(goal, env, maybe_unsized)?;
            self.append_predicate(&mut canonical, predicate)?;
        }
        Ok(())
    }
    fn validate_ty(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
        top: bool,
        depth: usize,
    ) -> Result<()> {
        let mut walk = TypeWalk {
            maybe_unsized,
            ..TypeWalk::default()
        };
        self.validate_ty_inner(
            ty,
            env,
            depth,
            if top {
                TypeUse::Root
            } else {
                TypeUse::Aggregate
            },
            &mut walk,
        )
        .map(|_| ())
    }
    fn validate_actual(&self, ty: &Ty, env: &GenericEnv) -> Result<()> {
        self.validate_ty_inner(ty, env, 0, TypeUse::Actual, &mut TypeWalk::default())
            .map(|_| ())
    }
    fn validate_ty_inner(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
        usage: TypeUse,
        walk: &mut TypeWalk,
    ) -> Result<usize> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        walk.visits = walk
            .visits
            .checked_add(1)
            .ok_or_else(|| FactsError::resource(&self.budget, "type walk visit count overflow"))?;
        let key = TypeWalk::key(ty).map(|node| (node, usage));
        if let Some(key) = key
            && let Some(height) = walk.lookup(key, &self.budget)?
        {
            charged(
                &self.budget,
                self.budget.check_depth(depth.saturating_add(height)),
            )?;
            return Ok(height);
        }
        let maybe_unsized = walk.maybe_unsized;
        let mut height = 0usize;
        let mut child = |t: &Ty, next_use: TypeUse| -> Result<()> {
            height = height.max(
                self.validate_ty_inner(t, env, depth + 1, next_use, walk)?
                    .saturating_add(1),
            );
            Ok(())
        };
        match ty {
            Ty::Param(param) => {
                walk.has_parameters = true;
                charged(&self.budget, self.budget.charge_work(env.scopes().len()))?;
                if !env.contains(*param) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "type parameter outside authoritative environment",
                    ));
                }
                if Some(*param) == maybe_unsized
                    && !matches!(usage, TypeUse::Root | TypeUse::Referent { .. })
                {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: possibly-unsized trait Self in a Sized aggregate position",
                    ));
                }
            }
            Ty::Tuple(parts) => {
                if parts.is_empty() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "empty tuple must have canonical Unit type",
                    ));
                }
                for t in parts.iter() {
                    child(
                        t,
                        if usage == TypeUse::Actual {
                            TypeUse::Actual
                        } else {
                            TypeUse::Aggregate
                        },
                    )?;
                }
            }
            Ty::Array { element, len } => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err(FactsError::resource(
                        &self.budget,
                        "array length limit exceeded",
                    ));
                }
                child(
                    element,
                    if usage == TypeUse::Actual {
                        TypeUse::Actual
                    } else {
                        TypeUse::Aggregate
                    },
                )?;
            }
            Ty::Ref { kind, referent } => {
                let depth = match usage {
                    TypeUse::Root | TypeUse::Aggregate => 1,
                    TypeUse::Referent { depth, .. } => depth.checked_add(1).ok_or_else(|| {
                        FactsError::resource(&self.budget, "reference value depth overflow")
                    })?,
                    TypeUse::Actual => {
                        return Err(FactsError::unsupported(
                            &self.budget,
                            "unsupported: reference-containing aggregate executable actual",
                        ));
                    }
                };
                charged(&self.budget, self.budget.check_depth(usize::from(depth)))?;
                child(
                    referent,
                    TypeUse::Referent {
                        depth,
                        shared: *kind == ir::BorrowKind::Shared,
                    },
                )?;
            }
            Ty::Struct(t) => {
                if t.def().id.snapshot != self.registry
                    || t.def().source_decl.snapshot != self.registry
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "foreign nominal identity",
                    ));
                }
                self.schema.validate_nominal(ty, env, &self.budget)?;
                for t in t.def().type_args.iter() {
                    child(
                        t,
                        if usage == TypeUse::Actual {
                            TypeUse::Actual
                        } else {
                            TypeUse::Root
                        },
                    )?;
                }
                for field in t.def().fields.iter() {
                    child(&field.ty, TypeUse::Aggregate)?;
                }
            }
            Ty::Enum(t) => {
                if t.def().id.snapshot != self.registry
                    || t.def().source_decl.snapshot != self.registry
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "foreign nominal identity",
                    ));
                }
                self.schema.validate_nominal(ty, env, &self.budget)?;
                for t in t.def().type_args.iter() {
                    child(
                        t,
                        if usage == TypeUse::Actual {
                            TypeUse::Actual
                        } else {
                            TypeUse::Root
                        },
                    )?;
                }
                for variant in t.def().variants.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    for field in variant.fields.iter() {
                        child(&field.ty, TypeUse::Aggregate)?;
                    }
                }
            }
            Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Never => {}
        }
        if let Some(key) = key {
            walk.insert(key, height, &self.budget)?;
        }
        Ok(height)
    }
    fn validate_signature_shape(
        &self,
        signature: &Signature,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
    ) -> Result<()> {
        // This validates formation and stored metadata, never deriving new
        // origins from coincident types after specialization.
        let mut walk = TypeWalk {
            maybe_unsized,
            ..TypeWalk::default()
        };
        for t in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            self.validate_ty_inner(t, env, 0, TypeUse::Root, &mut walk)?;
        }
        charged(
            &self.budget,
            ir::validate_stored_ref_contract(
                &signature.params,
                &signature.result,
                &signature.ref_contract,
                &self.budget,
            ),
        )
    }
    fn validate_schema_signature(
        &self,
        declaration: DeclId,
        signature: &Signature,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
    ) -> Result<()> {
        let info = self.info(declaration)?;
        if info.region_contract_mode == RegionContractMode::LegacyUnnamedFreeNoReturn {
            let c = &signature.ref_contract;
            if info.kind != DeclKind::Function
                || info.owner.is_some()
                || info.receiver != ReceiverMode::None
                || matches!(signature.result, Ty::Ref { .. })
                || c.region_count != 0
                || !c.input_regions.is_empty()
                || !c.results.is_empty()
                || !c.outlives.is_empty()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "legacy erased reference contract does not qualify for its declaration",
                ));
            }
            for ty in &signature.params {
                charged(&self.budget, self.budget.charge_work(3))?;
                if charged(&self.budget, ir::root_reference_layers(ty))? > 1 {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "legacy erased declaration contains a nested reference",
                    ));
                }
            }
            return self.validate_signature_shape(signature, env, maybe_unsized);
        }
        self.validate_position_coverage(signature)?;
        let normalized = charged(
            &self.budget,
            ir::normalize_ref_contract(&signature.ref_contract, &self.budget),
        )?;
        if !self.eq_contract(&signature.ref_contract, &normalized)? {
            return Err(FactsError::invalid(
                &self.budget,
                "noncanonical declaration region contract",
            ));
        }
        let derived = charged(
            &self.budget,
            ir::derive_return_contracts(
                &signature.params,
                &signature.result,
                &normalized,
                &self.budget,
            ),
        )?;
        for row in derived.iter().chain(signature.ref_contract.results.iter()) {
            self.charge_returns(&row.returns)?;
        }
        charged(
            &self.budget,
            self.budget.charge_work(derived.len().saturating_add(1)),
        )?;
        if derived != signature.ref_contract.results {
            return Err(FactsError::invalid(
                &self.budget,
                "declaration return origins are not the complete rigid canonical contract",
            ));
        }
        self.validate_signature_shape(signature, env, maybe_unsized)
    }
    fn validate_declaration_schema(
        &self,
        declaration: DeclId,
        header: &GenericSchema,
    ) -> Result<()> {
        let canonical = self.schema.declaration_schema(declaration)?;
        if !self.same_env(&canonical.env, &header.env)?
            || !self.eq_signature(&canonical.signature, &header.signature)?
            || canonical.predicates.len() != header.predicates.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "header disagrees with authoritative original declaration schema",
            ));
        }
        for (a, b) in canonical.predicates.iter().zip(header.predicates.iter()) {
            if !self.eq_predicate(a, b)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "header predicates disagree with original declaration schema",
                ));
            }
        }
        if let Some(trusted) = self
            .catalog
            .declaration(declaration)
            .and_then(|d| d.schema.as_ref())
        {
            if !self.same_env(&trusted.env, &canonical.env)?
                || !self.eq_signature(&trusted.signature, &canonical.signature)?
                || trusted.predicates.len() != canonical.predicates.len()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "raw adapter disagrees with immutable catalog schema",
                ));
            }
            for (a, b) in trusted.predicates.iter().zip(canonical.predicates.iter()) {
                if !self.eq_predicate(a, b)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "raw adapter changed catalog obligations",
                    ));
                }
            }
        }
        Ok(())
    }
    fn validate_position_coverage(&self, signature: &Signature) -> Result<()> {
        let contract = &signature.ref_contract;
        let mut row = 0usize;
        for (param, ty) in signature.params.iter().enumerate() {
            charged(&self.budget, self.budget.charge_work(3))?;
            let count = charged(&self.budget, ir::root_reference_layers(ty))?;
            for depth in 0..count {
                charged(&self.budget, self.budget.charge_work(1))?;
                let expected = ir::InputReference {
                    param,
                    position: ir::ReferencePosition { depth: depth as u8 },
                };
                if contract.input_regions.get(row).map(|r| r.input) != Some(expected) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "reference contract input positions are not exact and dense",
                    ));
                }
                row += 1;
            }
        }
        charged(&self.budget, self.budget.charge_work(3))?;
        let results = charged(&self.budget, ir::root_reference_layers(&signature.result))?;
        if row != contract.input_regions.len() || results != contract.results.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "reference contract has missing or extra positions",
            ));
        }
        for (depth, result) in contract.results.iter().enumerate() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if usize::from(result.position.depth) != depth
                || matches!(result.returns, ir::ReturnContract::NoReference)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "reference result position is duplicate, missing or has no origin mapping",
                ));
            }
        }
        Ok(())
    }
    fn charge_returns(&self, returns: &ir::ReturnContract) -> Result<()> {
        use ir::ReturnContract::*;
        match returns {
            NoReference => charged(&self.budget, self.budget.charge_work(1)),
            SharedInputPaths(paths) | MutInputPaths(paths) => {
                charged(&self.budget, self.budget.charge_work(paths.len()))?;
                for path in paths.iter() {
                    charged(
                        &self.budget,
                        self.budget
                            .charge_work(path.projection.len().saturating_add(1)),
                    )?;
                }
                Ok(())
            }
            SharedInputEnvelopes(inputs) | MutInputEnvelopes(inputs) => charged(
                &self.budget,
                self.budget.charge_work(inputs.len().saturating_add(1)),
            ),
        }
    }
    fn eq_contract(&self, ca: &ir::RefContract, cb: &ir::RefContract) -> Result<bool> {
        charged(
            &self.budget,
            self.budget.charge_work(
                ca.input_regions
                    .len()
                    .saturating_add(cb.input_regions.len())
                    .saturating_add(ca.results.len())
                    .saturating_add(cb.results.len())
                    .saturating_add(ca.outlives.len())
                    .saturating_add(cb.outlives.len())
                    .saturating_add(1),
            ),
        )?;
        for row in ca.results.iter().chain(cb.results.iter()) {
            self.charge_returns(&row.returns)?;
        }
        Ok(ca == cb)
    }
    fn same_regions(&self, a: &ir::RefContract, b: &ir::RefContract) -> Result<bool> {
        charged(
            &self.budget,
            self.budget.charge_work(
                a.input_regions
                    .len()
                    .saturating_add(b.input_regions.len())
                    .saturating_add(a.results.len())
                    .saturating_add(b.results.len())
                    .saturating_add(a.outlives.len())
                    .saturating_add(b.outlives.len())
                    .saturating_add(1),
            ),
        )?;
        Ok(a.region_count == b.region_count
            && a.input_regions == b.input_regions
            && a.results.len() == b.results.len()
            && a.results
                .iter()
                .zip(b.results.iter())
                .all(|(a, b)| a.position == b.position && a.region == b.region)
            && a.outlives == b.outlives)
    }
    fn return_refines(
        &self,
        required: &ir::RefContract,
        provided: &ir::RefContract,
    ) -> Result<bool> {
        if !self.same_regions(required, provided)? {
            return Ok(false);
        }
        for (required, provided) in required.results.iter().zip(provided.results.iter()) {
            if !self.return_origin_refines(&required.returns, &provided.returns)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn return_origin_refines(
        &self,
        required: &ir::ReturnContract,
        provided: &ir::ReturnContract,
    ) -> Result<bool> {
        use ir::ReturnContract::*;
        self.charge_returns(required)?;
        self.charge_returns(provided)?;
        let allowed = match required {
            SharedInputEnvelopes(inputs)
                if matches!(provided, SharedInputPaths(_) | SharedInputEnvelopes(_)) =>
            {
                inputs
            }
            MutInputEnvelopes(inputs)
                if matches!(provided, MutInputPaths(_) | MutInputEnvelopes(_)) =>
            {
                inputs
            }
            _ => return Ok(required == provided),
        };
        let contains = |reference: ir::InputReference| -> Result<bool> {
            for input in allowed.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if input.input == reference {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        match provided {
            SharedInputPaths(paths) | MutInputPaths(paths) => {
                for path in paths.iter() {
                    if !contains(path.input)? {
                        return Ok(false);
                    }
                }
            }
            SharedInputEnvelopes(inputs) | MutInputEnvelopes(inputs) => {
                for input in inputs.iter() {
                    if !contains(input.input)? {
                        return Ok(false);
                    }
                }
            }
            NoReference => return Ok(false),
        }
        Ok(true)
    }
    fn eq_ty(&self, left: &Ty, right: &Ty, depth: usize) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match (left, right) {
            (Ty::Param(a), Ty::Param(b)) => Ok(a == b),
            (Ty::Int(a), Ty::Int(b)) => Ok(a == b),
            (Ty::Bool, Ty::Bool) | (Ty::Unit, Ty::Unit) | (Ty::Never, Ty::Never) => Ok(true),
            (Ty::Tuple(a), Ty::Tuple(b)) => self.eq_types(a, b, depth + 1),
            (
                Ty::Array {
                    element: a,
                    len: al,
                },
                Ty::Array {
                    element: b,
                    len: bl,
                },
            ) => Ok(al == bl && self.eq_ty(a, b, depth + 1)?),
            (
                Ty::Ref {
                    kind: a,
                    referent: at,
                },
                Ty::Ref {
                    kind: b,
                    referent: bt,
                },
            ) => Ok(a == b && self.eq_ty(at, bt, depth + 1)?),
            (Ty::Struct(a), Ty::Struct(b)) => Ok(a.def().source_decl == b.def().source_decl
                && self.eq_types(&a.def().type_args, &b.def().type_args, depth + 1)?),
            (Ty::Enum(a), Ty::Enum(b)) => Ok(a.def().source_decl == b.def().source_decl
                && self.eq_types(&a.def().type_args, &b.def().type_args, depth + 1)?),
            _ => Ok(false),
        }
    }
    fn eq_types(&self, a: &[Ty], b: &[Ty], depth: usize) -> Result<bool> {
        if a.len() != b.len() {
            return Ok(false);
        }
        for (a, b) in a.iter().zip(b) {
            if !self.eq_ty(a, b, depth)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn eq_predicate(&self, a: &Predicate, b: &Predicate) -> Result<bool> {
        let (Predicate::Implements(a), Predicate::Implements(b)) = (a, b);
        Ok(a.declaration == b.declaration
            && self.eq_ty(&a.self_ty, &b.self_ty, 0)?
            && self.eq_types(&a.args, &b.args, 0)?)
    }
    fn eq_signature(&self, a: &Signature, b: &Signature) -> Result<bool> {
        if !self.eq_types(&a.params, &b.params, 0)? || !self.eq_ty(&a.result, &b.result, 0)? {
            return Ok(false);
        }
        self.eq_contract(&a.ref_contract, &b.ref_contract)
    }
    fn append_predicate(&self, output: &mut Vec<Predicate>, predicate: &Predicate) -> Result<()> {
        for old in output.iter() {
            if self.eq_predicate(old, predicate)? {
                return Ok(());
            }
        }
        if output.len() >= MAX_PREDICATES {
            return Err(FactsError::resource(
                &self.budget,
                "predicate environment limit exceeded",
            ));
        }
        push(&self.budget, output, predicate.clone())
    }
    fn validate_supertrait_graph(&self) -> Result<()> {
        fn visit(
            world: &ValidatedTraitWorld,
            declaration: DeclId,
            active: &mut Vec<DeclId>,
            complete: &mut Vec<DeclId>,
        ) -> Result<()> {
            charged(&world.budget, world.budget.check_depth(active.len()))?;
            charged(
                &world.budget,
                world.budget.charge_work(
                    active
                        .len()
                        .saturating_add(complete.len())
                        .saturating_add(1),
                ),
            )?;
            if active.contains(&declaration) {
                return Err(FactsError::cycle(
                    &world.budget,
                    "cyclic supertrait declaration metadata",
                ));
            }
            if complete.contains(&declaration) {
                return Ok(());
            }
            push(&world.budget, active, declaration)?;
            for bound in world.trait_header(declaration)?.supertraits.iter() {
                charged(&world.budget, world.budget.charge_work(1))?;
                visit(world, bound.declaration, active, complete)?;
            }
            active.pop();
            push(&world.budget, complete, declaration)
        }
        let mut active = Vec::new();
        let mut complete = Vec::new();
        for header in self.headers.traits.iter() {
            visit(self, header.declaration, &mut active, &mut complete)?;
        }
        Ok(())
    }
    fn validate_headers(&self) -> Result<()> {
        let mut seen = Vec::new();
        for id in self
            .headers
            .traits
            .iter()
            .map(|h| h.declaration)
            .chain(self.headers.items.iter().map(|h| h.declaration))
            .chain(self.headers.impls.iter().map(|h| h.declaration))
            .chain(self.headers.callables.iter().map(|h| h.declaration))
        {
            charged(&self.budget, self.budget.charge_work(seen.len()))?;
            if seen.contains(&id) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "duplicate trait-world header declaration",
                ));
            }
            push(&self.budget, &mut seen, id)?;
        }
        for h in self.headers.traits.iter() {
            let info = self.info(h.declaration)?;
            if info.kind != DeclKind::Trait
                || !self.same_env(&info.env, &h.env)?
                || !h.env.contains(h.self_param)
                || h.self_param.owner.declaration != h.declaration
                || h.env.total_arity() != 1
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "invalid authoritative trait Self binder",
                ));
            }
            let mut items = Vec::new();
            if h.supertraits.len() > MAX_PREDICATES {
                return Err(FactsError::resource(
                    &self.budget,
                    "supertrait header limit exceeded",
                ));
            }
            for goal in h.supertraits.iter() {
                self.validate_ref_in(goal, &h.env, Some(h.self_param))?;
            }
            if let Some(canonical) = self
                .catalog
                .headers()
                .traits
                .iter()
                .find(|c| c.declaration == h.declaration)
            {
                if canonical.items != h.items || canonical.supertraits.len() != h.supertraits.len()
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "canonical trait header was replaced",
                    ));
                }
                for (a, b) in canonical.supertraits.iter().zip(h.supertraits.iter()) {
                    if !self.eq_predicate(
                        &Predicate::Implements(a.clone()),
                        &Predicate::Implements(b.clone()),
                    )? {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "canonical supertraits were replaced",
                        ));
                    }
                }
            }
            for id in h.items.iter() {
                charged(&self.budget, self.budget.charge_work(items.len()))?;
                if items.contains(id) || self.item_header(*id)?.trait_decl != h.declaration {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "duplicate or foreign required trait item",
                    ));
                }
                push(&self.budget, &mut items, *id)?;
            }
        }
        for item in self.headers.items.iter() {
            let info = self.info(item.declaration)?;
            let trait_header = self.trait_header(item.trait_decl)?;
            charged(
                &self.budget,
                self.budget.charge_work(trait_header.items.len()),
            )?;
            if info.kind != DeclKind::TraitItem
                || info.owner != Some(item.trait_decl)
                || info.receiver != item.receiver
                || !trait_header.items.contains(&item.declaration)
                || !self.same_parameters(&item.schema.env, &trait_header.env)?
                || !self.same_env(&info.env, &item.schema.env)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "required trait item schema/owner mismatch",
                ));
            }
            self.validate_declaration_schema(item.declaration, &item.schema)?;
            if let Some(canonical) = self
                .catalog
                .headers()
                .items
                .iter()
                .find(|c| c.declaration == item.declaration)
                && item.default != canonical.default
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "canonical required/provided item classification changed",
                ));
            }
            if let Some(default) = item.default
                && (self.catalog.declaration(item.declaration).is_none()
                    || self.catalog.declaration(default).is_none())
            {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: source trait default body",
                ));
            }
            self.validate_schema_signature(
                item.declaration,
                &item.schema.signature,
                &item.schema.env,
                Some(trait_header.self_param),
            )?;
            self.validate_predicates(
                &item.schema.predicates,
                &item.schema.env,
                Some(trait_header.self_param),
            )?;
            if item.receiver == ReceiverMode::Value
                && !matches!(item.schema.signature.params.first(), Some(Ty::Param(p)) if *p == trait_header.self_param)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait receiver is not its owner-qualified Self",
                ));
            }
            if let ReceiverMode::Borrowed { kind } = item.receiver
                && !matches!(item.schema.signature.params.first(),
                    Some(Ty::Ref { kind: actual, referent }) if *actual == kind
                        && matches!(referent.as_ref(), Ty::Param(p) if *p == trait_header.self_param))
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "borrowed trait receiver is not a reference to its owner-qualified Self",
                ));
            }
        }
        for callable in self.headers.callables.iter() {
            let info = self.info(callable.declaration)?;
            if !matches!(info.kind, DeclKind::Function | DeclKind::AssociatedFunction)
                || !self.same_env(&info.env, &callable.schema.env)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "callable schema disagrees with registry",
                ));
            }
            self.validate_declaration_schema(callable.declaration, &callable.schema)?;
            self.validate_schema_signature(
                callable.declaration,
                &callable.schema.signature,
                &callable.schema.env,
                None,
            )?;
            if let ReceiverMode::Borrowed { kind } = info.receiver
                && !matches!(callable.schema.signature.params.first(),
                    Some(Ty::Ref { kind: actual, .. }) if *actual == kind)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "borrowed callable receiver disagrees with canonical parameter zero",
                ));
            }
            self.validate_predicates(&callable.schema.predicates, &callable.schema.env, None)?;
        }
        for (index, a) in self.headers.impls.iter().enumerate() {
            for b in self.headers.impls[..index].iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if a.trait_ref.declaration == b.trait_ref.declaration {
                    let mut bindings = Vec::new();
                    if self.unify(
                        Term {
                            side: 0,
                            ty: &a.trait_ref.self_ty,
                        },
                        Term {
                            side: 1,
                            ty: &b.trait_ref.self_ty,
                        },
                        &mut bindings,
                        0,
                    )? {
                        return Err(FactsError::ambiguous(
                            &self.budget,
                            "overlapping trait impl heads (predicate absence cannot establish disjointness)",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    fn validate_impl(&self, h: &ImplHeader) -> Result<()> {
        let info = self.info(h.declaration)?;
        if info.kind != DeclKind::TraitImpl || !self.same_env(&info.env, &h.schema_env)? {
            return Err(FactsError::invalid(
                &self.budget,
                "trait impl schema disagrees with registry",
            ));
        }
        if !matches!(h.trait_ref.self_ty, Ty::Struct(_) | Ty::Enum(_)) {
            return Err(
                if self.catalog.declaration(h.trait_ref.declaration).is_some() {
                    FactsError::invalid(
                        &self.budget,
                        "canonical foreign trait implementation requires local nominal Self",
                    )
                } else {
                    FactsError::unsupported(
                        &self.budget,
                        "unsupported: nonnominal trait implementation head",
                    )
                },
            );
        }
        if h.trait_ref.declaration == self.catalog.id(CatalogItem::Sized) {
            return Err(FactsError::invalid(
                &self.budget,
                "explicit canonical Sized implementations are invalid",
            ));
        }
        self.validate_ref_in(&h.trait_ref, &h.schema_env, None)?;
        self.validate_predicates(&h.predicates, &h.schema_env, None)?;
        for p in h.schema_env.ordered_params() {
            if !self.contains_param(&h.trait_ref.self_ty, p, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "unconstrained trait impl parameter",
                ));
            }
        }
        let trait_header = self.trait_header(h.trait_ref.declaration)?;
        for id in trait_header.items.iter() {
            let item = self.item_header(*id)?;
            if item.default.is_none() && !h.items.iter().any(|(mapped, _)| mapped == id) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait impl has missing required items",
                ));
            }
        }
        for callable in self.headers.callables.iter() {
            if self.info(callable.declaration)?.owner == Some(h.declaration) {
                charged(&self.budget, self.budget.charge_work(h.items.len()))?;
                if !h
                    .items
                    .iter()
                    .any(|(_, target)| *target == callable.declaration)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "extra unmapped trait implementation callable",
                    ));
                }
            }
        }
        let mut seen_items = Vec::new();
        let mut seen_targets = Vec::new();
        for (item_id, callable_id) in h.items.iter() {
            charged(
                &self.budget,
                self.budget
                    .charge_work(seen_items.len().saturating_add(seen_targets.len())),
            )?;
            if seen_items.contains(item_id) || seen_targets.contains(callable_id) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "duplicate trait implementation item",
                ));
            }
            push(&self.budget, &mut seen_items, *item_id)?;
            push(&self.budget, &mut seen_targets, *callable_id)?;
            let item = self.item_header(*item_id)?;
            let target = self.callable_header(*callable_id)?;
            let target_info = self.info(*callable_id)?;
            if item.trait_decl != h.trait_ref.declaration
                || target_info.owner != Some(h.declaration)
                || target_info.receiver != item.receiver
                || target_info.public
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait implementation item owner/receiver/visibility mismatch",
                ));
            }
            if !self.same_parameters(&target.schema.env, &h.schema_env)? {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: generic trait method implementation",
                ));
            }
            let env = self.predicate_env(h.declaration)?;
            let context = ProofContext {
                predicates: &env,
                phase: FactsPhase::Universal,
            };
            let (expected, _) = self.instantiate_item(item, &h.trait_ref.self_ty, context)?;
            self.validate_signature_shape(&expected.signature, &h.schema_env, None)?;
            self.validate_predicates(&expected.predicates, &h.schema_env, None)?;
            if !self.eq_types(
                &expected.signature.params,
                &target.schema.signature.params,
                0,
            )? || !self.eq_ty(
                &expected.signature.result,
                &target.schema.signature.result,
                0,
            )? || !self.return_refines(
                &expected.signature.ref_contract,
                &target.schema.signature.ref_contract,
            )? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait implementation signature does not match required declaration",
                ));
            }
            for predicate in target.schema.predicates.iter() {
                let mut allowed = false;
                for candidate in env.predicates.iter().chain(expected.predicates.iter()) {
                    if self.eq_predicate(predicate, candidate)? {
                        allowed = true;
                        break;
                    }
                }
                if !allowed {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "trait implementation strengthens required predicates",
                    ));
                }
            }
        }
        Ok(())
    }
    fn supers_for(&self, goal: &TraitRef) -> Result<Vec<Predicate>> {
        let header = self.trait_header(goal.declaration)?;
        let mut supers = Vec::new();
        for declared in header.supertraits.iter() {
            if !matches!(&declared.self_ty, Ty::Param(p) if *p == header.self_param) {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: supertrait Self type transformation",
                ));
            }
            push(
                &self.budget,
                &mut supers,
                Predicate::Implements(TraitRef {
                    declaration: declared.declaration,
                    self_ty: goal.self_ty.clone(),
                    args: declared.args.clone(),
                }),
            )?;
        }
        Ok(supers)
    }
    fn close_predicates(&self, predicates: &mut Vec<Predicate>) -> Result<()> {
        let mut index = 0;
        while index < predicates.len() {
            let Predicate::Implements(goal) = &predicates[index];
            let supers = self.supers_for(goal)?;
            for p in supers {
                self.append_predicate(predicates, &p)?;
            }
            index += 1;
        }
        Ok(())
    }
    fn implicit_sized(&self, env: &GenericEnv, predicates: &mut Vec<Predicate>) -> Result<()> {
        for p in env.ordered_params() {
            if self.info(p.owner.declaration)?.kind == DeclKind::Trait {
                continue;
            }
            self.append_predicate(
                predicates,
                &Predicate::Implements(TraitRef {
                    declaration: self.catalog.id(CatalogItem::Sized),
                    self_ty: Ty::Param(p),
                    args: Arc::from([]),
                }),
            )?;
        }
        Ok(())
    }
    fn build_declared_environments(&mut self) -> Result<()> {
        for h in self.headers.impls.iter() {
            let info = self.info(h.declaration)?;
            if info.kind != DeclKind::TraitImpl || !self.same_env(&info.env, &h.schema_env)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "impl bounds have foreign declaration authority",
                ));
            }
            self.validate_ref_in(&h.trait_ref, &h.schema_env, None)?;
            self.validate_predicates(&h.predicates, &h.schema_env, None)?;
            let mut predicates = Vec::new();
            for p in h.predicates.iter() {
                self.append_predicate(&mut predicates, p)?;
            }
            self.implicit_sized(&h.schema_env, &mut predicates)?;
            self.close_predicates(&mut predicates)?;
            retain::<PredicateEnv>(&self.budget, 1)?;
            let index = self.environments.len();
            push(
                &self.budget,
                &mut self.environments,
                Arc::new(PredicateEnv {
                    world: self.id,
                    index,
                    declaration: Some(h.declaration),
                    env: h.schema_env.clone(),
                    predicates: predicates.into(),
                }),
            )?;
        }
        Ok(())
    }
    fn carrier_contract(
        &self,
        subject: SignatureSubject<'_>,
        facts: &ValidatedFacts,
    ) -> Result<Arc<CarrierContractView>> {
        self.check_facts(facts, true)?;
        self.check_context(ProofContext::from_facts(facts))?;
        let (declaration, arguments, signature) = match subject {
            SignatureSubject::Body => match &facts.subject {
                FactsSubject::UniversalCallable(declaration) => {
                    let raw = &self.callable_header(*declaration)?.schema;
                    let mut args = Vec::new();
                    for p in raw.env.ordered_params() {
                        push(&self.budget, &mut args, Ty::Param(p))?;
                    }
                    (
                        *declaration,
                        Arc::<[Ty]>::from(args),
                        clone_schema(raw, &self.budget)?.signature,
                    )
                }
                FactsSubject::ClosedInstance(target) => (
                    target.declaration,
                    target.type_args.clone(),
                    self.discharge_direct(target, facts)?.signature,
                ),
                FactsSubject::SchemaItem(_) => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "schema item cannot publish a body carrier view",
                    ));
                }
            },
            SignatureSubject::Call(Callee::Direct(target)) => (
                target.declaration,
                target.type_args.clone(),
                self.discharge_direct(target, facts)?.signature,
            ),
            SignatureSubject::Call(Callee::TraitItem(call)) => {
                let mut args = Vec::new();
                push(&self.budget, &mut args, call.trait_ref.self_ty.clone())?;
                for arg in call.trait_ref.args.iter().chain(call.method_args.iter()) {
                    push(&self.budget, &mut args, arg.clone())?;
                }
                (
                    call.item,
                    Arc::<[Ty]>::from(args),
                    self.instantiate_trait_signature(call, facts)?,
                )
            }
        };
        let raw = self.schema.declaration_schema(declaration)?;
        if raw.env.total_arity() != arguments.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "carrier subject argument arity mismatch",
            ));
        }
        let retained = self.declaration_carriers(declaration)?;
        charged(&self.budget, self.budget.charge_work(self.origins.len()))?;
        let origin = self
            .origins
            .iter()
            .find(|(id, _)| *id == declaration)
            .map(|(_, origin)| origin.clone())
            .ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "carrier subject has no retained origin schema",
                )
            })?;
        let mut nominals = Vec::new();
        for (old, new) in raw
            .signature
            .params
            .iter()
            .zip(&signature.params)
            .chain(std::iter::once((&raw.signature.result, &signature.result)))
        {
            self.collect_nominal_mappings(old, new, &mut nominals, 0)?;
        }
        let mut components = Vec::new();
        let mut component_regions = usize::from(raw.signature.ref_contract.region_count);
        for unused in retained.source_regions.normalized.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if unused.is_none() {
                component_regions = component_regions.checked_add(1).ok_or_else(|| {
                    FactsError::resource(&self.budget, "named component region overflow")
                })?;
            }
        }
        for (index, parameter) in raw.env.ordered_params().enumerate() {
            for relative in charged(
                &self.budget,
                ir::carrier_locations(&arguments[index], &self.budget),
            )?
            .iter()
            {
                if matches!(relative.channel, ir::CarrierChannel::Fixed(_)) {
                    component_regions = component_regions.checked_add(1).ok_or_else(|| {
                        FactsError::resource(&self.budget, "hidden component region overflow")
                    })?;
                }
                if component_regions > ir::MAX_REGION_PARAMS {
                    return Err(FactsError::resource(
                        &self.budget,
                        "expanded carrier region limit exceeded",
                    ));
                }
                push(
                    &self.budget,
                    &mut components,
                    ir::CarrierComponentMapping {
                        original_parameter: parameter,
                        original: ir::CarrierLocation {
                            path: relative.path.clone(),
                            channel: ir::CarrierChannel::ParamTail {
                                parameter,
                                prefix: match relative.channel {
                                    ir::CarrierChannel::Fixed(position) => position.depth,
                                    ir::CarrierChannel::ParamTail { prefix, .. } => prefix,
                                },
                            },
                        },
                        substituted: relative.clone(),
                    },
                )?;
            }
        }
        let mut inputs = Vec::new();
        reserve_carriers(&self.budget, &mut inputs, retained.contract.inputs.len())?;
        for input in retained.contract.inputs.iter() {
            for expanded in self.expand_carrier(&input.location, &raw.env, &arguments, &nominals)? {
                if inputs.len() >= ir::MAX_LOAN_SITES {
                    return Err(FactsError::resource(
                        &self.budget,
                        "carrier input origin limit exceeded",
                    ));
                }
                push(
                    &self.budget,
                    &mut inputs,
                    ir::InputCarrier {
                        param: input.param,
                        location: expanded.location,
                    },
                )?;
            }
        }
        let mut results = Vec::new();
        for result in retained.contract.results.iter() {
            for output in self.expand_carrier(&result.location, &raw.env, &arguments, &nominals)? {
                let sources = self.expand_carrier_sources(
                    &result.sources,
                    output.component.as_ref(),
                    &raw.env,
                    &arguments,
                    &nominals,
                )?;
                push(
                    &self.budget,
                    &mut results,
                    ir::CarrierReturn {
                        location: output.location,
                        sources,
                    },
                )?;
            }
        }
        let mut writes = Vec::new();
        for write in retained.contract.writes.iter() {
            for destination in
                self.expand_carrier(&write.destination.location, &raw.env, &arguments, &nominals)?
            {
                let destination_ty =
                    signature
                        .params
                        .get(write.destination.param)
                        .ok_or_else(|| {
                            FactsError::invalid(
                                &self.budget,
                                "expanded carrier write parameter is outside canonical signature",
                            )
                        })?;
                if !self.carrier_writable(destination_ty, &destination.location)? {
                    continue;
                }
                let sources = self.expand_carrier_sources(
                    &write.sources,
                    destination.component.as_ref(),
                    &raw.env,
                    &arguments,
                    &nominals,
                )?;
                push(
                    &self.budget,
                    &mut writes,
                    ir::CarrierWriteEnvelope {
                        destination: ir::InputCarrier {
                            param: write.destination.param,
                            location: destination.location,
                        },
                        sources,
                    },
                )?;
            }
        }
        let mut constraints = Vec::new();
        for edge in retained.regions.constraints.iter() {
            let remap = |term: &ir::CarrierRegionTerm| -> Result<ir::CarrierRegionTerm> {
                Ok(match term {
                    ir::CarrierRegionTerm::Fixed(old) => {
                        ir::CarrierRegionTerm::Fixed(self.transport_fixed_region(
                            *old,
                            &raw.signature.ref_contract,
                            &signature.ref_contract,
                        )?)
                    }
                    other => other.clone(),
                })
            };
            push(
                &self.budget,
                &mut constraints,
                ir::CarrierRegionConstraint {
                    longer: remap(&edge.longer)?,
                    shorter: remap(&edge.shorter)?,
                },
            )?;
        }
        let mut normalized = Vec::new();
        for original in retained.source_regions.normalized.iter() {
            let mapped = original
                .map(|id| {
                    self.transport_fixed_region(
                        id,
                        &raw.signature.ref_contract,
                        &signature.ref_contract,
                    )
                })
                .transpose()?;
            push(&self.budget, &mut normalized, mapped)?;
        }
        retain::<ir::SourceRegionEnv>(&self.budget, 1)?;
        let source_regions = Arc::new(ir::SourceRegionEnv {
            declaration,
            region_count: retained.source_regions.region_count,
            outlives: retained.source_regions.outlives.clone(),
            normalized: normalized.into(),
        });
        retain::<CarrierContractView>(&self.budget, 1)?;
        let view = Arc::new(CarrierContractView {
            world: self.id,
            registry: self.registry,
            environment: facts.predicates.index,
            phase: facts.phase(),
            declaration,
            arguments,
            original_env: raw.env,
            original_contract: retained.contract.clone(),
            contract: ir::CarrierContract {
                inputs: inputs.into(),
                results: results.into(),
                writes: writes.into(),
            },
            origin,
            fixed: signature.ref_contract,
            regions: ir::CarrierRegionSchema {
                constraints: constraints.into(),
            },
            source_regions,
            components: components.into(),
            nominals: nominals.into(),
            budget: self.budget.clone(),
        });
        view.validate_for(facts)?;
        Ok(view)
    }
    fn transport_fixed_region(
        &self,
        old: ir::RegionParamId,
        original: &ir::RefContract,
        realized: &ir::RefContract,
    ) -> Result<ir::RegionParamId> {
        for row in original.input_regions.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if row.region == old {
                for new in realized.input_regions.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if row.input == new.input {
                        return Ok(new.region);
                    }
                }
            }
        }
        for row in original.results.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if row.region == old {
                for new in realized.results.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if row.position == new.position {
                        return Ok(new.region);
                    }
                }
            }
        }
        Err(FactsError::invalid(
            &self.budget,
            "transport lost an original fixed region",
        ))
    }
    fn expand_carrier_sources(
        &self,
        sources: &[ir::InputCarrier],
        component: Option<&ir::CarrierLocation>,
        env: &GenericEnv,
        args: &[Ty],
        nominals: &[ir::NominalCarrierMapping],
    ) -> Result<Arc<[ir::InputCarrier]>> {
        let mut output = Vec::new();
        for source in sources {
            for expanded in self.expand_carrier(&source.location, env, args, nominals)? {
                let matches = match (component, expanded.component.as_ref()) {
                    (Some(a), Some(b)) => self.same_location(a, b)?,
                    (None, None) => true,
                    _ => false,
                };
                if matches {
                    push(
                        &self.budget,
                        &mut output,
                        ir::InputCarrier {
                            param: source.param,
                            location: expanded.location,
                        },
                    )?;
                }
            }
        }
        Ok(output.into())
    }
    fn expand_carrier(
        &self,
        location: &ir::CarrierLocation,
        env: &GenericEnv,
        args: &[Ty],
        nominals: &[ir::NominalCarrierMapping],
    ) -> Result<Vec<ExpandedCarrier>> {
        let mut path = Vec::new();
        for step in location.path.0.iter() {
            let mut step = step.clone();
            if let ir::CarrierStep::VariantField { variant, .. } = &mut step {
                for mapping in nominals {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if mapping.original == variant.owner {
                        variant.owner = mapping.instantiated;
                        break;
                    }
                }
            }
            push(&self.budget, &mut path, step)?;
        }
        let mut output = Vec::new();
        match location.channel {
            ir::CarrierChannel::Fixed(position) => {
                push(
                    &self.budget,
                    &mut output,
                    ExpandedCarrier {
                        location: ir::CarrierLocation {
                            path: ir::CarrierPath(path.into()),
                            channel: ir::CarrierChannel::Fixed(position),
                        },
                        component: None,
                    },
                )?;
            }
            ir::CarrierChannel::ParamTail { parameter, prefix } => {
                let mut actual = None;
                for (index, p) in env.ordered_params().enumerate() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if p == parameter {
                        actual = args.get(index);
                        break;
                    }
                }
                let actual = actual.ok_or_else(|| {
                    FactsError::invalid(
                        &self.budget,
                        "carrier family is outside exact original substitution",
                    )
                })?;
                for relative in
                    charged(&self.budget, ir::carrier_locations(actual, &self.budget))?.iter()
                {
                    let mut full = Vec::new();
                    for step in &path {
                        push(&self.budget, &mut full, step.clone())?;
                    }
                    let channel = if relative.path.0.is_empty() {
                        match relative.channel {
                            ir::CarrierChannel::Fixed(p) => {
                                ir::CarrierChannel::Fixed(ir::ReferencePosition {
                                    depth: prefix.checked_add(p.depth).ok_or_else(|| {
                                        FactsError::resource(
                                            &self.budget,
                                            "carrier fixed prefix overflow",
                                        )
                                    })?,
                                })
                            }
                            ir::CarrierChannel::ParamTail {
                                parameter,
                                prefix: p,
                            } => ir::CarrierChannel::ParamTail {
                                parameter,
                                prefix: prefix.checked_add(p).ok_or_else(|| {
                                    FactsError::resource(
                                        &self.budget,
                                        "carrier symbolic prefix overflow",
                                    )
                                })?,
                            },
                        }
                    } else {
                        for _ in 0..prefix {
                            push(&self.budget, &mut full, ir::CarrierStep::Referent)?;
                        }
                        for step in relative.path.0.iter() {
                            push(&self.budget, &mut full, step.clone())?;
                        }
                        relative.channel.clone()
                    };
                    push(
                        &self.budget,
                        &mut output,
                        ExpandedCarrier {
                            location: ir::CarrierLocation {
                                path: ir::CarrierPath(full.into()),
                                channel,
                            },
                            component: Some(relative.clone()),
                        },
                    )?;
                }
            }
        }
        Ok(output)
    }
    fn collect_nominal_mappings(
        &self,
        original: &Ty,
        instantiated: &Ty,
        output: &mut Vec<ir::NominalCarrierMapping>,
        depth: usize,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match (original, instantiated) {
            (Ty::Param(_), _) => {}
            (
                Ty::Ref {
                    kind: a,
                    referent: a_ty,
                },
                Ty::Ref {
                    kind: b,
                    referent: b_ty,
                },
            ) if a == b => self.collect_nominal_mappings(a_ty, b_ty, output, depth + 1)?,
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                for (a, b) in a.iter().zip(b.iter()) {
                    self.collect_nominal_mappings(a, b, output, depth + 1)?;
                }
            }
            (
                Ty::Array {
                    element: a,
                    len: al,
                },
                Ty::Array {
                    element: b,
                    len: bl,
                },
            ) if al == bl => self.collect_nominal_mappings(a, b, output, depth + 1)?,
            (Ty::Struct(a), Ty::Struct(b))
                if a.def().source_decl == b.def().source_decl
                    && a.def().fields.len() == b.def().fields.len() =>
            {
                charged(&self.budget, self.budget.charge_work(output.len()))?;
                if output
                    .iter()
                    .any(|m| m.original == a.def().id && m.instantiated == b.def().id)
                {
                    return Ok(());
                }
                push(
                    &self.budget,
                    output,
                    ir::NominalCarrierMapping {
                        original: a.def().id,
                        instantiated: b.def().id,
                    },
                )?;
                for (a, b) in a.def().fields.iter().zip(b.def().fields.iter()) {
                    self.collect_nominal_mappings(&a.ty, &b.ty, output, depth + 1)?;
                }
            }
            (Ty::Enum(a), Ty::Enum(b))
                if a.def().source_decl == b.def().source_decl
                    && a.def().variants.len() == b.def().variants.len() =>
            {
                charged(&self.budget, self.budget.charge_work(output.len()))?;
                if output
                    .iter()
                    .any(|m| m.original == a.def().id && m.instantiated == b.def().id)
                {
                    return Ok(());
                }
                push(
                    &self.budget,
                    output,
                    ir::NominalCarrierMapping {
                        original: a.def().id,
                        instantiated: b.def().id,
                    },
                )?;
                for (a, b) in a.def().variants.iter().zip(b.def().variants.iter()) {
                    if a.fields.len() != b.fields.len() {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "nominal carrier variant correspondence changed",
                        ));
                    }
                    for (a, b) in a.fields.iter().zip(b.fields.iter()) {
                        self.collect_nominal_mappings(&a.ty, &b.ty, output, depth + 1)?;
                    }
                }
            }
            _ if self.eq_ty(original, instantiated, depth)? => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "nominal carrier transport changed original canonical structure",
                ));
            }
        }
        Ok(())
    }
    fn declaration_carriers(&self, declaration: DeclId) -> Result<&DeclarationCarriers> {
        for row in &self.carriers {
            charged(&self.budget, self.budget.charge_work(1))?;
            if row.declaration == declaration {
                return Ok(row);
            }
        }
        Err(FactsError::invalid(
            &self.budget,
            "declaration has no retained carrier schema",
        ))
    }
    fn validate_source_regions(
        &self,
        declaration: DeclId,
        signature: &Signature,
    ) -> Result<Arc<ir::SourceRegionEnv>> {
        let regions = self.schema.source_region_env(declaration)?;
        let count = usize::from(regions.region_count);
        if count > ir::MAX_REGION_PARAMS || regions.outlives.len() > 4096 {
            return Err(FactsError::resource(
                &self.budget,
                "source lifetime environment resource limit exceeded",
            ));
        }
        if regions.declaration != declaration
            || regions.normalized.len() != count
            || (self.catalog.declaration(declaration).is_some() && count != 0)
        {
            return Err(FactsError::invalid(
                &self.budget,
                "source lifetime environment has foreign or incomplete authority",
            ));
        }
        charged(
            &self.budget,
            self.budget.charge_work(count + regions.outlives.len()),
        )?;
        let mut closure = [0u64; ir::MAX_REGION_PARAMS];
        for (index, mapped) in regions.normalized.iter().enumerate() {
            closure[index] = 1u64 << index;
            if mapped.is_some_and(|id| id.0 >= signature.ref_contract.region_count) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source lifetime maps outside canonical signature",
                ));
            }
        }
        for (a, b) in regions.outlives.iter() {
            if a.declaration != declaration
                || b.declaration != declaration
                || usize::from(a.index) >= count
                || usize::from(b.index) >= count
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source lifetime edge escapes original declaration",
                ));
            }
            closure[usize::from(a.index)] |= 1u64 << b.index;
        }
        charged(
            &self.budget,
            self.budget.charge_work(
                count
                    .checked_mul(count)
                    .and_then(|n| n.checked_mul(count))
                    .ok_or_else(|| {
                        FactsError::resource(&self.budget, "source lifetime closure overflow")
                    })?,
            ),
        )?;
        for k in 0..count {
            for i in 0..count {
                if closure[i] & (1u64 << k) != 0 {
                    closure[i] |= closure[k];
                }
            }
        }
        for i in 0..count {
            for j in 0..count {
                if closure[i] & (1u64 << j) == 0 {
                    continue;
                }
                if let (Some(a), Some(b)) = (regions.normalized[i], regions.normalized[j]) {
                    charged(
                        &self.budget,
                        self.budget
                            .charge_work(signature.ref_contract.outlives.len() + 1),
                    )?;
                    if a != b && !signature.ref_contract.outlives.contains(&(a, b)) {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "source lifetime relation disagrees with canonical normalized signature",
                        ));
                    }
                }
            }
        }
        Ok(regions)
    }
    fn same_location(&self, a: &ir::CarrierLocation, b: &ir::CarrierLocation) -> Result<bool> {
        charged(
            &self.budget,
            self.budget.charge_work(
                a.path
                    .0
                    .len()
                    .saturating_add(b.path.0.len())
                    .saturating_add(1),
            ),
        )?;
        Ok(a == b)
    }
    fn carrier_writable(&self, mut ty: &Ty, location: &ir::CarrierLocation) -> Result<bool> {
        let mut saw_deref = false;
        let mut all_mutable = true;
        for step in location.path.0.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            ty = match (step, ty) {
                (ir::CarrierStep::Referent, Ty::Ref { kind, referent }) => {
                    saw_deref = true;
                    all_mutable &= *kind == ir::BorrowKind::Mut;
                    referent
                }
                (ir::CarrierStep::Field(index), _) => ty.field_ty(*index).ok_or_else(|| {
                    FactsError::invalid(
                        &self.budget,
                        "carrier effect field is outside original type",
                    )
                })?,
                (ir::CarrierStep::ConstantIndex(index), Ty::Array { element, len })
                    if index < len =>
                {
                    element
                }
                (ir::CarrierStep::VariantField { variant, field }, Ty::Enum(value))
                    if variant.owner == value.def().id =>
                {
                    &value
                        .def()
                        .variants
                        .get(variant.index)
                        .and_then(|v| v.fields.get(*field))
                        .ok_or_else(|| {
                            FactsError::invalid(
                                &self.budget,
                                "carrier variant effect is outside original type",
                            )
                        })?
                        .ty
                }
                _ => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "carrier effect lens is not canonical",
                    ));
                }
            };
        }
        let prefix = match location.channel {
            ir::CarrierChannel::Fixed(p) => p.depth,
            ir::CarrierChannel::ParamTail { prefix, .. } => prefix,
        };
        for _ in 0..prefix {
            charged(&self.budget, self.budget.charge_work(1))?;
            let Ty::Ref { kind, referent } = ty else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "carrier effect prefix is outside original type",
                ));
            };
            saw_deref = true;
            all_mutable &= *kind == ir::BorrowKind::Mut;
            ty = referent;
        }
        Ok(saw_deref && all_mutable)
    }
    fn derive_carriers(
        &self,
        signature: &Signature,
        source: &ir::SourceRegionEnv,
    ) -> Result<(ir::CarrierContract, ir::CarrierRegionSchema)> {
        let mut inputs = Vec::new();
        reserve_carriers(&self.budget, &mut inputs, signature.params.len())?;
        for (param, ty) in signature.params.iter().enumerate() {
            for location in charged(&self.budget, ir::carrier_locations(ty, &self.budget))?.iter() {
                if inputs.len() >= ir::MAX_LOAN_SITES {
                    return Err(FactsError::resource(
                        &self.budget,
                        "declaration carrier origin limit exceeded",
                    ));
                }
                push(
                    &self.budget,
                    &mut inputs,
                    ir::InputCarrier {
                        param,
                        location: location.clone(),
                    },
                )?;
            }
        }
        let mut results = Vec::new();
        for location in charged(
            &self.budget,
            ir::carrier_locations(&signature.result, &self.budget),
        )?
        .iter()
        {
            let mut sources = Vec::new();
            for input in &inputs {
                if self.carrier_source_eligible(
                    signature,
                    location,
                    &signature.result,
                    input,
                    None,
                )? {
                    push(&self.budget, &mut sources, input.clone())?;
                }
            }
            push(
                &self.budget,
                &mut results,
                ir::CarrierReturn {
                    location: location.clone(),
                    sources: sources.into(),
                },
            )?;
        }
        let mut writes = Vec::new();
        for destination in &inputs {
            if !self
                .carrier_writable(&signature.params[destination.param], &destination.location)?
            {
                continue;
            }
            let mut sources = Vec::new();
            for input in &inputs {
                if self.carrier_source_eligible(
                    signature,
                    &destination.location,
                    &signature.params[destination.param],
                    input,
                    Some(destination.param),
                )? {
                    push(&self.budget, &mut sources, input.clone())?;
                }
            }
            push(
                &self.budget,
                &mut writes,
                ir::CarrierWriteEnvelope {
                    destination: destination.clone(),
                    sources: sources.into(),
                },
            )?;
        }
        // ENTRY snapshots must include origins that can reach a returned carrier
        // through any admitted write. This finite closure is header-only and is
        // also the conservative recursion bootstrap.
        let mut changed = true;
        while changed {
            changed = false;
            for result in &mut results {
                let mut sources = Vec::new();
                for input in result.sources.iter() {
                    push(&self.budget, &mut sources, input.clone())?;
                }
                for write in &writes {
                    let mut reachable = false;
                    for input in &sources {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        if input.param == write.destination.param
                            && self.same_location(&input.location, &write.destination.location)?
                        {
                            reachable = true;
                            break;
                        }
                    }
                    if reachable {
                        for input in write.sources.iter() {
                            let mut exists = false;
                            for old in &sources {
                                charged(&self.budget, self.budget.charge_work(1))?;
                                if old.param == input.param
                                    && self.same_location(&old.location, &input.location)?
                                {
                                    exists = true;
                                    break;
                                }
                            }
                            if !exists {
                                push(&self.budget, &mut sources, input.clone())?;
                                changed = true;
                            }
                        }
                    }
                }
                result.sources = sources.into();
            }
        }
        let mut constraints = Vec::new();
        for &(longer, shorter) in source.outlives.iter() {
            push(
                &self.budget,
                &mut constraints,
                ir::CarrierRegionConstraint {
                    longer: ir::CarrierRegionTerm::Named(longer),
                    shorter: ir::CarrierRegionTerm::Named(shorter),
                },
            )?;
        }
        for &(longer, shorter) in signature.ref_contract.outlives.iter() {
            push(
                &self.budget,
                &mut constraints,
                ir::CarrierRegionConstraint {
                    longer: ir::CarrierRegionTerm::Fixed(longer),
                    shorter: ir::CarrierRegionTerm::Fixed(shorter),
                },
            )?;
        }
        for input in &inputs {
            if let ir::CarrierChannel::ParamTail { parameter, prefix } = input.location.channel
                && input.location.path.0.is_empty()
            {
                for row in signature.ref_contract.input_regions.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if row.input.param == input.param && row.input.position.depth < prefix {
                        push(
                            &self.budget,
                            &mut constraints,
                            ir::CarrierRegionConstraint {
                                longer: ir::CarrierRegionTerm::Family { parameter },
                                shorter: ir::CarrierRegionTerm::Fixed(row.region),
                            },
                        )?;
                    }
                }
            }
        }
        Ok((
            ir::CarrierContract {
                inputs: inputs.into(),
                results: results.into(),
                writes: writes.into(),
            },
            ir::CarrierRegionSchema {
                constraints: constraints.into(),
            },
        ))
    }
    fn carrier_source_eligible(
        &self,
        signature: &Signature,
        destination: &ir::CarrierLocation,
        destination_ty: &Ty,
        input: &ir::InputCarrier,
        write: Option<usize>,
    ) -> Result<bool> {
        let a = charged(
            &self.budget,
            ir::carrier_at(destination_ty, destination, &self.budget),
        )?;
        let b = charged(
            &self.budget,
            ir::carrier_at(
                &signature.params[input.param],
                &input.location,
                &self.budget,
            ),
        )?;
        match (&destination.channel, &input.location.channel) {
            (
                ir::CarrierChannel::ParamTail { parameter: a, .. },
                ir::CarrierChannel::ParamTail { parameter: b, .. },
            ) => Ok(a == b),
            (ir::CarrierChannel::Fixed(position), ir::CarrierChannel::Fixed(source_position)) => {
                if let Some(destination_param) = write {
                    if !self.eq_ty(a, b, 0)? {
                        return Ok(false);
                    }
                    if !destination.path.0.is_empty() || !input.location.path.0.is_empty() {
                        return Ok(false);
                    }
                    let mut destination_region = None;
                    let mut source_region = None;
                    for row in signature.ref_contract.input_regions.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        if row.input.param == destination_param && row.input.position == *position {
                            destination_region = Some(row.region);
                        }
                        if row.input.param == input.param && row.input.position == *source_position
                        {
                            source_region = Some(row.region);
                        }
                    }
                    if let (Some(longer), Some(shorter)) = (source_region, destination_region) {
                        charged(
                            &self.budget,
                            self.budget
                                .charge_work(signature.ref_contract.outlives.len() + 1),
                        )?;
                        return Ok(longer == shorter
                            || signature.ref_contract.outlives.contains(&(longer, shorter)));
                    }
                    return Ok(false);
                }
                if !destination.path.0.is_empty() || !input.location.path.0.is_empty() {
                    return Ok(false);
                }
                for row in signature.ref_contract.results.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if row.position != *position {
                        continue;
                    }
                    let expected = ir::InputReference {
                        param: input.param,
                        position: *source_position,
                    };
                    return Ok(match &row.returns {
                        ir::ReturnContract::SharedInputPaths(paths)
                        | ir::ReturnContract::MutInputPaths(paths) => {
                            charged(&self.budget, self.budget.charge_work(paths.len()))?;
                            paths.iter().any(|p| p.input == expected)
                        }
                        ir::ReturnContract::SharedInputEnvelopes(paths)
                        | ir::ReturnContract::MutInputEnvelopes(paths) => {
                            charged(&self.budget, self.budget.charge_work(paths.len()))?;
                            paths.iter().any(|p| p.input == expected)
                        }
                        ir::ReturnContract::NoReference => false,
                    });
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }
    fn build_origin_schemas(&mut self) -> Result<()> {
        let mut declarations = Vec::new();
        for declaration in self
            .headers
            .items
            .iter()
            .map(|h| h.declaration)
            .chain(self.headers.callables.iter().map(|h| h.declaration))
        {
            charged(&self.budget, self.budget.charge_work(declarations.len()))?;
            if !declarations.contains(&declaration) {
                push(&self.budget, &mut declarations, declaration)?;
            }
        }
        for declaration in declarations {
            let raw = self.schema.declaration_schema(declaration)?;
            let schema = charged(
                &self.budget,
                ir::derive_signature_origin_schema(&raw.signature, &raw.env, &self.budget),
            )?;
            let source_regions = self.validate_source_regions(declaration, &raw.signature)?;
            let (contract, regions) = self.derive_carriers(&raw.signature, &source_regions)?;
            push(&self.budget, &mut self.origins, (declaration, schema))?;
            push(
                &self.budget,
                &mut self.carriers,
                DeclarationCarriers {
                    declaration,
                    contract,
                    regions,
                    source_regions,
                },
            )?;
        }
        Ok(())
    }
    fn require_impl_wf(
        &self,
        h: &ImplHeader,
        actual: &Ty,
        context: ProofContext<'_>,
        active: &mut Vec<GoalKey>,
    ) -> Result<()> {
        let goal = TraitRef {
            declaration: h.trait_ref.declaration,
            self_ty: actual.clone(),
            args: Arc::from([]),
        };
        for p in self.supers_for(&goal)? {
            self.require_in(&p, context, active)?;
        }
        if goal.declaration != self.catalog.id(CatalogItem::Copy) {
            return Ok(());
        }
        let source_decl = match actual {
            Ty::Struct(t) => t.def().source_decl,
            Ty::Enum(t) => t.def().source_decl,
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "source Copy implementation requires local nominal Self",
                ));
            }
        };
        for other in self.headers.impls.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if other.trait_ref.declaration == self.catalog.id(CatalogItem::Drop) {
                let other_decl = match &other.trait_ref.self_ty {
                    Ty::Struct(t) => Some(t.def().source_decl),
                    Ty::Enum(t) => Some(t.def().source_decl),
                    _ => None,
                };
                if other_decl == Some(source_decl) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "canonical Copy and Drop cannot be implemented for the same type",
                    ));
                }
            }
        }
        self.require_in(
            &Predicate::Implements(TraitRef {
                declaration: self.catalog.id(CatalogItem::Sized),
                self_ty: actual.clone(),
                args: Arc::from([]),
            }),
            context,
            active,
        )?;
        let mut field = |ty: &Ty| -> Result<()> {
            self.require_in(
                &Predicate::Implements(TraitRef {
                    declaration: self.catalog.id(CatalogItem::Copy),
                    self_ty: ty.clone(),
                    args: Arc::from([]),
                }),
                context,
                active,
            )?;
            Ok(())
        };
        match actual {
            Ty::Struct(t) => {
                for f in t.def().fields.iter() {
                    field(&f.ty)?;
                }
            }
            Ty::Enum(t) => {
                for v in t.def().variants.iter() {
                    for f in v.fields.iter() {
                        field(&f.ty)?;
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn build_environments(&mut self) -> Result<()> {
        // Header schemas are now globally validated. These environments remain
        // inaccessible until the immutable world is successfully returned.
        for item in self.headers.items.iter() {
            let mut predicates = Vec::new();
            for p in item.schema.predicates.iter() {
                self.append_predicate(&mut predicates, p)?;
            }
            self.implicit_sized(&item.schema.env, &mut predicates)?;
            self.append_predicate(
                &mut predicates,
                &Predicate::Implements(TraitRef {
                    declaration: item.trait_decl,
                    self_ty: Ty::Param(self.trait_header(item.trait_decl)?.self_param),
                    args: Arc::from([]),
                }),
            )?;
            self.close_predicates(&mut predicates)?;
            retain::<PredicateEnv>(&self.budget, 1)?;
            let env = Arc::new(PredicateEnv {
                world: self.id,
                index: self.environments.len(),
                declaration: Some(item.declaration),
                env: item.schema.env.clone(),
                predicates: predicates.into(),
            });
            push(&self.budget, &mut self.environments, env)?;
        }
        for callable in self.headers.callables.iter() {
            let mut requirements = Vec::new();
            for p in callable.schema.predicates.iter() {
                self.append_predicate(&mut requirements, p)?;
            }
            self.implicit_sized(&callable.schema.env, &mut requirements)?;
            let owner = self.info(callable.declaration)?.owner;
            let mut implementation = None;
            for h in self.headers.impls.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if Some(h.declaration) == owner {
                    implementation = Some(h);
                    break;
                }
            }
            if let Some(h) = implementation {
                for p in h.predicates.iter() {
                    self.append_predicate(&mut requirements, p)?;
                }
            }
            self.close_predicates(&mut requirements)?;
            retain::<Predicate>(&self.budget, requirements.len())?;
            let declared: Arc<[Predicate]> = requirements.clone().into();
            push(
                &self.budget,
                &mut self.requirements,
                (callable.declaration, declared),
            )?;
            if let Some(h) = implementation {
                self.append_predicate(
                    &mut requirements,
                    &Predicate::Implements(h.trait_ref.clone()),
                )?;
            }
            self.close_predicates(&mut requirements)?;
            retain::<PredicateEnv>(&self.budget, 1)?;
            let env = Arc::new(PredicateEnv {
                world: self.id,
                index: self.environments.len(),
                declaration: Some(callable.declaration),
                env: callable.schema.env.clone(),
                predicates: requirements.into(),
            });
            push(&self.budget, &mut self.environments, env)?;
        }
        Ok(())
    }
    fn contains_param(&self, ty: &Ty, p: ParamId, depth: usize) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if let Ty::Param(q) = ty {
            return Ok(*q == p);
        }
        for child in type_arguments(ty) {
            if self.contains_param(child, p, depth + 1)? {
                return Ok(true);
            }
        }
        if let Ty::Array { element, .. }
        | Ty::Ref {
            referent: element, ..
        } = ty
        {
            return self.contains_param(element, p, depth + 1);
        }
        Ok(false)
    }
}

#[derive(Clone, Copy)]
struct Term<'a> {
    side: u8,
    ty: &'a Ty,
}
#[derive(Clone, Copy)]
struct Binding<'a> {
    side: u8,
    parameter: ParamId,
    value: Term<'a>,
}
impl ValidatedTraitWorld {
    fn dereference<'a>(
        &self,
        mut term: Term<'a>,
        bindings: &[Binding<'a>],
        depth: usize,
    ) -> Result<Term<'a>> {
        let mut hops = depth;
        loop {
            charged(&self.budget, self.budget.check_depth(hops))?;
            let Ty::Param(p) = term.ty else {
                return Ok(term);
            };
            let mut found = None;
            for b in bindings {
                charged(&self.budget, self.budget.charge_work(1))?;
                if b.side == term.side && b.parameter == *p {
                    found = Some(b.value);
                    break;
                }
            }
            match found {
                Some(value) => {
                    term = value;
                    hops += 1
                }
                None => return Ok(term),
            }
        }
    }
    fn occurs(
        &self,
        side: u8,
        parameter: ParamId,
        term: Term<'_>,
        bindings: &[Binding<'_>],
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let term = self.dereference(term, bindings, depth)?;
        if let Ty::Param(p) = term.ty {
            return Ok(term.side == side && *p == parameter);
        }
        for child in type_arguments(term.ty) {
            if self.occurs(
                side,
                parameter,
                Term {
                    side: term.side,
                    ty: child,
                },
                bindings,
                depth + 1,
            )? {
                return Ok(true);
            }
        }
        if let Ty::Array { element, .. }
        | Ty::Ref {
            referent: element, ..
        } = term.ty
        {
            return self.occurs(
                side,
                parameter,
                Term {
                    side: term.side,
                    ty: element,
                },
                bindings,
                depth + 1,
            );
        }
        Ok(false)
    }
    fn unify<'a>(
        &self,
        left: Term<'a>,
        right: Term<'a>,
        bindings: &mut Vec<Binding<'a>>,
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let left = self.dereference(left, bindings, depth)?;
        let right = self.dereference(right, bindings, depth)?;
        if let Ty::Param(p) = left.ty {
            if matches!(right.ty,Ty::Param(q) if left.side==right.side && p==q) {
                return Ok(true);
            }
            if self.occurs(left.side, *p, right, bindings, depth + 1)? {
                return Ok(false);
            }
            push(
                &self.budget,
                bindings,
                Binding {
                    side: left.side,
                    parameter: *p,
                    value: right,
                },
            )?;
            return Ok(true);
        }
        if matches!(right.ty, Ty::Param(_)) {
            return self.unify(right, left, bindings, depth + 1);
        }
        match (left.ty, right.ty) {
            (
                Ty::Array {
                    element: a,
                    len: al,
                },
                Ty::Array {
                    element: b,
                    len: bl,
                },
            ) => {
                return Ok(al == bl
                    && self.unify(
                        Term {
                            side: left.side,
                            ty: a,
                        },
                        Term {
                            side: right.side,
                            ty: b,
                        },
                        bindings,
                        depth + 1,
                    )?);
            }
            (
                Ty::Ref {
                    kind: a,
                    referent: at,
                },
                Ty::Ref {
                    kind: b,
                    referent: bt,
                },
            ) => {
                return Ok(a == b
                    && self.unify(
                        Term {
                            side: left.side,
                            ty: at,
                        },
                        Term {
                            side: right.side,
                            ty: bt,
                        },
                        bindings,
                        depth + 1,
                    )?);
            }
            (Ty::Tuple(_), Ty::Tuple(_)) => {}
            (Ty::Struct(a), Ty::Struct(b)) if a.def().source_decl == b.def().source_decl => {}
            (Ty::Enum(a), Ty::Enum(b)) if a.def().source_decl == b.def().source_decl => {}
            _ => return self.eq_ty(left.ty, right.ty, depth + 1),
        }
        let a = type_arguments(left.ty);
        let b = type_arguments(right.ty);
        if a.len() != b.len() {
            return Ok(false);
        }
        for (a, b) in a.iter().zip(b) {
            if !self.unify(
                Term {
                    side: left.side,
                    ty: a,
                },
                Term {
                    side: right.side,
                    ty: b,
                },
                bindings,
                depth + 1,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn check_facts(&self, facts: &ValidatedFacts, executable: bool) -> Result<()> {
        charged(&self.budget, self.budget.charge_work(1))?;
        if !std::ptr::eq(self, facts.world.as_ref())
            || facts.predicates.world != self.id
            || !self.budget.same_authority(facts.budget())
            || facts.registry_snapshot() != self.registry
        {
            return Err(FactsError::invalid(
                &self.budget,
                "facts belong to another world, registry or planning ledger",
            ));
        }
        if executable && facts.phase() == FactsPhase::Schema {
            return Err(FactsError::invalid(
                &self.budget,
                "required schema is not an executable body",
            ));
        }
        for proof in facts.requirements.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if proof.0.world != self.id
                || proof.0.environment != self.empty.index
                || proof.0.phase != FactsPhase::Closed
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closed entry evidence has foreign authority",
                ));
            }
        }
        Ok(())
    }
    pub fn schema_facts(self: &Arc<Self>, item: DeclId) -> Result<Arc<ValidatedFacts>> {
        self.item_header(item)?;
        let predicates = self.predicate_env(item)?;
        retain::<ValidatedFacts>(&self.budget, 1)?;
        Ok(Arc::new(ValidatedFacts {
            world: self.clone(),
            subject: FactsSubject::SchemaItem(item),
            predicates,
            requirements: Arc::from([]),
        }))
    }
    pub fn callable_facts(self: &Arc<Self>, decl: DeclId) -> Result<Arc<ValidatedFacts>> {
        self.ensure_provider(decl)?;
        self.callable_header(decl)?;
        let predicates = self.predicate_env(decl)?;
        retain::<ValidatedFacts>(&self.budget, 1)?;
        Ok(Arc::new(ValidatedFacts {
            world: self.clone(),
            subject: FactsSubject::UniversalCallable(decl),
            predicates,
            requirements: Arc::from([]),
        }))
    }
    pub fn instance_facts(self: &Arc<Self>, target: &CallTarget) -> Result<Arc<ValidatedFacts>> {
        // The temporary context is private and never published before all entry
        // requirements have been proved. Its assumptions are genuinely empty.
        let mut facts = ValidatedFacts {
            world: self.clone(),
            subject: FactsSubject::ClosedInstance(target.clone()),
            predicates: self.empty.clone(),
            requirements: Arc::from([]),
        };
        let proof = self.discharge_direct(target, &facts)?;
        facts.requirements = proof.evidence;
        retain::<ValidatedFacts>(&self.budget, 1)?;
        Ok(Arc::new(facts))
    }
    fn callable_schema(&self, id: DeclId) -> Result<GenericSchema> {
        let raw = &self.callable_header(id)?.schema;
        let mut predicates = None;
        for (decl, p) in &self.requirements {
            charged(&self.budget, self.budget.charge_work(1))?;
            if *decl == id {
                predicates = Some(p.clone());
                break;
            }
        }
        let predicates = predicates.ok_or_else(|| {
            FactsError::invalid(&self.budget, "callable requirements not validated")
        })?;
        retain::<Ty>(&self.budget, raw.signature.params.len())?;
        Ok(GenericSchema {
            env: raw.env.clone(),
            signature: raw.signature.clone(),
            predicates,
        })
    }
    fn instantiate_in(
        &self,
        schema: &GenericSchema,
        args: &[Ty],
        facts: ProofContext<'_>,
        declaration: Option<DeclId>,
    ) -> Result<InstantiatedSchema> {
        if args.len() != schema.env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "generic argument arity mismatch",
            ));
        }
        for arg in args {
            self.validate_executable_actual(arg, facts.env())?;
        }
        let mut result = self
            .schema
            .instantiate_schema(schema, args, facts.env(), &self.budget)?;
        if !self.eq_contract(
            &schema.signature.ref_contract,
            &result.signature.ref_contract,
        )? {
            return Err(FactsError::invalid(
                &self.budget,
                "instantiation changed declaration-derived return authority",
            ));
        }
        result.signature.ref_contract =
            self.realize_ref_contract(schema, args, &result.signature, declaration)?;
        self.validate_signature_shape(&result.signature, facts.env(), None)?;
        self.validate_predicates(&result.predicates, facts.env(), None)?;
        Ok(result)
    }
    fn validate_executable_actual(&self, actual: &Ty, env: &GenericEnv) -> Result<()> {
        charged(
            &self.budget,
            ir::validate_root_reference_actual(actual, &self.budget),
        )?;
        if !matches!(actual, Ty::Ref { .. }) {
            return self.validate_actual(actual, env);
        }
        self.validate_ty(actual, env, None, true, 0)
    }
    fn realize_ref_contract(
        &self,
        schema: &GenericSchema,
        args: &[Ty],
        realized: &Signature,
        declaration: Option<DeclId>,
    ) -> Result<Arc<ir::RefContract>> {
        let origin = charged(
            &self.budget,
            ir::derive_signature_origin_schema(&schema.signature, &schema.env, &self.budget),
        )?;
        let mut expanded = Vec::new();
        for (index, parameter) in schema.env.ordered_params().enumerate() {
            let actual = args
                .get(index)
                .ok_or_else(|| FactsError::invalid(&self.budget, "topology actual is missing"))?;
            let layers = charged(&self.budget, ir::root_reference_layers(actual))?;
            let scan_work = origin
                .input_tails
                .len()
                .checked_add(origin.result_tails.len())
                .ok_or_else(|| {
                    FactsError::resource(&self.budget, "carrier tail lookup work overflow")
                })?;
            charged(&self.budget, self.budget.charge_work(scan_work))?;
            let used = origin.input_tails.iter().any(|t| t.parameter == parameter)
                || origin.result_tails.iter().any(|t| t.parameter == parameter);
            if !used {
                continue;
            }
            for relative in 0..layers {
                if usize::from(origin.fixed.region_count) + expanded.len() >= ir::MAX_REGION_PARAMS
                {
                    return Err(FactsError::resource(
                        &self.budget,
                        "carrier topology region limit exceeded",
                    ));
                }
                push(&self.budget, &mut expanded, (parameter, relative as u8))?;
            }
        }
        if expanded.is_empty() {
            return Ok(origin.fixed.clone());
        }
        let mut inputs = Vec::new();
        let mut results = Vec::new();
        let mut edges = Vec::new();
        let mut next = usize::from(origin.fixed.region_count);
        for row in origin.fixed.input_regions.iter() {
            push(&self.budget, &mut inputs, row.clone())?;
        }
        charged(
            &self.budget,
            self.budget.charge_work(schema.signature.params.len()),
        )?;
        if inputs.is_empty()
            && schema
                .signature
                .params
                .iter()
                .any(|ty| matches!(ty, Ty::Ref { .. }))
        {
            // Erasure is declaration-owned legacy authority, not permission
            // carried by an arbitrary empty structural substitution result.
            let declaration = declaration.ok_or_else(|| {
                FactsError::invalid(&self.budget, "legacy topology requires callable authority")
            })?;
            let header = self.callable_header(declaration)?;
            if self.info(declaration)?.region_contract_mode
                != RegionContractMode::LegacyUnnamedFreeNoReturn
                || !self.same_env(&schema.env, &header.schema.env)?
                || !self.eq_signature(&schema.signature, &header.schema.signature)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "legacy topology differs from its exact callable declaration",
                ));
            }
            self.validate_schema_signature(declaration, &schema.signature, &schema.env, None)?;
            // A nonempty positioned contract must cover every original fixed
            // input, including references with no parameter-carrier tail.
            for (param, ty) in schema.signature.params.iter().enumerate() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if !matches!(ty, Ty::Ref { .. }) {
                    continue;
                }
                if next >= ir::MAX_REGION_PARAMS {
                    return Err(FactsError::resource(
                        &self.budget,
                        "carrier topology region limit exceeded",
                    ));
                }
                push(
                    &self.budget,
                    &mut inputs,
                    ir::InputRegion {
                        input: ir::InputReference {
                            param,
                            position: ir::ReferencePosition { depth: 0 },
                        },
                        region: ir::RegionParamId(next as u16),
                    },
                )?;
                next += 1;
            }
        }
        for row in origin.fixed.results.iter() {
            push(&self.budget, &mut results, row.clone())?;
        }
        for edge in origin.fixed.outlives.iter() {
            push(&self.budget, &mut edges, *edge)?;
        }
        for &(parameter, relative) in &expanded {
            if next >= ir::MAX_REGION_PARAMS {
                return Err(FactsError::resource(
                    &self.budget,
                    "carrier topology region limit exceeded",
                ));
            }
            let region = ir::RegionParamId(next as u16);
            next += 1;
            for tail in origin.input_tails.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if tail.parameter != parameter {
                    continue;
                }
                let depth = tail
                    .prefix
                    .checked_add(relative)
                    .ok_or_else(|| FactsError::resource(&self.budget, "carrier prefix overflow"))?;
                if usize::from(depth) >= self.budget.limits().depth {
                    return Err(FactsError::resource(
                        &self.budget,
                        "carrier reference depth limit exceeded",
                    ));
                }
                let input = ir::InputReference {
                    param: tail.param,
                    position: ir::ReferencePosition { depth },
                };
                // Earlier relative layers are parents too. One nearest-parent
                // edge suffices because normalization closes the hierarchy.
                for old in inputs.iter().rev() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if old.input.param == input.param && old.input.position.depth < depth {
                        push(&self.budget, &mut edges, (region, old.region))?;
                        break;
                    }
                }
                push(&self.budget, &mut inputs, ir::InputRegion { input, region })?;
            }
            for tail in origin.result_tails.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if tail.parameter != parameter {
                    continue;
                }
                let depth = tail.prefix.checked_add(relative).ok_or_else(|| {
                    FactsError::resource(&self.budget, "carrier result prefix overflow")
                })?;
                let position = ir::ReferencePosition { depth };
                let (kind, _) =
                    charged(&self.budget, ir::reference_at(&realized.result, position))?;
                let mut sources = Vec::new();
                for input in tail.inputs.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if input.parameter != parameter {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "topology origin crosses rigid parameter identity",
                        ));
                    }
                    let source = ir::InputReference {
                        param: input.param,
                        position: ir::ReferencePosition {
                            depth: input.prefix.checked_add(relative).ok_or_else(|| {
                                FactsError::resource(&self.budget, "carrier input prefix overflow")
                            })?,
                        },
                    };
                    let (source_kind, _) = charged(
                        &self.budget,
                        ir::reference_at(&realized.params[source.param], source.position),
                    )?;
                    if kind == ir::BorrowKind::Shared || source_kind == ir::BorrowKind::Mut {
                        push(
                            &self.budget,
                            &mut sources,
                            ir::InputEnvelope { input: source },
                        )?;
                    }
                }
                let returns = match kind {
                    ir::BorrowKind::Shared => {
                        ir::ReturnContract::SharedInputEnvelopes(sources.into())
                    }
                    ir::BorrowKind::Mut => ir::ReturnContract::MutInputEnvelopes(sources.into()),
                };
                push(
                    &self.budget,
                    &mut results,
                    ir::ResultReference {
                        position,
                        region,
                        returns,
                    },
                )?;
            }
        }
        let sort_work = inputs
            .len()
            .checked_mul(inputs.len())
            .and_then(|n| n.checked_add(results.len().checked_mul(results.len())?))
            .ok_or_else(|| FactsError::resource(&self.budget, "carrier ordering work overflow"))?;
        charged(&self.budget, self.budget.charge_work(sort_work))?;
        inputs.sort_by_key(|r| r.input);
        results.sort_by_key(|r| r.position);
        if inputs.len() > 4096 || results.len() > 4096 {
            return Err(FactsError::resource(
                &self.budget,
                "carrier positioned row limit exceeded",
            ));
        }
        retain::<ir::RefContract>(&self.budget, 1)?;
        let contract = ir::RefContract {
            region_count: next as u16,
            input_regions: inputs.into(),
            results: results.into(),
            outlives: edges.into(),
        };
        charged(
            &self.budget,
            ir::normalize_ref_contract(&contract, &self.budget),
        )
    }
    fn ensure_provider(&self, declaration: DeclId) -> Result<()> {
        if let Some(record) = self.catalog.declaration(declaration) {
            match record.callable_provider {
                Some(CallableProvider::Recipe { .. }) => {}
                Some(CallableProvider::UnavailableProvided { .. }) => {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: canonical provided body is unavailable",
                    ));
                }
                _ => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "signature-only catalog item is not an executable target",
                    ));
                }
            }
        }
        Ok(())
    }
    fn check_context(&self, context: ProofContext<'_>) -> Result<()> {
        if context.predicates.world != self.id {
            return Err(FactsError::invalid(
                &self.budget,
                "topology context belongs to another world",
            ));
        }
        let expected = if context.predicates.index == usize::MAX {
            &self.empty
        } else {
            self.environments
                .get(context.predicates.index)
                .ok_or_else(|| {
                    FactsError::invalid(
                        &self.budget,
                        "topology predicate environment index is foreign",
                    )
                })?
        };
        if !std::ptr::eq(expected.as_ref(), context.predicates) {
            return Err(FactsError::invalid(
                &self.budget,
                "topology context is not the world's retained predicate authority",
            ));
        }
        if context.predicates.index == usize::MAX
            && (context.env().is_generic() || !context.predicates.predicates.is_empty())
        {
            return Err(FactsError::invalid(
                &self.budget,
                "nonempty topology context used the empty sentinel",
            ));
        }
        Ok(())
    }
    fn instantiate_item(
        &self,
        item: &TraitItemHeader,
        actual: &Ty,
        context: ProofContext<'_>,
    ) -> Result<(InstantiatedSchema, Option<Arc<SignatureTopologyProof>>)> {
        self.check_context(context)?;
        let owner = self.trait_header(item.trait_decl)?;
        let info = self.info(item.declaration)?;
        if info.kind != DeclKind::TraitItem
            || info.owner != Some(owner.declaration)
            || item.schema.env.ordered_params().next() != Some(owner.self_param)
            || owner.self_param.index != 0
        {
            return Err(FactsError::invalid(
                &self.budget,
                "schema substitution lost authoritative Trait Self membership",
            ));
        }
        self.validate_ty(actual, context.env(), None, true, 0)?;
        charged(&self.budget, self.budget.charge_work(self.origins.len()))?;
        let origin = self
            .origins
            .iter()
            .find(|(d, _)| *d == item.declaration)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "trait item has no original signature-origin schema",
                )
            })?;
        let actual_layers = charged(&self.budget, ir::root_reference_layers(actual))?;
        let expands = actual_layers > 0
            && (!origin.input_tails.is_empty() || !origin.result_tails.is_empty());
        let fixed = &origin.fixed;
        let total = usize::from(fixed.region_count)
            .checked_add(if expands { actual_layers } else { 0 })
            .ok_or_else(|| FactsError::resource(&self.budget, "topology region overflow"))?;
        if total > 64 {
            return Err(FactsError::resource(
                &self.budget,
                "topology positioned region limit exceeded",
            ));
        }
        let introduced = origin
            .input_tails
            .len()
            .checked_add(origin.result_tails.len())
            .and_then(|n| n.checked_mul(actual_layers))
            .and_then(|n| n.checked_add(fixed.input_regions.len()))
            .and_then(|n| n.checked_add(fixed.results.len()))
            .ok_or_else(|| FactsError::resource(&self.budget, "topology position overflow"))?;
        if introduced > 4096 {
            return Err(FactsError::resource(
                &self.budget,
                "topology reference position limit exceeded",
            ));
        }
        // Schema topology is common to Source and Recipe execution. Logical
        // membership never reaches this allocation/admission path.
        for (prefix, parameter) in origin
            .input_tails
            .iter()
            .map(|t| (t.prefix, t.parameter))
            .chain(origin.result_tails.iter().map(|t| (t.prefix, t.parameter)))
        {
            charged(&self.budget, self.budget.charge_work(1))?;
            if parameter != owner.self_param {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: executable method type parameters",
                ));
            }
            charged(
                &self.budget,
                self.budget.check_depth(
                    usize::from(prefix)
                        .checked_add(actual_layers)
                        .ok_or_else(|| {
                            FactsError::resource(&self.budget, "schema reference depth overflow")
                        })?,
                ),
            )?;
        }
        let mut instantiated = self.schema.instantiate_schema(
            &item.schema,
            std::slice::from_ref(actual),
            context.env(),
            &self.budget,
        )?;
        if !self.eq_contract(
            &item.schema.signature.ref_contract,
            &instantiated.signature.ref_contract,
        )? {
            return Err(FactsError::invalid(
                &self.budget,
                "raw schema adapter changed return authority before topology validation",
            ));
        }
        if !expands {
            self.validate_signature_shape(&instantiated.signature, context.env(), None)?;
            return Ok((instantiated, None));
        }
        let mut inputs = Vec::new();
        let mut results = Vec::new();
        let mut edges = Vec::new();
        let mut carriers = Vec::new();
        for row in fixed.input_regions.iter() {
            push(&self.budget, &mut inputs, row.clone())?;
        }
        for row in fixed.results.iter() {
            push(&self.budget, &mut results, row.clone())?;
        }
        for edge in fixed.outlives.iter() {
            push(&self.budget, &mut edges, *edge)?;
        }
        for relative in 0..actual_layers {
            let position = ir::ReferencePosition {
                depth: relative as u8,
            };
            let region = ir::RegionParamId(fixed.region_count + relative as u16);
            let mut ci = Vec::new();
            let mut cr = Vec::new();
            for tail in origin.input_tails.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if tail.parameter != owner.self_param {
                    continue;
                }
                let input = ir::InputReference {
                    param: tail.param,
                    position: ir::ReferencePosition {
                        depth: tail.prefix + relative as u8,
                    },
                };
                push(&self.budget, &mut inputs, ir::InputRegion { input, region })?;
                push(&self.budget, &mut ci, input)?;
            }
            for tail in origin.result_tails.iter() {
                if tail.parameter != owner.self_param {
                    continue;
                }
                let output = ir::ReferencePosition {
                    depth: tail.prefix + relative as u8,
                };
                let (kind, _) = charged(
                    &self.budget,
                    ir::reference_at(&instantiated.signature.result, output),
                )?;
                let mut envelopes = Vec::new();
                for input in tail.inputs.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if input.parameter != tail.parameter {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "latent result widened to a different parameter",
                        ));
                    }
                    let anchor = ir::InputReference {
                        param: input.param,
                        position: ir::ReferencePosition {
                            depth: input.prefix + relative as u8,
                        },
                    };
                    let ty = instantiated
                        .signature
                        .params
                        .get(anchor.param)
                        .ok_or_else(|| {
                            FactsError::invalid(
                                &self.budget,
                                "latent input index is outside signature",
                            )
                        })?;
                    let (input_kind, _) =
                        charged(&self.budget, ir::reference_at(ty, anchor.position))?;
                    if kind == ir::BorrowKind::Shared || input_kind == ir::BorrowKind::Mut {
                        push(
                            &self.budget,
                            &mut envelopes,
                            ir::InputEnvelope { input: anchor },
                        )?;
                    }
                }
                let returns = if kind == ir::BorrowKind::Shared {
                    ir::ReturnContract::SharedInputEnvelopes(envelopes.into())
                } else {
                    ir::ReturnContract::MutInputEnvelopes(envelopes.into())
                };
                push(
                    &self.budget,
                    &mut results,
                    ir::ResultReference {
                        position: output,
                        region,
                        returns,
                    },
                )?;
                push(&self.budget, &mut cr, output)?;
            }
            push(
                &self.budget,
                &mut carriers,
                CarrierRegionBinding {
                    carrier: ir::ParameterCarrier {
                        parameter: owner.self_param,
                        relative: position,
                    },
                    region,
                    inputs: ci.into(),
                    results: cr.into(),
                },
            )?;
        }
        charged(
            &self.budget,
            self.budget.charge_work(
                inputs
                    .len()
                    .saturating_mul(inputs.len())
                    .saturating_add(results.len().saturating_mul(results.len())),
            ),
        )?;
        inputs.sort_unstable_by_key(|r| (r.input.param, r.input.position.depth));
        results.sort_unstable_by_key(|r| r.position.depth);
        for rows in inputs.windows(2) {
            if rows[0].input.param == rows[1].input.param {
                push(&self.budget, &mut edges, (rows[1].region, rows[0].region))?;
            }
        }
        for rows in results.windows(2) {
            push(&self.budget, &mut edges, (rows[1].region, rows[0].region))?;
        }
        retain::<ir::RefContract>(&self.budget, 1)?;
        let raw = ir::RefContract {
            region_count: total as u16,
            input_regions: inputs.into(),
            results: results.into(),
            outlives: edges.into(),
        };
        let realized = charged(&self.budget, ir::normalize_ref_contract(&raw, &self.budget))?;
        let mut renumber = Vec::new();
        charged(
            &self.budget,
            self.budget.charge_work(
                total
                    .saturating_mul(raw.input_regions.len().saturating_add(raw.results.len()))
                    .saturating_mul(2),
            ),
        )?;
        for old in 0..total {
            let region = ir::RegionParamId(old as u16);
            let mapped = raw
                .input_regions
                .iter()
                .find(|r| r.region == region)
                .and_then(|r| {
                    realized
                        .input_regions
                        .iter()
                        .find(|n| n.input == r.input)
                        .map(|r| r.region)
                })
                .or_else(|| {
                    raw.results
                        .iter()
                        .find(|r| r.region == region)
                        .and_then(|r| {
                            realized
                                .results
                                .iter()
                                .find(|n| n.position == r.position)
                                .map(|r| r.region)
                        })
                })
                .ok_or_else(|| {
                    FactsError::invalid(&self.budget, "topology region correspondence was lost")
                })?;
            push(&self.budget, &mut renumber, mapped)?;
        }
        for carrier in &mut carriers {
            carrier.region = renumber[usize::from(carrier.region.0)];
        }
        let mut fixed_regions = Vec::new();
        for region in renumber.iter().take(usize::from(fixed.region_count)) {
            push(&self.budget, &mut fixed_regions, *region)?;
        }
        instantiated.signature.ref_contract = realized.clone();
        self.validate_signature_shape(&instantiated.signature, context.env(), None)?;
        self.validate_position_coverage(&instantiated.signature)?;
        retain::<Ty>(&self.budget, 1)?;
        retain::<SignatureTopologyProof>(&self.budget, 1)?;
        let proof = Arc::new(SignatureTopologyProof {
            world: self.id,
            registry: self.registry,
            environment: context.predicates.index,
            phase: context.phase,
            declaration: item.declaration,
            arguments: Arc::from([actual.clone()]),
            schema: origin,
            fixed_regions: fixed_regions.into(),
            carriers: carriers.into(),
            realized,
            budget: self.budget.clone(),
        });
        self.validate_topology(&proof, context)?;
        Ok((instantiated, Some(proof)))
    }
    fn validate_topology(
        &self,
        proof: &SignatureTopologyProof,
        context: ProofContext<'_>,
    ) -> Result<()> {
        self.check_context(context)?;
        charged(&self.budget, self.budget.charge_work(self.origins.len()))?;
        if proof.world != self.id
            || proof.registry != self.registry
            || proof.environment != context.predicates.index
            || proof.phase != context.phase
            || !proof.budget.same_authority(&self.budget)
            || proof.arguments.len() != 1
            || proof.fixed_regions.len() != usize::from(proof.schema.fixed.region_count)
            || !self
                .origins
                .iter()
                .any(|(d, s)| *d == proof.declaration && Arc::ptr_eq(s, &proof.schema))
        {
            return Err(FactsError::invalid(
                &self.budget,
                "signature topology certificate lost exact context authority",
            ));
        }
        let owner = self.trait_header(self.item_header(proof.declaration)?.trait_decl)?;
        for binding in proof.carriers.iter() {
            let scans = binding
                .inputs
                .len()
                .checked_mul(proof.realized.input_regions.len())
                .and_then(|n| {
                    binding
                        .results
                        .len()
                        .checked_mul(proof.realized.results.len())
                        .and_then(|m| n.checked_add(m))
                })
                .ok_or_else(|| {
                    FactsError::resource(&self.budget, "topology verification work overflow")
                })?;
            charged(&self.budget, self.budget.charge_work(scans))?;
            charged(
                &self.budget,
                self.budget.charge_work(
                    binding
                        .inputs
                        .len()
                        .saturating_add(binding.results.len())
                        .saturating_add(1),
                ),
            )?;
            if binding.carrier.parameter != owner.self_param
                || binding.region.0 >= proof.realized.region_count
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "topology carrier binding is foreign",
                ));
            }
            for input in binding.inputs.iter() {
                if !proof
                    .realized
                    .input_regions
                    .iter()
                    .any(|r| r.input == *input && r.region == binding.region)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "topology input binding mismatch",
                    ));
                }
            }
            for result in binding.results.iter() {
                if !proof
                    .realized
                    .results
                    .iter()
                    .any(|r| r.position == *result && r.region == binding.region)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "topology result binding mismatch",
                    ));
                }
            }
        }
        Ok(())
    }
    fn require_in(
        &self,
        predicate: &Predicate,
        facts: ProofContext<'_>,
        active: &mut Vec<GoalKey>,
    ) -> Result<ProofEvidence> {
        match self.prove_in(predicate, facts, active)? {
            ProofOutcome::Proved(p) => Ok(p),
            ProofOutcome::NotProven => Err(FactsError::unproved(
                &self.budget,
                "trait obligation is not proven by caller bounds or a conditional implementation",
            )),
        }
    }
    fn same_key(&self, a: &GoalKey, b: &GoalKey) -> Result<bool> {
        Ok(a.environment == b.environment
            && a.phase == b.phase
            && self.eq_predicate(&a.goal, &b.goal)?)
    }
    fn prove(
        &self,
        predicate: &Predicate,
        facts: &ValidatedFacts,
        active: &mut Vec<GoalKey>,
    ) -> Result<ProofOutcome> {
        self.prove_in(predicate, ProofContext::from_facts(facts), active)
    }
    fn prove_in(
        &self,
        predicate: &Predicate,
        facts: ProofContext<'_>,
        active: &mut Vec<GoalKey>,
    ) -> Result<ProofOutcome> {
        charged(&self.budget, self.budget.check_depth(active.len()))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let Predicate::Implements(goal) = predicate;
        self.validate_ref_in(goal, facts.env(), None)?;
        let key = GoalKey {
            environment: facts.predicates.index,
            phase: facts.phase,
            goal: predicate.clone(),
        };
        for old in active.iter() {
            if self.same_key(old, &key)? {
                return Err(FactsError::cycle(
                    &self.budget,
                    "ordinary trait obligation cycle is not self-justifying evidence",
                ));
            }
        }
        let index = {
            let mut memo = self.memo.lock().map_err(|_| {
                FactsError::resource(&self.budget, "trait obligation cache poisoned")
            })?;
            let mut index = None;
            for (i, entry) in memo.iter().enumerate() {
                if self.same_key(&entry.key, &key)? {
                    match &entry.state {
                        MemoState::Complete(outcome) => return Ok(outcome.clone()),
                        MemoState::Errored(e) => return Err(e.clone()),
                        MemoState::Active => {
                            index = Some(i);
                            break;
                        }
                    }
                }
            }
            if let Some(index) = index {
                index
            } else {
                charged(&self.budget, self.budget.charge_obligation())?;
                let index = memo.len();
                push(
                    &self.budget,
                    &mut memo,
                    MemoEntry {
                        key: key.clone(),
                        state: MemoState::Active,
                    },
                )?;
                index
            }
        };
        if let Err(error) = push(&self.budget, active, key.clone()) {
            let mut memo = self.memo.lock().map_err(|_| {
                FactsError::resource(&self.budget, "trait obligation cache poisoned")
            })?;
            memo[index].state = MemoState::Errored(error.clone());
            return Err(error);
        }
        let result = self.prove_uncached(predicate, facts, active);
        active.pop();
        // No cache lock is held over recursive execution. Concurrent requests
        // can compute the same active key independently, never treating another
        // worker's active entry as a cycle or receiving a fresh goal allowance.
        let mut memo = self
            .memo
            .lock()
            .map_err(|_| FactsError::resource(&self.budget, "trait obligation cache poisoned"))?;
        memo[index].state = match &result {
            Ok(outcome) => MemoState::Complete(outcome.clone()),
            Err(e) => MemoState::Errored(e.clone()),
        };
        result
    }
    fn evidence(
        &self,
        predicate: &Predicate,
        facts: ProofContext<'_>,
        origin: ProofOrigin,
    ) -> Result<ProofOutcome> {
        retain::<ProofRecord>(&self.budget, 1)?;
        Ok(ProofOutcome::Proved(ProofEvidence(Arc::new(ProofRecord {
            world: self.id,
            environment: facts.predicates.index,
            phase: facts.phase,
            goal: predicate.clone(),
            origin,
        }))))
    }
    fn prove_uncached(
        &self,
        predicate: &Predicate,
        facts: ProofContext<'_>,
        active: &mut Vec<GoalKey>,
    ) -> Result<ProofOutcome> {
        for assumption in facts.predicates.predicates.iter() {
            if self.eq_predicate(assumption, predicate)? {
                return self.evidence(predicate, facts, ProofOrigin::Assumption);
            }
        }
        let Predicate::Implements(goal) = predicate;
        if let Some(outcome) = self.prove_builtin(predicate, facts, active)? {
            return Ok(outcome);
        }
        let mut selected = None;
        for implementation in self.headers.impls.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if implementation.trait_ref.declaration != goal.declaration {
                continue;
            }
            let mut slots = Vec::new();
            for _ in implementation.schema_env.ordered_params() {
                push(&self.budget, &mut slots, None)?;
            }
            if !self.match_head(
                &implementation.trait_ref.self_ty,
                &goal.self_ty,
                &implementation.schema_env,
                &mut slots,
                0,
            )? {
                continue;
            }
            let mut args = Vec::new();
            for slot in slots {
                push(
                    &self.budget,
                    &mut args,
                    slot.ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "unconstrained impl parameter reached candidate selection",
                        )
                    })?,
                )?;
            }
            // Use the raw structural substitution callback, never invoke body
            // checking or a second compiler to justify an implementation.
            retain::<ir::RefContract>(&self.budget, 1)?;
            let schema = GenericSchema {
                env: implementation.schema_env.clone(),
                signature: Signature {
                    ref_contract: Arc::new(ir::RefContract::default()),
                    params: Vec::new(),
                    result: implementation.trait_ref.self_ty.clone(),
                },
                predicates: implementation.predicates.clone(),
            };
            let instantiated = self.instantiate_in(&schema, &args, facts, None)?;
            if !self.eq_ty(&instantiated.signature.result, &goal.self_ty, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "impl head substitution disagrees with matched goal",
                ));
            }
            let mut satisfied = true;
            for required in instantiated.predicates.iter() {
                match self.prove_in(required, facts, active)? {
                    ProofOutcome::Proved(_) => {}
                    ProofOutcome::NotProven => {
                        satisfied = false;
                        break;
                    }
                }
            }
            if !satisfied {
                continue;
            }
            self.require_impl_wf(implementation, &goal.self_ty, facts, active)?;
            if selected.is_some() {
                return Err(FactsError::ambiguous(
                    &self.budget,
                    "multiple applicable trait implementations",
                ));
            }
            selected = Some((implementation.declaration, args));
        }
        if let Some((declaration, args)) = selected {
            self.evidence(
                predicate,
                facts,
                ProofOrigin::Implementation {
                    declaration,
                    arguments: args.into(),
                },
            )
        } else {
            Ok(ProofOutcome::NotProven)
        }
    }
    fn prove_builtin(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
        active: &mut Vec<GoalKey>,
    ) -> Result<Option<ProofOutcome>> {
        let Predicate::Implements(goal) = predicate;
        let item = if goal.declaration == self.catalog.id(CatalogItem::Sized) {
            CatalogItem::Sized
        } else if goal.declaration == self.catalog.id(CatalogItem::Clone) {
            CatalogItem::Clone
        } else if goal.declaration == self.catalog.id(CatalogItem::Copy) {
            CatalogItem::Copy
        } else {
            return Ok(None);
        };
        let mut children: Vec<&Ty> = Vec::new();
        let known = match &goal.self_ty {
            Ty::Param(_) => return Ok(Some(ProofOutcome::NotProven)),
            Ty::Struct(t) if item == CatalogItem::Sized => {
                for f in t.def().fields.iter() {
                    push(&self.budget, &mut children, &f.ty)?;
                }
                true
            }
            Ty::Enum(t) if item == CatalogItem::Sized => {
                for v in t.def().variants.iter() {
                    for f in v.fields.iter() {
                        push(&self.budget, &mut children, &f.ty)?;
                    }
                }
                true
            }
            Ty::Struct(_) | Ty::Enum(_) => return Ok(None),
            Ty::Tuple(parts) => {
                for t in parts.iter() {
                    push(&self.budget, &mut children, t)?;
                }
                true
            }
            Ty::Array { element, .. } => {
                push(&self.budget, &mut children, element.as_ref())?;
                true
            }
            Ty::Ref { kind, .. } => item == CatalogItem::Sized || *kind == ir::BorrowKind::Shared,
            Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Never => true,
        };
        if !known {
            return Ok(Some(ProofOutcome::NotProven));
        }
        for ty in children {
            let child = Predicate::Implements(TraitRef {
                declaration: goal.declaration,
                self_ty: ty.clone(),
                args: Arc::from([]),
            });
            if matches!(
                self.prove_in(&child, context, active)?,
                ProofOutcome::NotProven
            ) {
                return Ok(Some(ProofOutcome::NotProven));
            }
        }
        for supertrait in self.supers_for(goal)? {
            if matches!(
                self.prove_in(&supertrait, context, active)?,
                ProofOutcome::NotProven
            ) {
                return Ok(Some(ProofOutcome::NotProven));
            }
        }
        self.evidence(predicate, context, ProofOrigin::Builtin { item })
            .map(Some)
    }
    fn trait_signature_with_topology(
        &self,
        call: &TraitCall,
        context: ProofContext<'_>,
    ) -> Result<(Signature, Option<Arc<SignatureTopologyProof>>)> {
        self.validate_ref_in(&call.trait_ref, context.env(), None)?;
        let item = self.item_header(call.item)?;
        if item.trait_decl != call.trait_ref.declaration {
            return Err(FactsError::invalid(
                &self.budget,
                "trait call item belongs to another trait",
            ));
        }
        if !call.method_args.is_empty() {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: generic trait method arguments",
            ));
        }
        self.require_in(
            &Predicate::Implements(call.trait_ref.clone()),
            context,
            &mut Vec::new(),
        )?;
        let (instantiated, topology) =
            self.instantiate_item(item, &call.trait_ref.self_ty, context)?;
        for predicate in instantiated.predicates.iter() {
            self.require_in(predicate, context, &mut Vec::new())?;
        }
        Ok((instantiated.signature, topology))
    }
    fn trait_signature_in(&self, call: &TraitCall, context: ProofContext<'_>) -> Result<Signature> {
        self.trait_signature_with_topology(call, context)
            .map(|(s, _)| s)
    }
    fn discharge_in(
        &self,
        target: &CallTarget,
        context: ProofContext<'_>,
    ) -> Result<DirectCallProof> {
        self.ensure_provider(target.declaration)?;
        let schema = self.callable_schema(target.declaration)?;
        let instantiated = self.instantiate_in(
            &schema,
            &target.type_args,
            context,
            Some(target.declaration),
        )?;
        let mut evidence = Vec::new();
        for predicate in instantiated.predicates.iter() {
            push(
                &self.budget,
                &mut evidence,
                self.require_in(predicate, context, &mut Vec::new())?,
            )?;
        }
        Ok(DirectCallProof {
            target: target.clone(),
            signature: instantiated.signature,
            evidence: evidence.into(),
        })
    }
    fn goal_is_closed(&self, goal: &TraitRef, env: &GenericEnv) -> Result<bool> {
        let mut walk = TypeWalk::default();
        self.validate_ty_inner(&goal.self_ty, env, 0, TypeUse::Root, &mut walk)?;
        for ty in goal.args.iter() {
            self.validate_ty_inner(ty, env, 0, TypeUse::Actual, &mut walk)?;
        }
        Ok(!walk.has_parameters)
    }
    fn match_head(
        &self,
        pattern: &Ty,
        actual: &Ty,
        env: &GenericEnv,
        slots: &mut [Option<Ty>],
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if let Ty::Param(p) = pattern {
            let mut index = None;
            for (i, parameter) in env.ordered_params().enumerate() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if parameter == *p {
                    index = Some(i);
                    break;
                }
            }
            let index = index.ok_or_else(|| {
                FactsError::invalid(&self.budget, "impl pattern parameter outside its binder")
            })?;
            return match &slots[index] {
                Some(old) => self.eq_ty(old, actual, depth + 1),
                None => {
                    slots[index] = Some(actual.clone());
                    Ok(true)
                }
            };
        }
        match (pattern, actual) {
            (
                Ty::Ref {
                    kind: a,
                    referent: at,
                },
                Ty::Ref {
                    kind: b,
                    referent: bt,
                },
            ) => {
                return Ok(a == b && self.match_head(at, bt, env, slots, depth + 1)?);
            }
            (Ty::Tuple(_), Ty::Tuple(_)) => {}
            (Ty::Struct(a), Ty::Struct(b)) if a.def().source_decl == b.def().source_decl => {}
            (Ty::Enum(a), Ty::Enum(b)) if a.def().source_decl == b.def().source_decl => {}
            (
                Ty::Array {
                    element: a,
                    len: al,
                },
                Ty::Array {
                    element: b,
                    len: bl,
                },
            ) => return Ok(al == bl && self.match_head(a, b, env, slots, depth + 1)?),
            _ => return self.eq_ty(pattern, actual, depth + 1),
        }
        let a = type_arguments(pattern);
        let b = type_arguments(actual);
        if a.len() != b.len() {
            return Ok(false);
        }
        for (a, b) in a.iter().zip(b) {
            if !self.match_head(a, b, env, slots, depth + 1)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl TraitCx for ValidatedTraitWorld {
    fn validate_signature(
        &self,
        subject: SignatureSubject<'_>,
        signature: &Signature,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        self.check_facts(facts, true)?;
        let expected = match subject {
            SignatureSubject::Body => match &facts.subject {
                FactsSubject::UniversalCallable(declaration) => {
                    let signature = &self.callable_header(*declaration)?.schema.signature;
                    retain::<Ty>(&self.budget, signature.params.len())?;
                    signature.clone()
                }
                FactsSubject::ClosedInstance(target) => {
                    self.discharge_direct(target, facts)?.signature
                }
                FactsSubject::SchemaItem(_) => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "schema facts cannot validate an executable body",
                    ));
                }
            },
            SignatureSubject::Call(Callee::Direct(target)) => {
                self.discharge_direct(target, facts)?.signature
            }
            SignatureSubject::Call(Callee::TraitItem(call)) => {
                self.instantiate_trait_signature(call, facts)?
            }
        };
        if !self.eq_signature(&expected, signature)? {
            return Err(FactsError::invalid(
                &self.budget,
                "signature differs from its exact canonical world subject",
            ));
        }
        self.validate_signature_shape(signature, facts.env(), None)
    }
    fn callee_receiver(&self, callee: &Callee, facts: &ValidatedFacts) -> Result<ReceiverMode> {
        self.check_facts(facts, true)?;
        match callee {
            Callee::Direct(target) => {
                self.discharge_direct(target, facts)?;
                Ok(self.info(target.declaration)?.receiver)
            }
            Callee::TraitItem(call) => {
                self.instantiate_trait_signature(call, facts)?;
                Ok(self.item_header(call.item)?.receiver)
            }
        }
    }
    fn validate_trait_ref(&self, goal: &TraitRef, facts: &ValidatedFacts) -> Result<()> {
        self.check_facts(facts, true)?;
        self.validate_ref_in(goal, facts.env(), None)
    }
    fn instantiate_trait_signature(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<Signature> {
        self.check_facts(facts, true)?;
        self.trait_signature_in(call, ProofContext::from_facts(facts))
    }
    fn prove_goal(&self, goal: &Predicate, facts: &ValidatedFacts) -> Result<ProofOutcome> {
        self.check_facts(facts, true)?;
        self.prove(goal, facts, &mut Vec::new())
    }
    fn discharge_direct(
        &self,
        target: &CallTarget,
        facts: &ValidatedFacts,
    ) -> Result<DirectCallProof> {
        self.check_facts(facts, true)?;
        self.discharge_in(target, ProofContext::from_facts(facts))
    }
    fn resolve_trait_call(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<ResolvedTraitCall> {
        self.check_facts(facts, true)?;
        let caller = ProofContext::from_facts(facts);
        let (signature, caller_topology) = self.trait_signature_with_topology(call, caller)?;
        let predicate = Predicate::Implements(call.trait_ref.clone());
        if facts.phase() == FactsPhase::Universal
            && !self.goal_is_closed(&call.trait_ref, facts.env())?
        {
            let evidence = self.require_in(&predicate, caller, &mut Vec::new())?;
            return Ok(ResolvedTraitCall {
                callee: Callee::TraitItem(call.clone()),
                signature,
                _evidence: evidence,
                _refinement: None,
                _topology: caller_topology,
            });
        }
        // The same obligation engine runs with the world's private empty
        // environment, even when a caller body has unrelated generic bounds.
        let closed = ProofContext {
            predicates: &self.empty,
            phase: FactsPhase::Closed,
        };
        let (signature, topology) = self.trait_signature_with_topology(call, closed)?;
        let evidence = self.require_in(&predicate, closed, &mut Vec::new())?;
        if evidence.0.world != self.id
            || evidence.0.environment != self.empty.index
            || evidence.0.phase != FactsPhase::Closed
            || !self.eq_predicate(&evidence.0.goal, &predicate)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "dispatch evidence does not prove the exact goal",
            ));
        }
        let mut target = None;
        if let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &evidence.0.origin
        {
            for h in self.headers.impls.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if h.declaration == *declaration {
                    for (item, body) in h.items.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        if *item == call.item {
                            target = Some(CallTarget {
                                declaration: *body,
                                type_args: arguments.clone(),
                            });
                            break;
                        }
                    }
                    if target.is_none()
                        && let Some(default) = self.item_header(call.item)?.default
                    {
                        self.ensure_provider(default)?;
                        target = Some(CallTarget {
                            declaration: default,
                            type_args: Arc::from([call.trait_ref.self_ty.clone()]),
                        });
                    }
                    break;
                }
            }
        } else if let ProofOrigin::Builtin {
            item: CatalogItem::Clone,
        } = &evidence.0.origin
        {
            if call.item != self.catalog.id(CatalogItem::CloneItem) {
                if let Some(default) = self.item_header(call.item)?.default {
                    self.ensure_provider(default)?;
                }
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: builtin trait item has no executable provider",
                ));
            }
            let mut args = Vec::new();
            let descriptor = match &call.trait_ref.self_ty {
                Ty::Int(i) => RecipeDescriptor::CloneInt(*i),
                Ty::Bool => RecipeDescriptor::CloneBool,
                Ty::Unit => RecipeDescriptor::CloneUnit,
                Ty::Never => {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: CloneNever executable body",
                    ));
                }
                Ty::Tuple(parts) => {
                    for p in parts.iter() {
                        push(&self.budget, &mut args, p.clone())?;
                    }
                    RecipeDescriptor::CloneTuple {
                        arity: parts.len().try_into().map_err(|_| {
                            FactsError::resource(&self.budget, "tuple Clone arity overflow")
                        })?,
                    }
                }
                Ty::Array { element, len } => {
                    push(&self.budget, &mut args, element.as_ref().clone())?;
                    RecipeDescriptor::CloneArray {
                        len: (*len).try_into().map_err(|_| {
                            FactsError::resource(&self.budget, "array Clone length overflow")
                        })?,
                    }
                }
                Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent,
                } => {
                    self.validate_executable_actual(referent, closed.env())?;
                    push(&self.budget, &mut args, referent.as_ref().clone())?;
                    RecipeDescriptor::CloneSharedRef
                }
                _ => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "builtin Clone evidence has invalid Self shape",
                    ));
                }
            };
            let record = self.catalog.recipe(descriptor)?;
            self.ensure_provider(record.declaration)?;
            target = Some(CallTarget {
                declaration: record.declaration,
                type_args: args.into(),
            });
        } else {
            return Err(FactsError::invalid(
                &self.budget,
                "closed dispatch cannot use assumption or noncallable builtin evidence",
            ));
        }
        let target = target.ok_or_else(|| {
            FactsError::invalid(
                &self.budget,
                "selected implementation has no required item mapping",
            )
        })?;
        let direct = self.discharge_in(&target, closed)?;
        if !self.eq_types(&signature.params, &direct.signature.params, 0)?
            || !self.eq_ty(&signature.result, &direct.signature.result, 0)?
            || !self.return_refines(&signature.ref_contract, &direct.signature.ref_contract)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "resolved implementation signature disagrees with trait declaration",
            ));
        }
        let item = self.item_header(call.item)?;
        let implementation = self.callable_header(target.declaration)?;
        retain::<ReturnRefinementProof>(&self.budget, 1)?;
        let refinement = Arc::new(ReturnRefinementProof {
            world: self.id,
            item: call.item,
            implementation: target.declaration,
            required: item.schema.signature.ref_contract.clone(),
            provided: implementation.schema.signature.ref_contract.clone(),
            topology,
        });
        if let Some(topology) = &refinement.topology {
            self.validate_topology(topology, closed)?;
        }
        // Retain exact rigid source identities, never derive a widened list
        // from the specialized signature or retain an ArcWorld backreference.
        if refinement.world != self.id
            || refinement.item != call.item
            || refinement.implementation != target.declaration
            || !self.eq_contract(&refinement.required, &item.schema.signature.ref_contract)?
            || !self.eq_contract(
                &refinement.provided,
                &implementation.schema.signature.ref_contract,
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "return refinement changed canonical authority",
            ));
        }
        Ok(ResolvedTraitCall {
            callee: Callee::Direct(direct.target),
            signature: direct.signature,
            _evidence: evidence,
            _refinement: Some(refinement),
            _topology: None,
        })
    }
}

fn type_arguments(ty: &Ty) -> &[Ty] {
    match ty {
        Ty::Tuple(v) => v,
        Ty::Struct(v) => &v.def().type_args,
        Ty::Enum(v) => &v.def().type_args,
        _ => &[],
    }
}
impl std::fmt::Debug for ValidatedTraitWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedTraitWorld")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

/// Shared raw fixtures, not a second proof service. Every returned capability
/// goes through the production world builder and its real facts factories.
#[cfg(test)]
pub(crate) mod test_support {
    pub fn source_decl(snapshot: SnapshotId, ordinal: u32, budget: &PlanBudget) -> Result<DeclId> {
        super::catalog::fixture_source_decl(snapshot, ordinal, budget)
    }
    use super::*;
    use ir::{BinderId, FactsCtx, NominalAppId, NominalAppInfo, SubstitutionCx};

    pub(super) struct FixtureSchema {
        registry: SnapshotId,
        budget: PlanBudget,
        declarations: Vec<SchemaDecl>,
        schemas: Vec<(DeclId, GenericSchema)>,
        nominals: Vec<Ty>,
        catalog: Option<Arc<CoreCatalog>>,
    }
    impl FixtureSchema {
        fn register(
            &mut self,
            declaration: DeclId,
            kind: DeclKind,
            env: Arc<GenericEnv>,
            owner: Option<DeclId>,
            receiver: ReceiverMode,
        ) -> Result<()> {
            if declaration.snapshot != self.registry {
                return Err(FactsError::invalid(
                    &self.budget,
                    "fixture declaration belongs to another snapshot",
                ));
            }
            for prior in &self.declarations {
                charged(&self.budget, self.budget.charge_work(1))?;
                if prior.declaration == declaration {
                    return Ok(());
                }
            }
            let own = env
                .scopes()
                .iter()
                .find(|s| s.binder().declaration == declaration)
                .cloned();
            let own_env = match own {
                Some(e) => e,
                None => Arc::new(charged(
                    &self.budget,
                    ParamEnv::new(BinderId { declaration }, 0),
                )?),
            };
            push(
                &self.budget,
                &mut self.declarations,
                SchemaDecl {
                    declaration,
                    kind,
                    own_env,
                    env,
                    owner,
                    module: Arc::from([]),
                    public: false,
                    receiver,
                    region_contract_mode: RegionContractMode::Positioned,
                },
            )
        }
        fn gather(&mut self, ty: &Ty, depth: usize) -> Result<()> {
            charged(&self.budget, self.budget.check_depth(depth))?;
            charged(&self.budget, self.budget.charge_work(1))?;
            if let Some(id) = nominal_id(ty) {
                if id.snapshot != self.registry {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "fixture nominal belongs to another snapshot",
                    ));
                }
                for old in &self.nominals {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if nominal_id(old) == Some(id) {
                        if !same_nominal_object(old, ty) {
                            return Err(FactsError::invalid(
                                &self.budget,
                                "fixture reuses nominal identity with different canonical metadata",
                            ));
                        }
                        return Ok(());
                    }
                }
                push(&self.budget, &mut self.nominals, ty.clone())?;
            }
            for child in type_arguments(ty) {
                self.gather(child, depth + 1)?;
            }
            match ty {
                Ty::Array { element, .. }
                | Ty::Ref {
                    referent: element, ..
                } => self.gather(element, depth + 1)?,
                Ty::Struct(s) => {
                    for f in s.def().fields.iter() {
                        self.gather(&f.ty, depth + 1)?
                    }
                }
                Ty::Enum(e) => {
                    for v in e.def().variants.iter() {
                        for f in v.fields.iter() {
                            self.gather(&f.ty, depth + 1)?
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }
    fn nominal_id(ty: &Ty) -> Option<NominalAppId> {
        match ty {
            Ty::Struct(t) => Some(t.def().id),
            Ty::Enum(t) => Some(t.def().id),
            _ => None,
        }
    }
    fn nominal_decl(ty: &Ty) -> Option<DeclId> {
        match ty {
            Ty::Struct(t) => Some(t.def().source_decl),
            Ty::Enum(t) => Some(t.def().source_decl),
            _ => None,
        }
    }
    fn same_nominal_object(a: &Ty, b: &Ty) -> bool {
        match (a, b) {
            (Ty::Struct(a), Ty::Struct(b)) => std::ptr::eq(a.def(), b.def()),
            (Ty::Enum(a), Ty::Enum(b)) => std::ptr::eq(a.def(), b.def()),
            _ => false,
        }
    }
    fn namespace(ty: &Ty, budget: &PlanBudget, depth: usize) -> Result<Option<SnapshotId>> {
        charged(budget, budget.check_depth(depth))?;
        charged(budget, budget.charge_work(1))?;
        if let Some(d) = nominal_decl(ty) {
            return Ok(Some(d.snapshot));
        }
        for child in type_arguments(ty) {
            if let Some(s) = namespace(child, budget, depth + 1)? {
                return Ok(Some(s));
            }
        }
        if let Ty::Array { element, .. }
        | Ty::Ref {
            referent: element, ..
        } = ty
        {
            return namespace(element, budget, depth + 1);
        }
        Ok(None)
    }
    pub(super) fn adapter(
        headers: &HeaderSet,
        types: &[Ty],
        budget: &PlanBudget,
    ) -> Result<Arc<FixtureSchema>> {
        let registry = headers
            .callables
            .first()
            .map(|c| c.declaration.snapshot)
            .or_else(|| headers.traits.first().map(|c| c.declaration.snapshot))
            .or_else(|| headers.impls.first().map(|c| c.declaration.snapshot));
        let registry = match registry {
            Some(s) => s,
            None => charged(budget, SnapshotId::allocate())?,
        };
        let mut schema = FixtureSchema {
            registry,
            budget: budget.clone(),
            declarations: Vec::new(),
            schemas: Vec::new(),
            nominals: Vec::new(),
            catalog: None,
        };
        for h in headers.traits.iter() {
            schema.register(
                h.declaration,
                DeclKind::Trait,
                h.env.clone(),
                None,
                ReceiverMode::None,
            )?;
        }
        for h in headers.items.iter() {
            schema.register(
                h.declaration,
                DeclKind::TraitItem,
                h.schema.env.clone(),
                Some(h.trait_decl),
                h.receiver,
            )?;
        }
        for h in headers.impls.iter() {
            schema.register(
                h.declaration,
                DeclKind::TraitImpl,
                h.schema_env.clone(),
                None,
                ReceiverMode::None,
            )?;
            schema.gather(&h.trait_ref.self_ty, 0)?;
        }
        for h in headers.callables.iter() {
            let mut owner = None;
            let mut receiver = ReceiverMode::None;
            for implementation in headers.impls.iter() {
                for (item, target) in implementation.items.iter() {
                    charged(budget, budget.charge_work(1))?;
                    if *target == h.declaration {
                        owner = Some(implementation.declaration);
                        for declaration in headers.items.iter() {
                            if declaration.declaration == *item {
                                receiver = declaration.receiver;
                                break;
                            }
                        }
                    }
                }
            }
            schema.register(
                h.declaration,
                if owner.is_some() {
                    DeclKind::AssociatedFunction
                } else {
                    DeclKind::Function
                },
                h.schema.env.clone(),
                owner,
                receiver,
            )?;
        }
        for h in headers.callables.iter() {
            for (index, scope) in h.schema.env.scopes().iter().enumerate() {
                let prefix = Arc::new(charged(
                    budget,
                    GenericEnv::new(&h.schema.env.scopes()[..=index], budget),
                )?);
                schema.register(
                    scope.binder().declaration,
                    DeclKind::Impl,
                    prefix,
                    None,
                    ReceiverMode::None,
                )?;
            }
        }
        for raw in headers
            .callables
            .iter()
            .map(|h| &h.schema)
            .chain(headers.items.iter().map(|h| &h.schema))
        {
            for ty in raw
                .signature
                .params
                .iter()
                .chain(std::iter::once(&raw.signature.result))
            {
                schema.gather(ty, 0)?;
            }
            for Predicate::Implements(goal) in raw.predicates.iter() {
                schema.gather(&goal.self_ty, 0)?;
            }
        }
        for h in headers.impls.iter() {
            for Predicate::Implements(goal) in h.predicates.iter() {
                schema.gather(&goal.self_ty, 0)?;
            }
        }
        for ty in types {
            schema.gather(ty, 0)?;
        }
        for h in headers
            .callables
            .iter()
            .map(|h| (h.declaration, &h.schema))
            .chain(headers.items.iter().map(|h| (h.declaration, &h.schema)))
        {
            retain::<Ty>(budget, h.1.signature.params.len())?;
            push(budget, &mut schema.schemas, (h.0, h.1.clone()))?;
        }
        retain::<FixtureSchema>(budget, 1)?;
        let builder = CoreCatalogBuilder::new(registry, budget)?;
        let mut end = builder.source_start();
        for decl in schema
            .declarations
            .iter()
            .map(|d| d.declaration)
            .chain(schema.nominals.iter().filter_map(nominal_decl))
        {
            charged(budget, budget.charge_work(1))?;
            if decl.index < builder.source_start() {
                return Err(FactsError::invalid(
                    budget,
                    "fixture source declaration collides with canonical catalog prefix",
                ));
            }
            end = end.max(decl.index.checked_add(1).ok_or_else(|| {
                FactsError::resource(budget, "fixture source declaration index overflow")
            })?);
        }
        schema.catalog = Some(builder.finish(end, RecipeShapeInventory::new(), budget)?);
        Ok(Arc::new(schema))
    }
    impl SchemaCx for FixtureSchema {
        fn budget(&self) -> &PlanBudget {
            &self.budget
        }
        fn registry_snapshot(&self) -> SnapshotId {
            self.registry
        }
        fn core_catalog(&self) -> &Arc<CoreCatalog> {
            self.catalog
                .as_ref()
                .expect("fixture catalog must be frozen before adapter publication")
        }
        fn declaration_info(&self, id: DeclId) -> Result<SchemaDecl> {
            if let Some(row) = self.core_catalog().declaration(id) {
                return Ok(row.info.clone());
            }
            for info in &self.declarations {
                charged(&self.budget, self.budget.charge_work(1))?;
                if info.declaration == id {
                    return Ok(info.clone());
                }
            }
            Err(FactsError::invalid(
                &self.budget,
                "fixture declaration is unregistered",
            ))
        }
        fn declaration_schema(&self, id: DeclId) -> Result<GenericSchema> {
            if let Some(row) = self.core_catalog().declaration(id) {
                let schema = row.schema.as_ref().ok_or_else(|| {
                    FactsError::invalid(&self.budget, "catalog declaration has no signature schema")
                })?;
                return clone_schema(schema, &self.budget);
            }
            for (declaration, schema) in &self.schemas {
                charged(&self.budget, self.budget.charge_work(1))?;
                if *declaration == id {
                    retain::<Ty>(&self.budget, schema.signature.params.len())?;
                    charged(
                        &self.budget,
                        self.budget.charge_work(schema.signature.params.len()),
                    )?;
                    return Ok(schema.clone());
                }
            }
            Err(FactsError::invalid(
                &self.budget,
                "fixture has no original declaration schema",
            ))
        }
        fn source_region_env(&self, id: DeclId) -> Result<Arc<ir::SourceRegionEnv>> {
            self.declaration_info(id)?;
            retain::<ir::SourceRegionEnv>(&self.budget, 1)?;
            Ok(Arc::new(ir::SourceRegionEnv {
                declaration: id,
                region_count: 0,
                outlives: Arc::from([]),
                normalized: Arc::from([]),
            }))
        }
        fn instantiate_schema(
            &self,
            schema: &GenericSchema,
            args: &[Ty],
            caller: &GenericEnv,
            budget: &PlanBudget,
        ) -> Result<InstantiatedSchema> {
            if !self.budget.same_authority(budget) {
                return Err(FactsError::invalid(
                    budget,
                    "fixture substituted with a foreign ledger",
                ));
            }
            let mut cx = FixtureSubstitution { schema: self };
            let caller = FactsCtx::Universal(caller);
            let mut item_schema = false;
            for scope in schema.env.scopes() {
                if self.declaration_info(scope.binder().declaration)?.kind == DeclKind::TraitItem {
                    item_schema = true;
                }
            }
            let substitute = |ty: &Ty, cx: &mut FixtureSubstitution<'_>| {
                if item_schema {
                    ir::substitute_schema_ty(ty, &schema.env, args, caller, cx, self, budget)
                } else {
                    ir::substitute_ty(ty, &schema.env, args, caller, cx, budget)
                }
            };
            let mut params = Vec::new();
            for p in &schema.signature.params {
                push(
                    budget,
                    &mut params,
                    charged(budget, substitute(p, &mut cx))?,
                )?;
            }
            let result = charged(budget, substitute(&schema.signature.result, &mut cx))?;
            let mut predicates = Vec::new();
            for Predicate::Implements(goal) in schema.predicates.iter() {
                let self_ty = charged(budget, substitute(&goal.self_ty, &mut cx))?;
                let mut trait_args = Vec::new();
                for a in goal.args.iter() {
                    push(
                        budget,
                        &mut trait_args,
                        charged(budget, substitute(a, &mut cx))?,
                    )?;
                }
                push(
                    budget,
                    &mut predicates,
                    Predicate::Implements(TraitRef {
                        declaration: goal.declaration,
                        self_ty,
                        args: trait_args.into(),
                    }),
                )?;
            }
            Ok(InstantiatedSchema {
                signature: Signature {
                    ref_contract: schema.signature.ref_contract.clone(),
                    params,
                    result,
                },
                predicates: predicates.into(),
            })
        }
        fn validate_nominal(&self, ty: &Ty, _env: &GenericEnv, budget: &PlanBudget) -> Result<()> {
            if !self.budget.same_authority(budget) {
                return Err(FactsError::invalid(
                    budget,
                    "fixture validated with a foreign ledger",
                ));
            }
            for registered in &self.nominals {
                charged(budget, budget.charge_work(1))?;
                if same_nominal_object(registered, ty) {
                    return Ok(());
                }
            }
            Err(FactsError::invalid(
                budget,
                "fixture nominal metadata is not canonical in this registry",
            ))
        }
    }
    struct FixtureSubstitution<'a> {
        schema: &'a FixtureSchema,
    }
    impl SubstitutionCx for FixtureSubstitution<'_> {
        fn nominal_application(
            &self,
            id: NominalAppId,
        ) -> std::result::Result<NominalAppInfo, String> {
            for ty in &self.schema.nominals {
                self.schema.budget.charge_work(1)?;
                if nominal_id(ty) == Some(id) {
                    return Ok(NominalAppInfo {
                        source_decl: nominal_decl(ty).ok_or("fixture nominal kind mismatch")?,
                        type_args: type_arguments(ty).into(),
                    });
                }
            }
            Err("unregistered fixture nominal application".into())
        }
        fn intern_nominal(
            &mut self,
            declaration: DeclId,
            args: &[Ty],
            _caller: FactsCtx<'_>,
            budget: &PlanBudget,
        ) -> std::result::Result<Ty, String> {
            for ty in &self.schema.nominals {
                budget.charge_work(1)?;
                if nominal_decl(ty) == Some(declaration) {
                    // These fixtures register exact canonical applications. They do
                    // not manufacture a parallel interner or infer Copy by shape.
                    budget.charge_work(args.len())?;
                    if type_arguments(ty) == args {
                        return Ok(ty.clone());
                    }
                }
            }
            Err("fixture nominal application was not registered; use the real frontend interner for generative cases".into())
        }
        fn instantiate_signature(
            &mut self,
            _target: &CallTarget,
            _caller: FactsCtx<'_>,
            _budget: &PlanBudget,
        ) -> std::result::Result<Signature, String> {
            Err("raw fixture substitution cannot select or prove a call".into())
        }
    }
    pub fn world(
        callables: &[CallableHeader],
        budget: &PlanBudget,
    ) -> Result<Arc<ValidatedTraitWorld>> {
        retain::<CallableHeader>(budget, callables.len())?;
        for callable in callables {
            retain::<Ty>(budget, callable.schema.signature.params.len())?;
        }
        let headers = HeaderSet {
            callables: callables.into(),
            ..HeaderSet::default()
        };
        let raw = adapter(&headers, &[], budget)?;
        ValidatedTraitWorld::build(headers, raw, budget)
    }
    pub fn facts_for_signature(
        signature: &Signature,
        types: &[Ty],
        budget: &PlanBudget,
    ) -> Result<Arc<ValidatedFacts>> {
        let mut registry = None;
        for ty in types
            .iter()
            .chain(signature.params.iter())
            .chain(std::iter::once(&signature.result))
        {
            if let Some(s) = namespace(ty, budget, 0)? {
                registry = Some(s);
                break;
            }
        }
        let registry = match registry {
            Some(s) => s,
            None => charged(budget, SnapshotId::allocate())?,
        };
        // Discover nominal declaration indices before choosing the fixture's
        // callable identity. A nominal at index zero is not a function at zero.
        let empty_headers = HeaderSet::default();
        let mut raw = FixtureSchema {
            registry,
            budget: budget.clone(),
            declarations: Vec::new(),
            schemas: Vec::new(),
            nominals: Vec::new(),
            catalog: None,
        };
        for ty in types
            .iter()
            .chain(signature.params.iter())
            .chain(std::iter::once(&signature.result))
        {
            raw.gather(ty, 0)?;
        }
        let mut index = source_decl(registry, 0, budget)?.index;
        for nominal in &raw.nominals {
            let decl = nominal_decl(nominal)
                .ok_or_else(|| FactsError::invalid(budget, "fixture nominal kind mismatch"))?;
            index = index.max(decl.index.checked_add(1).ok_or_else(|| {
                FactsError::resource(budget, "fixture declaration indices exhausted")
            })?);
        }
        let declaration = DeclId {
            snapshot: registry,
            index,
        };
        let env = Arc::new(charged(budget, GenericEnv::new(&[], budget))?);
        retain::<Ty>(budget, signature.params.len())?;
        let callable = CallableHeader {
            declaration,
            schema: GenericSchema {
                env: env.clone(),
                signature: signature.clone(),
                predicates: Arc::from([]),
            },
        };
        raw.register(
            declaration,
            DeclKind::Function,
            env,
            None,
            ReceiverMode::None,
        )?;
        retain::<Ty>(budget, callable.schema.signature.params.len())?;
        push(
            budget,
            &mut raw.schemas,
            (declaration, callable.schema.clone()),
        )?;
        let headers = HeaderSet {
            callables: Arc::from([callable]),
            ..empty_headers
        };
        let raw = adapter(&headers, types, budget)?;
        let world = ValidatedTraitWorld::build(headers, raw, budget)?;
        let facts = world.instance_facts(&CallTarget {
            declaration,
            type_args: Arc::from([]),
        })?;
        for ty in types {
            facts.validate_ty(ty)?;
        }
        Ok(facts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ir::{
        BinderId, BorrowKind, FieldDef, IntTy, NominalAppId, RefContract, StructDef, StructTy,
    };

    struct Case {
        budget: PlanBudget,
        headers: HeaderSet,
        types: Vec<Ty>,
        caller: DeclId,
        body: DeclId,
        item: DeclId,
        value_trait: DeclId,
        caller_parameter: Ty,
        leaf: Ty,
        boxed: Ty,
    }
    fn scope(declaration: DeclId, arity: usize, budget: &PlanBudget) -> Arc<ParamEnv> {
        retain::<ParamEnv>(budget, 1).unwrap();
        Arc::new(ParamEnv::new(BinderId { declaration }, arity).unwrap())
    }
    fn env(scopes: &[Arc<ParamEnv>], budget: &PlanBudget) -> Arc<GenericEnv> {
        Arc::new(GenericEnv::new(scopes, budget).unwrap())
    }
    fn sig(params: Vec<Ty>, result: Ty) -> Signature {
        Signature {
            ref_contract: Arc::new(RefContract::default()),
            params,
            result,
        }
    }
    fn nominal(registry: SnapshotId, index: usize, declaration: DeclId, args: Vec<Ty>) -> Ty {
        Ty::Struct(StructTy::new(StructDef {
            id: NominalAppId {
                snapshot: registry,
                index,
            },
            source_decl: declaration,
            type_args: args.into(),
            name: "fixture".into(),
            module: String::new(),
            public: false,
            fields: Arc::<[FieldDef]>::from([]),
        }))
    }
    fn case() -> Case {
        let budget = PlanBudget::default();
        let registry = SnapshotId::allocate().unwrap();
        let id = |index| test_support::source_decl(registry, index, &budget).unwrap();
        let i32_ty = Ty::Int(IntTy::I32);
        let marker_self = scope(id(0), 1, &budget);
        let marker_env = env(std::slice::from_ref(&marker_self), &budget);
        let value_self = scope(id(2), 1, &budget);
        let value_env = env(std::slice::from_ref(&value_self), &budget);
        let item_scope = scope(id(3), 0, &budget);
        let item_env = env(&[value_self.clone(), item_scope], &budget);
        let implementation_scope = scope(id(5), 1, &budget);
        let implementation_env = env(std::slice::from_ref(&implementation_scope), &budget);
        let parameter = Ty::Param(ParamId {
            owner: implementation_scope.binder(),
            index: 0,
        });
        let leaf = nominal(registry, 2, id(6), Vec::new());
        let pattern = nominal(registry, 0, id(4), vec![parameter.clone()]);
        let boxed = nominal(registry, 1, id(4), vec![leaf.clone()]);
        let body_scope = scope(id(9), 0, &budget);
        let body_env = env(&[implementation_scope, body_scope], &budget);
        let caller_scope = scope(id(10), 1, &budget);
        let caller_env = env(std::slice::from_ref(&caller_scope), &budget);
        let caller_parameter = Ty::Param(ParamId {
            owner: caller_scope.binder(),
            index: 0,
        });
        let empty = env(&[], &budget);
        let headers = HeaderSet {
            traits: Arc::from([
                TraitHeader {
                    declaration: id(0),
                    self_param: ParamId {
                        owner: marker_self.binder(),
                        index: 0,
                    },
                    env: marker_env,
                    items: Arc::from([]),
                    supertraits: Arc::from([]),
                },
                TraitHeader {
                    declaration: id(2),
                    self_param: ParamId {
                        owner: value_self.binder(),
                        index: 0,
                    },
                    env: value_env,
                    items: Arc::from([id(3)]),
                    supertraits: Arc::from([]),
                },
            ]),
            items: Arc::from([TraitItemHeader {
                declaration: id(3),
                trait_decl: id(2),
                receiver: ReceiverMode::Value,
                default: None,
                schema: GenericSchema {
                    env: item_env,
                    signature: sig(
                        vec![Ty::Param(ParamId {
                            owner: value_self.binder(),
                            index: 0,
                        })],
                        i32_ty.clone(),
                    ),
                    predicates: Arc::from([]),
                },
            }]),
            impls: Arc::from([
                ImplHeader {
                    declaration: id(5),
                    trait_ref: TraitRef {
                        declaration: id(2),
                        self_ty: pattern.clone(),
                        args: Arc::from([]),
                    },
                    schema_env: implementation_env,
                    predicates: Arc::from([Predicate::Implements(TraitRef {
                        declaration: id(0),
                        self_ty: parameter,
                        args: Arc::from([]),
                    })]),
                    items: Arc::from([(id(3), id(9))]),
                },
                ImplHeader {
                    declaration: id(7),
                    trait_ref: TraitRef {
                        declaration: id(0),
                        self_ty: leaf.clone(),
                        args: Arc::from([]),
                    },
                    schema_env: empty,
                    predicates: Arc::from([]),
                    items: Arc::from([]),
                },
            ]),
            callables: Arc::from([
                CallableHeader {
                    declaration: id(9),
                    schema: GenericSchema {
                        env: body_env,
                        signature: sig(vec![pattern.clone()], i32_ty.clone()),
                        predicates: Arc::from([]),
                    },
                },
                CallableHeader {
                    declaration: id(10),
                    schema: GenericSchema {
                        env: caller_env,
                        signature: sig(vec![caller_parameter.clone()], i32_ty),
                        predicates: Arc::from([Predicate::Implements(TraitRef {
                            declaration: id(2),
                            self_ty: caller_parameter.clone(),
                            args: Arc::from([]),
                        })]),
                    },
                },
            ]),
        };
        Case {
            budget: budget.clone(),
            headers,
            types: vec![pattern, boxed.clone(), leaf.clone()],
            caller: id(10),
            body: id(9),
            item: id(3),
            value_trait: id(2),
            caller_parameter,
            leaf,
            boxed,
        }
    }
    impl Case {
        fn build(&self) -> Result<Arc<ValidatedTraitWorld>> {
            let adapter = test_support::adapter(&self.headers, &self.types, &self.budget)?;
            ValidatedTraitWorld::build(self.headers.clone(), adapter, &self.budget)
        }
        fn goal(&self, self_ty: Ty) -> Predicate {
            Predicate::Implements(TraitRef {
                declaration: self.value_trait,
                self_ty,
                args: Arc::from([]),
            })
        }
        fn call(&self, self_ty: Ty) -> TraitCall {
            TraitCall {
                trait_ref: TraitRef {
                    declaration: self.value_trait,
                    self_ty,
                    args: Arc::from([]),
                },
                item: self.item,
                method_args: Arc::from([]),
            }
        }
    }

    fn reference(ty: Ty) -> Ty {
        Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        }
    }

    fn reference_signature(params: Vec<Ty>, result: Ty, budget: &PlanBudget) -> Signature {
        let mut input_regions = Vec::new();
        for (param, ty) in params.iter().enumerate() {
            for depth in 0..ir::root_reference_layers(ty).unwrap() {
                push(
                    budget,
                    &mut input_regions,
                    ir::InputRegion {
                        input: ir::InputReference {
                            param,
                            position: ir::ReferencePosition { depth: depth as u8 },
                        },
                        region: ir::RegionParamId(0),
                    },
                )
                .unwrap();
            }
        }
        let mut results = Vec::new();
        for depth in 0..ir::root_reference_layers(&result).unwrap() {
            push(
                budget,
                &mut results,
                ir::ResultReference {
                    position: ir::ReferencePosition { depth: depth as u8 },
                    region: ir::RegionParamId(0),
                    returns: ir::ReturnContract::NoReference,
                },
            )
            .unwrap();
        }
        let raw = RefContract {
            region_count: 1,
            input_regions: input_regions.into(),
            results: results.into(),
            ..RefContract::default()
        };
        let mut contract = (*ir::normalize_ref_contract(&raw, budget).unwrap()).clone();
        contract.results =
            ir::derive_return_contracts(&params, &result, &contract, budget).unwrap();
        Signature {
            params,
            result,
            ref_contract: Arc::new(contract),
        }
    }

    fn borrowed_case() -> Case {
        let mut c = case();
        let item = &mut Arc::make_mut(&mut c.headers.items)[0];
        item.receiver = ReceiverMode::Borrowed {
            kind: BorrowKind::Shared,
        };
        item.schema.signature = reference_signature(
            vec![reference(item.schema.signature.params[0].clone())],
            reference(Ty::Int(IntTy::I32)),
            &c.budget,
        );
        let implementation = &mut Arc::make_mut(&mut c.headers.callables)[0];
        implementation.schema.signature = reference_signature(
            vec![reference(implementation.schema.signature.params[0].clone())],
            reference(Ty::Int(IntTy::I32)),
            &c.budget,
        );
        c
    }

    fn nested_header(budget: &PlanBudget) -> CallableHeader {
        let declaration =
            test_support::source_decl(SnapshotId::allocate().unwrap(), 0, budget).unwrap();
        let binder = scope(declaration, 1, budget);
        let environment = env(std::slice::from_ref(&binder), budget);
        let parameter = Ty::Param(ParamId {
            owner: binder.binder(),
            index: 0,
        });
        let nested = reference(reference(parameter));
        let inputs = [0u8, 1].map(|depth| ir::InputRegion {
            input: ir::InputReference {
                param: 0,
                position: ir::ReferencePosition { depth },
            },
            region: ir::RegionParamId(u16::from(depth)),
        });
        let results = [0u8, 1].map(|depth| ir::ResultReference {
            position: ir::ReferencePosition { depth },
            region: ir::RegionParamId(u16::from(depth)),
            returns: ir::ReturnContract::NoReference,
        });
        let raw = RefContract {
            region_count: 2,
            input_regions: Arc::from(inputs),
            results: Arc::from(results),
            outlives: Arc::from([(ir::RegionParamId(1), ir::RegionParamId(0))]),
        };
        let mut contract = (*ir::normalize_ref_contract(&raw, budget).unwrap()).clone();
        contract.results =
            ir::derive_return_contracts(std::slice::from_ref(&nested), &nested, &contract, budget)
                .unwrap();
        CallableHeader {
            declaration,
            schema: GenericSchema {
                env: environment,
                signature: Signature {
                    params: vec![nested.clone()],
                    result: nested,
                    ref_contract: Arc::new(contract),
                },
                predicates: Arc::from([]),
            },
        }
    }

    #[test]
    fn nested_reference_world_positioned_schema_and_canonical_transport() {
        let budget = PlanBudget::default();
        let header = nested_header(&budget);
        let declaration = header.declaration;
        let world = test_support::world(std::slice::from_ref(&header), &budget).unwrap();
        let universal = world.callable_facts(declaration).unwrap();
        universal
            .validate_signature(SignatureSubject::Body, &header.schema.signature)
            .unwrap();
        let target = CallTarget {
            declaration,
            type_args: Arc::from([Ty::Int(IntTy::I32)]),
        };
        let closed = world.instance_facts(&target).unwrap();
        let proof = world.discharge_direct(&target, &closed).unwrap();
        assert_eq!(
            proof.signature().ref_contract,
            header.schema.signature.ref_contract
        );
        assert_eq!(proof.signature().ref_contract.results.len(), 2);
        closed
            .validate_signature(SignatureSubject::Body, proof.signature())
            .unwrap();
        let mut forged = proof.signature().clone();
        let contract = Arc::make_mut(&mut forged.ref_contract);
        Arc::make_mut(&mut contract.results)[1].returns =
            ir::ReturnContract::SharedInputEnvelopes(Arc::from([ir::InputEnvelope {
                input: ir::InputReference {
                    param: 0,
                    position: ir::ReferencePosition { depth: 0 },
                },
            }]));
        assert!(
            closed
                .validate_signature(SignatureSubject::Body, &forged)
                .is_err()
        );
        let mut omitted = header.schema.signature.clone();
        Arc::make_mut(&mut omitted.ref_contract).results = Arc::from([]);
        assert!(world.validate_position_coverage(&omitted).is_err());
    }

    #[test]
    fn nested_reference_world_original_schema_and_legacy_subject_authority() {
        use crate::compiler::frontend;
        let budget = PlanBudget::default();
        let program = frontend::parse(
            "fn legacy(x: &i32) -> i32 { *x } fn named<'a>(x: &'a i32) -> i32 { *x } fn main() {}",
        )
        .unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let legacy = plan.callable_decl(0).unwrap();
        let named = plan.callable_decl(1).unwrap();
        assert_eq!(
            world.info(legacy).unwrap().region_contract_mode,
            RegionContractMode::LegacyUnnamedFreeNoReturn
        );
        assert_eq!(
            world.info(named).unwrap().region_contract_mode,
            RegionContractMode::Positioned
        );
        let original = world.callable_header(legacy).unwrap();
        assert!(
            original
                .schema
                .signature
                .ref_contract
                .input_regions
                .is_empty()
        );
        world
            .callable_facts(legacy)
            .unwrap()
            .validate_signature(SignatureSubject::Body, &original.schema.signature)
            .unwrap();
        let mut forged = world.headers.clone();
        let named_header = Arc::make_mut(&mut forged.callables)
            .iter_mut()
            .find(|h| h.declaration == named)
            .unwrap();
        named_header.schema.signature.ref_contract = Arc::new(RefContract::default());
        assert!(
            matches!(ValidatedTraitWorld::build(forged, world.schema.clone(), &budget), Err(e) if e.kind() == FactsErrorKind::InvalidMetadata)
        );
        let declaration =
            test_support::source_decl(SnapshotId::allocate().unwrap(), 0, &budget).unwrap();
        let empty = env(&[], &budget);
        let raw = CallableHeader {
            declaration,
            schema: GenericSchema {
                env: empty,
                signature: sig(vec![reference(Ty::Int(IntTy::I32))], Ty::Unit),
                predicates: Arc::from([]),
            },
        };
        assert!(test_support::world(&[raw], &budget).is_err());
    }

    #[test]
    fn nested_reference_world_per_position_refinement_and_type_use_context() {
        let budget = PlanBudget::default();
        let header = nested_header(&budget);
        let world = test_support::world(std::slice::from_ref(&header), &budget).unwrap();
        let facts = world.callable_facts(header.declaration).unwrap();
        let required = &header.schema.signature.ref_contract;
        let mut provided = (**required).clone();
        Arc::make_mut(&mut provided.results)[1].returns =
            ir::ReturnContract::SharedInputPaths(Arc::from([ir::InputPath {
                input: ir::InputReference {
                    param: 0,
                    position: ir::ReferencePosition { depth: 1 },
                },
                projection: Arc::from([]),
            }]));
        assert!(world.return_refines(required, &provided).unwrap());
        Arc::make_mut(&mut provided.results)[1].returns =
            ir::ReturnContract::SharedInputPaths(Arc::from([ir::InputPath {
                input: ir::InputReference {
                    param: 0,
                    position: ir::ReferencePosition { depth: 0 },
                },
                projection: Arc::from([]),
            }]));
        assert!(!world.return_refines(required, &provided).unwrap());
        let nested = header.schema.signature.params[0].clone();
        let mut walk = TypeWalk::default();
        world
            .validate_ty_inner(&nested, facts.env(), 0, TypeUse::Root, &mut walk)
            .unwrap();
        world
            .validate_ty_inner(
                &nested,
                facts.env(),
                0,
                TypeUse::Referent {
                    depth: 1,
                    shared: true,
                },
                &mut walk,
            )
            .unwrap();
        let deeper = reference(nested.clone());
        facts.validate_ty(&deeper).unwrap();
        let carriers = ir::carrier_locations(&deeper, &budget).unwrap();
        assert_eq!(carriers.len(), 4);
        for (depth, carrier) in carriers[..3].iter().enumerate() {
            assert!(carrier.path.0.is_empty());
            assert_eq!(
                carrier.channel,
                ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: depth as u8 })
            );
        }
        assert!(carriers[3].path.0.is_empty());
        assert_eq!(
            carriers[3].channel,
            ir::CarrierChannel::ParamTail {
                parameter: facts.env().ordered_params().next().unwrap(),
                prefix: 3,
            }
        );
        assert!(world.validate_actual(&nested, facts.env()).is_err());
        let mutable = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: Arc::new(reference(Ty::Int(IntTy::I32))),
        };
        facts.validate_ty(&mutable).unwrap();
        assert_eq!(ir::carrier_locations(&mutable, &budget).unwrap().len(), 2);
        let empty_product = Ty::Array {
            element: Arc::new(nested),
            len: 0,
        };
        facts.validate_ty(&empty_product).unwrap();
        assert!(
            ir::carrier_locations(&empty_product, &budget)
                .unwrap()
                .is_empty()
        );
        assert!(
            world
                .validate_executable_actual(&empty_product, facts.env())
                .is_err()
        );
        // Typed structural validation grants no verified family body capability.
    }

    #[test]
    fn generic_reference_world_transport_and_type_context() {
        // Legacy erasure must survive scalar instances, but a reference actual
        // must restore every original fixed input before realizing its tails.
        let legacy_budget = PlanBudget::default();
        let program = crate::compiler::frontend::parse(
            "fn g<T>(d:&mut T,x:T,y:T)->T{*d=x;y} fn h<T>(d:&mut T,x:T,z:&i32)->T{*d=x;loop{}} fn main(){}",
        )
        .unwrap();
        let plan =
            crate::compiler::frontend::PlanCtx::from_program(&program, &legacy_budget).unwrap();
        let legacy_world = plan.trait_world().unwrap();
        for callable in 0..2 {
            let declaration = plan.callable_decl(callable).unwrap();
            let facts = legacy_world.callable_facts(declaration).unwrap();
            let schema = legacy_world.callable_schema(declaration).unwrap();
            let scalar = legacy_world
                .discharge_direct(
                    &CallTarget {
                        declaration,
                        type_args: Arc::from([Ty::Int(IntTy::I32)]),
                    },
                    &facts,
                )
                .unwrap();
            assert_eq!(scalar.signature().ref_contract.region_count, 0);
            assert!(scalar.signature().ref_contract.input_regions.is_empty());
            for layers in [1, 3] {
                let mut actual = Ty::Int(IntTy::I32);
                for _ in 0..layers {
                    actual = Ty::Ref {
                        kind: BorrowKind::Mut,
                        referent: Arc::new(actual),
                    };
                }
                let target = CallTarget {
                    declaration,
                    type_args: Arc::from([actual]),
                };
                let proof = legacy_world.discharge_direct(&target, &facts).unwrap();
                let contract = &proof.signature().ref_contract;
                assert_eq!(usize::from(contract.region_count), layers + 1 + callable);
                assert_eq!(contract.input_regions[0].input.param, 0);
                assert_eq!(contract.input_regions[0].input.position.depth, 0);
                for rows in contract.input_regions.windows(2) {
                    if rows[0].input.param == rows[1].input.param {
                        assert!(
                            contract
                                .outlives_region(rows[1].region, rows[0].region)
                                .unwrap()
                        );
                    }
                }
                assert_eq!(contract.input_regions.last().unwrap().input.param, 2);
                assert_eq!(contract.results.len(), layers);
                assert!(
                    legacy_world
                        .realize_ref_contract(&schema, &target.type_args, proof.signature(), None)
                        .is_err()
                );
            }
        }
        let budget = PlanBudget::default();
        let declaration =
            test_support::source_decl(SnapshotId::allocate().unwrap(), 0, &budget).unwrap();
        let scope = scope(declaration, 2, &budget);
        let environment = env(std::slice::from_ref(&scope), &budget);
        let parameter = |index| {
            Ty::Param(ParamId {
                owner: scope.binder(),
                index,
            })
        };
        let signature = reference_signature(
            vec![reference(parameter(0)), reference(parameter(1))],
            reference(parameter(0)),
            &budget,
        );
        let world = test_support::world(
            &[CallableHeader {
                declaration,
                schema: GenericSchema {
                    env: environment,
                    signature: signature.clone(),
                    predicates: Arc::from([]),
                },
            }],
            &budget,
        )
        .unwrap();
        let universal = world.callable_facts(declaration).unwrap();
        universal
            .validate_signature(SignatureSubject::Body, &signature)
            .unwrap();
        let target = CallTarget {
            declaration,
            type_args: Arc::from([Ty::Int(IntTy::I32), Ty::Int(IntTy::I32)]),
        };
        let closed = world.instance_facts(&target).unwrap();
        let canonical = world.discharge_direct(&target, &closed).unwrap();
        assert!(matches!(
            canonical.signature().ref_contract.results[0].returns,
            ir::ReturnContract::SharedInputEnvelopes(_)
        ));
        closed
            .validate_signature(SignatureSubject::Body, canonical.signature())
            .unwrap();
        let mut forged = canonical.signature().clone();
        Arc::make_mut(&mut Arc::make_mut(&mut forged.ref_contract).results)[0].returns =
            ir::ReturnContract::SharedInputPaths(Arc::from([
                ir::InputPath {
                    input: ir::InputReference {
                        param: 0,
                        position: ir::ReferencePosition { depth: 0 },
                    },
                    projection: Arc::from([]),
                },
                ir::InputPath {
                    input: ir::InputReference {
                        param: 1,
                        position: ir::ReferencePosition { depth: 0 },
                    },
                    projection: Arc::from([]),
                },
            ]));
        assert!(
            closed
                .validate_signature(SignatureSubject::Body, &forged)
                .is_err()
        );
        let transported = world
            .instance_facts(&CallTarget {
                declaration,
                type_args: Arc::from([reference(Ty::Int(IntTy::I32)), Ty::Int(IntTy::I32)]),
            })
            .unwrap();
        let view = transported
            .carrier_contract(SignatureSubject::Body)
            .unwrap();
        assert_eq!(view.ref_contract().input_regions.len(), 3);
        assert_eq!(view.contract().inputs.len(), 3);
        assert!(
            world
                .instance_facts(&CallTarget {
                    declaration,
                    type_args: Arc::from([
                        Ty::Tuple(Arc::from([reference(Ty::Int(IntTy::I32))])),
                        Ty::Int(IntTy::I32)
                    ]),
                })
                .is_err()
        );
        // Structural facts never create a verified executable template or body.
        assert_eq!(transported.phase(), FactsPhase::Closed);
        let root = reference(parameter(0));
        let mut walk = TypeWalk::default();
        world
            .validate_ty_inner(&root, universal.env(), 0, TypeUse::Root, &mut walk)
            .unwrap();
        world
            .validate_ty_inner(&root, universal.env(), 0, TypeUse::Aggregate, &mut walk)
            .unwrap();
        assert!(world.validate_actual(&root, universal.env()).is_err());
        assert!(universal.must_copy(&root).unwrap());
    }

    #[test]
    fn generic_reference_world_private_refinement_and_universal_closed_goal() {
        let c = borrowed_case();
        let world = c.build().unwrap();
        let universal = world.callable_facts(c.caller).unwrap();
        let open = world
            .resolve_trait_call(&c.call(c.caller_parameter.clone()), &universal)
            .unwrap();
        assert!(matches!(open.callee(), Callee::TraitItem(_)));
        assert!(open._refinement.is_none());
        let resolved = world
            .resolve_trait_call(&c.call(c.boxed.clone()), &universal)
            .unwrap();
        let Callee::Direct(target) = resolved.callee() else {
            panic!("closed goal was not privately resolved")
        };
        assert_eq!(target.declaration, c.body);
        assert!(resolved._refinement.is_some());
        // Generic implementation declaration origins stay abstract even when
        // this particular instantiation has fully concrete actuals.
        assert!(matches!(
            resolved.signature().ref_contract.results[0].returns,
            ir::ReturnContract::SharedInputEnvelopes(_)
        ));
        assert_eq!(
            world
                .callee_receiver(resolved.callee(), &universal)
                .unwrap(),
            ReceiverMode::Borrowed {
                kind: BorrowKind::Shared
            }
        );
        let old = Callee::TraitItem(c.call(c.boxed.clone()));
        let required = world
            .instantiate_trait_signature(&c.call(c.boxed.clone()), &universal)
            .unwrap();
        universal
            .validate_signature(SignatureSubject::Call(&old), &required)
            .unwrap();
        universal
            .validate_signature(
                SignatureSubject::Call(resolved.callee()),
                resolved.signature(),
            )
            .unwrap();
    }

    #[test]
    fn generic_reference_world_schema_self_regions_and_refinement_denials() {
        let mut c = borrowed_case();
        let world = c.build().unwrap();
        let schema = world.schema_facts(c.item).unwrap();
        let self_ty = Ty::Param(schema.env().ordered_params().next().unwrap());
        schema.validate_ty(&reference(self_ty.clone())).unwrap();
        assert!(
            schema
                .validate_ty(&reference(Ty::Array {
                    element: Arc::new(self_ty),
                    len: 0
                }))
                .is_err()
        );
        assert!(schema.must_copy(&reference(Ty::Int(IntTy::I32))).is_err());
        let required = &c.headers.items[0].schema.signature.ref_contract;
        let mut narrowed = (**required).clone();
        Arc::make_mut(&mut narrowed.results)[0].returns =
            ir::ReturnContract::SharedInputPaths(Arc::from([ir::InputPath {
                input: ir::InputReference {
                    param: 0,
                    position: ir::ReferencePosition { depth: 0 },
                },
                projection: Arc::from([]),
            }]));
        assert!(world.return_refines(required, &narrowed).unwrap());
        Arc::make_mut(&mut narrowed.input_regions)[0].region = ir::RegionParamId(1);
        assert!(!world.return_refines(required, &narrowed).unwrap());
        Arc::make_mut(&mut c.headers.items)[0].receiver = ReceiverMode::Borrowed {
            kind: BorrowKind::Mut,
        };
        assert!(c.build().is_err());
    }

    #[test]
    fn trait_world_conditional_assumptions_and_closed_dispatch() {
        let c = case();
        let world = c.build().unwrap();
        let universal = world.callable_facts(c.caller).unwrap();
        assert!(matches!(
            world
                .prove_goal(&c.goal(c.caller_parameter.clone()), &universal)
                .unwrap(),
            ProofOutcome::Proved(_)
        ));
        let abstract_call = world
            .resolve_trait_call(&c.call(c.caller_parameter.clone()), &universal)
            .unwrap();
        assert!(matches!(abstract_call.callee(), Callee::TraitItem(_)));
        assert!(!universal.must_copy(&c.caller_parameter).unwrap());
        let target = CallTarget {
            declaration: c.caller,
            type_args: Arc::from([c.boxed.clone()]),
        };
        let closed = world.instance_facts(&target).unwrap();
        assert!(closed.env().scopes().is_empty());
        assert!(closed.predicates().predicates().is_empty());
        assert_eq!(closed.target(), Some(&target));
        let resolved = world
            .resolve_trait_call(&c.call(c.boxed.clone()), &closed)
            .unwrap();
        let Callee::Direct(selected) = resolved.callee() else {
            panic!("closed call retained a trait item")
        };
        assert_eq!(selected.declaration, c.body);
        assert_eq!(selected.type_args.as_ref(), std::slice::from_ref(&c.leaf));
        assert!(
            world
                .signatures_equal(
                    resolved.signature(),
                    &sig(vec![c.boxed.clone()], Ty::Int(IntTy::I32))
                )
                .unwrap()
        );
        // A convenient implementation for Box<Leaf> is no proof for a rigid T.
        let body_facts = world.callable_facts(c.body).unwrap();
        assert!(
            matches!(world.prove_goal(&c.goal(c.caller_parameter.clone()),&body_facts),Err(e)if e.kind()==FactsErrorKind::InvalidMetadata)
        );
        let wrong_entry = CallTarget {
            declaration: c.caller,
            type_args: Arc::from([c.leaf.clone()]),
        };
        assert!(
            matches!(world.instance_facts(&wrong_entry),Err(e)if e.kind()==FactsErrorKind::Unproved)
        );
        let schema = world.schema_facts(c.item).unwrap();
        assert!(schema.must_copy(&Ty::Int(IntTy::I32)).is_err());
        assert!(world.callable_facts(c.item).is_err());
    }

    #[test]
    fn trait_world_conformance_coherence_schema_and_scope_authority() {
        let mut c = case();
        let world = c.build().unwrap();
        let schema = world.schema_facts(c.item).unwrap();
        let self_ty = schema.env().ordered_params().next().map(Ty::Param).unwrap();
        schema.validate_ty(&self_ty).unwrap();
        assert!(
            matches!(schema.validate_ty(&Ty::Array{element:Arc::new(self_ty),len:0}),Err(e)if e.kind()==FactsErrorKind::Unsupported)
        );
        let foreign = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        assert!(
            foreign
                .prove_goal(&c.goal(c.caller_parameter.clone()), &facts)
                .is_err()
        );
        let mut callables = c.headers.callables.to_vec();
        callables[0].schema.signature.result = Ty::Bool;
        c.headers.callables = callables.into();
        assert!(matches!(c.build(),Err(e)if e.kind()==FactsErrorKind::InvalidMetadata));
        let mut c = case();
        let mut implementations = c.headers.impls.to_vec();
        let mut overlap = implementations[1].clone();
        overlap.declaration =
            test_support::source_decl(overlap.declaration.snapshot, 11, &c.budget).unwrap();
        implementations.push(overlap);
        c.headers.impls = implementations.into();
        assert!(matches!(c.build(),Err(e)if e.kind()==FactsErrorKind::Ambiguous));
        struct ErasedScope {
            raw: Arc<dyn SchemaCx + Send + Sync>,
            declaration: DeclId,
        }
        impl SchemaCx for ErasedScope {
            fn source_region_env(&self, id: DeclId) -> Result<Arc<ir::SourceRegionEnv>> {
                self.raw.source_region_env(id)
            }
            fn core_catalog(&self) -> &Arc<CoreCatalog> {
                self.raw.core_catalog()
            }
            fn declaration_schema(&self, id: DeclId) -> Result<GenericSchema> {
                self.raw.declaration_schema(id)
            }
            fn budget(&self) -> &PlanBudget {
                self.raw.budget()
            }
            fn registry_snapshot(&self) -> SnapshotId {
                self.raw.registry_snapshot()
            }
            fn declaration_info(&self, id: DeclId) -> Result<SchemaDecl> {
                let mut row = self.raw.declaration_info(id)?;
                if id == self.declaration {
                    let scopes = row.env.scopes();
                    row.env = env(&scopes[..scopes.len() - 1], self.budget());
                }
                Ok(row)
            }
            fn instantiate_schema(
                &self,
                s: &GenericSchema,
                a: &[Ty],
                c: &GenericEnv,
                b: &PlanBudget,
            ) -> Result<InstantiatedSchema> {
                self.raw.instantiate_schema(s, a, c, b)
            }
            fn validate_nominal(&self, t: &Ty, e: &GenericEnv, b: &PlanBudget) -> Result<()> {
                self.raw.validate_nominal(t, e, b)
            }
        }
        let c = case();
        let raw = test_support::adapter(&c.headers, &c.types, &c.budget).unwrap();
        let erased = Arc::new(ErasedScope {
            raw,
            declaration: c.body,
        });
        assert!(
            matches!(ValidatedTraitWorld::build(c.headers,erased,&c.budget),Err(e)if e.kind()==FactsErrorKind::InvalidMetadata)
        );
    }

    #[test]
    fn trait_world_shared_resources_cache_and_local_cycle_boundaries() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let goal = c.goal(c.caller_parameter.clone());
        let key = GoalKey {
            environment: facts.predicates.index,
            phase: facts.phase(),
            goal: goal.clone(),
        };
        c.budget.charge_obligation().unwrap();
        push(
            &c.budget,
            &mut world.memo.lock().unwrap(),
            MemoEntry {
                key: key.clone(),
                state: MemoState::Active,
            },
        )
        .unwrap();
        // Another worker's Active entry is not this operation's DFS ancestry.
        assert!(matches!(
            world.prove_goal(&goal, &facts).unwrap(),
            ProofOutcome::Proved(_)
        ));
        let len = world.memo.lock().unwrap().len();
        world.prove_goal(&goal, &facts).unwrap();
        assert_eq!(world.memo.lock().unwrap().len(), len);
        assert!(
            matches!(world.prove(&goal,&facts,&mut vec![key]),Err(e)if e.kind()==FactsErrorKind::Cycle)
        );
        let budget = PlanBudget::default();
        for _ in 0..4096 {
            budget.charge_obligation().unwrap();
        }
        assert!(budget.clone().charge_obligation().is_err());
        let budget = PlanBudget::default();
        let rich = FactsError::invalid(&budget, &"x".repeat(DIAGNOSTIC_LIMIT + 100));
        assert_eq!(rich.kind(), FactsErrorKind::InvalidMetadata);
        let message = rich.into_legacy_diagnostic(&budget);
        assert!(message.len() <= DIAGNOSTIC_LIMIT);
        assert!(message.ends_with("[diagnostic truncated]"));
        let exhausted = FactsError::invalid(
            &budget,
            "this message must exceed the remaining shared diagnostic allowance",
        );
        assert!(exhausted.is_resource());
        let transport = exhausted.into_legacy_diagnostic(&budget);
        assert!(transport.len() < 80);
        assert!(transport.contains("resource limit"));
        let tiny = PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..ir::PlanLimits::default()
        });
        assert!(
            test_support::world(&c.headers.callables, &tiny)
                .unwrap_err()
                .is_resource()
        );
    }

    #[test]
    fn canonical_copy_logical_permission_and_supertrait_closure() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        assert!(facts.must_copy(&Ty::Int(IntTy::I32)).unwrap());
        assert!(!facts.must_copy(&c.caller_parameter).unwrap());
        assert!(!facts.must_copy(&c.leaf).unwrap());
        let shared = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Int(IntTy::I32)),
        };
        let nested = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(shared.clone()),
        };
        assert!(facts.must_copy(&nested).unwrap());
        let mutable = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: Arc::new(Ty::Int(IntTy::I32)),
        };
        assert!(!facts.must_copy(&mutable).unwrap());
        let mut callable = c
            .headers
            .callables
            .iter()
            .find(|h| h.declaration == c.caller)
            .unwrap()
            .clone();
        callable.schema.predicates = Arc::from([Predicate::Implements(TraitRef {
            declaration: world.catalog.id(CatalogItem::Copy),
            self_ty: c.caller_parameter.clone(),
            args: Arc::from([]),
        })]);
        let bounded = test_support::world(&[callable], &c.budget).unwrap();
        let bounded_facts = bounded.callable_facts(c.caller).unwrap();
        assert!(bounded_facts.must_copy(&c.caller_parameter).unwrap());
        for item in [CatalogItem::Clone, CatalogItem::Sized] {
            let predicate = Predicate::Implements(TraitRef {
                declaration: bounded.catalog.id(item),
                self_ty: c.caller_parameter.clone(),
                args: Arc::from([]),
            });
            assert!(matches!(
                bounded.prove_goal(&predicate, &bounded_facts).unwrap(),
                ProofOutcome::Proved(_)
            ));
        }
    }

    #[test]
    fn canonical_catalog_identity_and_provided_dispatch_boundaries() {
        let c = case();
        let world = c.build().unwrap();
        let catalog = &world.catalog;
        assert!(catalog.origin(c.caller).is_err());
        assert!(matches!(
            catalog.origin(catalog.id(CatalogItem::Copy)).unwrap(),
            DeclOrigin::CoreCatalog {
                version: catalog::CATALOG_VERSION,
                item: CatalogItem::Copy
            }
        ));
        let clone = world
            .item_header(catalog.id(CatalogItem::CloneItem))
            .unwrap();
        assert!(clone.default.is_none());
        let provided = world
            .item_header(catalog.id(CatalogItem::CloneFromItem))
            .unwrap();
        let default = provided.default.unwrap();
        assert_eq!(
            catalog.declaration(default).unwrap().callable_provider,
            Some(CallableProvider::UnavailableProvided {
                declaration: default
            })
        );
        assert!(
            matches!(world.callable_facts(default),Err(e) if e.kind()==FactsErrorKind::Unsupported)
        );
        let drop_item = world
            .item_header(catalog.id(CatalogItem::DropItem))
            .unwrap();
        assert!(drop_item.default.is_none());
        assert!(
            catalog
                .declaration(drop_item.declaration)
                .unwrap()
                .callable_provider
                .is_none()
        );
        assert!(
            matches!(catalog.recipe(RecipeDescriptor::CloneNever),Err(e) if e.kind()==FactsErrorKind::Unsupported)
        );
        assert!(test_support::source_decl(c.caller.snapshot, u32::MAX, &c.budget).is_err());
        let mut cyclic = case();
        let mut traits = cyclic.headers.traits.to_vec();
        let left = traits[0].declaration;
        let right = traits[1].declaration;
        traits[0].supertraits = Arc::from([TraitRef {
            declaration: right,
            self_ty: Ty::Param(traits[0].self_param),
            args: Arc::from([]),
        }]);
        traits[1].supertraits = Arc::from([TraitRef {
            declaration: left,
            self_ty: Ty::Param(traits[1].self_param),
            args: Arc::from([]),
        }]);
        cyclic.headers.traits = traits.into();
        assert!(matches!(cyclic.build(),Err(e) if e.kind()==FactsErrorKind::Cycle));
    }

    fn carrier_header(
        budget: &PlanBudget,
        parameters: usize,
        make: impl FnOnce(&[Ty]) -> Signature,
    ) -> CallableHeader {
        let snapshot = SnapshotId::allocate().unwrap();
        let declaration = test_support::source_decl(snapshot, 0, budget).unwrap();
        let own = scope(declaration, parameters, budget);
        let env = env(std::slice::from_ref(&own), budget);
        let parameters: Vec<_> = env.ordered_params().map(Ty::Param).collect();
        CallableHeader {
            declaration,
            schema: GenericSchema {
                signature: make(&parameters),
                env,
                predicates: Arc::from([]),
            },
        }
    }

    #[test]
    fn generic_carrier_world_same_parameter_transport_and_context_authority() {
        let budget = PlanBudget::default();
        let header = carrier_header(&budget, 2, |p| {
            sig(vec![p[0].clone(), p[1].clone()], p[0].clone())
        });
        let world = test_support::world(std::slice::from_ref(&header), &budget).unwrap();
        let universal = world.callable_facts(header.declaration).unwrap();
        let rigid = universal.carrier_contract(SignatureSubject::Body).unwrap();
        assert_eq!(rigid.contract().inputs.len(), 2);
        assert_eq!(rigid.contract().results.len(), 1);
        assert_eq!(rigid.contract().results[0].sources.len(), 1);
        assert_eq!(rigid.contract().results[0].sources[0].param, 0);
        let target = CallTarget {
            declaration: header.declaration,
            type_args: Arc::from([
                reference(Ty::Int(IntTy::I32)),
                reference(Ty::Int(IntTy::I32)),
            ]),
        };
        let facts = world.instance_facts(&target).unwrap();
        let realized = facts.carrier_contract(SignatureSubject::Body).unwrap();
        assert_eq!(realized.contract().inputs.len(), 2);
        assert_eq!(realized.contract().results[0].sources.len(), 1);
        assert_eq!(realized.contract().results[0].sources[0].param, 0);
        assert_eq!(realized.ref_contract().results.len(), 1);
        assert!(realized.validate_for(&universal).is_err());
        let other = test_support::world(std::slice::from_ref(&header), &budget).unwrap();
        assert!(
            realized
                .validate_for(&other.instance_facts(&target).unwrap())
                .is_err()
        );
        assert!(matches!(
            realized.contract().inputs[0].location.channel,
            ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 })
        ));
        // Shape-only facts cannot supply an executable checked-family template.
        assert_eq!(facts.phase(), FactsPhase::Closed);
    }

    #[test]
    fn generic_carrier_world_mutable_entry_effects_and_component_constraints() {
        let budget = PlanBudget::default();
        let header = carrier_header(&budget, 1, |p| {
            reference_signature(
                vec![
                    Ty::Ref {
                        kind: BorrowKind::Mut,
                        referent: Arc::new(p[0].clone()),
                    },
                    p[0].clone(),
                ],
                Ty::Unit,
                &budget,
            )
        });
        let world = test_support::world(std::slice::from_ref(&header), &budget).unwrap();
        let universal = world.callable_facts(header.declaration).unwrap();
        let rigid = universal.carrier_contract(SignatureSubject::Body).unwrap();
        assert_eq!(rigid.contract().writes.len(), 1);
        assert_eq!(rigid.contract().writes[0].destination.param, 0);
        assert!(
            rigid.contract().writes[0]
                .sources
                .iter()
                .any(|s| s.param == 1)
        );
        assert!(rigid.region_schema().constraints.iter().any(|c| matches!(
            c.longer,
            ir::CarrierRegionTerm::Family { .. }
        ) && matches!(
            c.shorter,
            ir::CarrierRegionTerm::Fixed(_)
        )));
        let target = CallTarget {
            declaration: header.declaration,
            type_args: Arc::from([Ty::Ref {
                kind: BorrowKind::Mut,
                referent: Arc::new(Ty::Int(IntTy::I32)),
            }]),
        };
        let facts = world.instance_facts(&target).unwrap();
        let realized = facts.carrier_contract(SignatureSubject::Body).unwrap();
        assert!(realized.contract().results.is_empty());
        assert_eq!(realized.contract().writes.len(), 1);
        assert_eq!(
            realized.contract().writes[0].destination.location.channel,
            ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 })
        );
        assert!(
            realized.contract().writes[0]
                .sources
                .iter()
                .any(|s| s.param == 1
                    && s.location.channel
                        == ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }))
        );
        assert_eq!(realized.component_mappings().len(), 1);
        let parameter = header.schema.env.ordered_params().next().unwrap();
        assert_eq!(realized.parameter_actual(parameter).unwrap().unwrap().0, 0);
        let mutable = |ty| Ty::Ref {
            kind: BorrowKind::Mut,
            referent: Arc::new(ty),
        };
        let shared_barrier = reference(mutable(Ty::Int(IntTy::I32)));
        let blocked_target = CallTarget {
            declaration: header.declaration,
            type_args: Arc::from([shared_barrier.clone()]),
        };
        let blocked = world
            .instance_facts(&blocked_target)
            .unwrap()
            .carrier_contract(SignatureSubject::Body)
            .unwrap();
        assert_eq!(blocked.contract().writes.len(), 1);
        assert_eq!(
            blocked.contract().writes[0].destination.location.channel,
            ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 })
        );
        assert!(
            !blocked
                .contract()
                .writes
                .iter()
                .any(|write| write.destination.location.channel
                    == ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 2 }))
        );
        let original_header = carrier_header(&budget, 0, |_| {
            reference_signature(vec![mutable(shared_barrier.clone())], Ty::Unit, &budget)
        });
        let original_world =
            test_support::world(std::slice::from_ref(&original_header), &budget).unwrap();
        let original = original_world
            .callable_facts(original_header.declaration)
            .unwrap()
            .carrier_contract(SignatureSubject::Body)
            .unwrap();
        assert_eq!(original.contract().writes.len(), 1);
        assert_eq!(
            original.contract().writes[0].destination.location.channel,
            ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 })
        );
        let cell = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([])),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
        };
        assert!(
            !world
                .carrier_writable(&mutable(Ty::Int(IntTy::I32)), &cell)
                .unwrap()
        );
        let field = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([
                ir::CarrierStep::Referent,
                ir::CarrierStep::Field(0),
            ])),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 }),
        };
        assert!(
            !world
                .carrier_writable(&mutable(Ty::Tuple(Arc::from([shared_barrier]))), &field)
                .unwrap()
        );
        assert!(
            world
                .carrier_writable(
                    &mutable(Ty::Tuple(Arc::from([mutable(mutable(Ty::Int(
                        IntTy::I32
                    )))]))),
                    &field
                )
                .unwrap()
        );
        let index = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([
                ir::CarrierStep::Referent,
                ir::CarrierStep::ConstantIndex(0),
            ])),
            channel: field.channel,
        };
        assert!(
            world
                .carrier_writable(
                    &mutable(Ty::Array {
                        element: Arc::new(mutable(mutable(Ty::Int(IntTy::I32)))),
                        len: 1,
                    }),
                    &index
                )
                .unwrap()
        );
    }

    #[test]
    fn generic_carrier_world_original_named_regions_and_resource_authority() {
        struct Regions {
            raw: Arc<dyn SchemaCx + Send + Sync>,
            declaration: DeclId,
            count: u16,
            foreign: bool,
        }
        impl SchemaCx for Regions {
            fn budget(&self) -> &PlanBudget {
                self.raw.budget()
            }
            fn registry_snapshot(&self) -> SnapshotId {
                self.raw.registry_snapshot()
            }
            fn core_catalog(&self) -> &Arc<CoreCatalog> {
                self.raw.core_catalog()
            }
            fn declaration_info(&self, id: DeclId) -> Result<SchemaDecl> {
                self.raw.declaration_info(id)
            }
            fn declaration_schema(&self, id: DeclId) -> Result<GenericSchema> {
                self.raw.declaration_schema(id)
            }
            fn source_region_env(&self, id: DeclId) -> Result<Arc<ir::SourceRegionEnv>> {
                if id != self.declaration {
                    return self.raw.source_region_env(id);
                }
                let declaration = if self.foreign {
                    self.raw.core_catalog().id(CatalogItem::CloneItem)
                } else {
                    id
                };
                Ok(Arc::new(ir::SourceRegionEnv {
                    declaration,
                    region_count: self.count,
                    normalized: vec![None; usize::from(self.count)].into(),
                    outlives: Arc::from([]),
                }))
            }
            fn instantiate_schema(
                &self,
                s: &GenericSchema,
                a: &[Ty],
                e: &GenericEnv,
                b: &PlanBudget,
            ) -> Result<InstantiatedSchema> {
                self.raw.instantiate_schema(s, a, e, b)
            }
            fn validate_nominal(&self, t: &Ty, e: &GenericEnv, b: &PlanBudget) -> Result<()> {
                self.raw.validate_nominal(t, e, b)
            }
        }
        let budget = PlanBudget::default();
        let header = carrier_header(&budget, 1, |p| sig(vec![p[0].clone()], p[0].clone()));
        let headers = HeaderSet {
            traits: Arc::from([]),
            items: Arc::from([]),
            impls: Arc::from([]),
            callables: Arc::from([header.clone()]),
        };
        let raw = test_support::adapter(&headers, &[], &budget).unwrap();
        let world = ValidatedTraitWorld::build(
            headers.clone(),
            Arc::new(Regions {
                raw: raw.clone(),
                declaration: header.declaration,
                count: 1,
                foreign: false,
            }),
            &budget,
        )
        .unwrap();
        let facts = world.callable_facts(header.declaration).unwrap();
        let original = facts.source_region_env(header.declaration).unwrap();
        assert_eq!(original.region_count, 1);
        assert_eq!(original.normalized.as_ref(), &[None]);
        assert!(
            matches!(ValidatedTraitWorld::build(headers.clone(),Arc::new(Regions{raw:raw.clone(),declaration:header.declaration,count:1,foreign:true}),&budget),Err(e)if e.kind()==FactsErrorKind::InvalidMetadata)
        );
        assert!(
            matches!(ValidatedTraitWorld::build(headers,Arc::new(Regions{raw,declaration:header.declaration,count:65,foreign:false}),&budget),Err(e)if e.is_resource())
        );
        budget.charge_work(budget.limits().work).unwrap_err();
        // Use up only the remaining work, without a fresh ledger or permission.
        let mut quantum = budget.limits().work / 2;
        while quantum != 0 {
            while budget.charge_work(quantum).is_ok() {}
            quantum /= 2;
        }
        assert!(matches!(facts.carrier_contract(SignatureSubject::Body),Err(e)if e.is_resource()));
    }

    #[test]
    fn canonical_required_self_topology_is_context_bound_and_not_logical_dispatch() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let shared = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(c.caller_parameter.clone()),
        };
        let call = TraitCall {
            trait_ref: TraitRef {
                declaration: world.catalog.id(CatalogItem::Clone),
                self_ty: shared,
                args: Arc::from([]),
            },
            item: world.catalog.id(CatalogItem::CloneItem),
            method_args: Arc::from([]),
        };
        let resolved = world.resolve_trait_call(&call, &facts).unwrap();
        assert!(matches!(resolved.callee(), Callee::TraitItem(_)));
        let proof = resolved._topology.as_ref().unwrap();
        assert_eq!(proof.environment, facts.predicates.index);
        assert_eq!(proof.phase, FactsPhase::Universal);
        assert_eq!(proof.realized.input_regions.len(), 2);
        let result = &proof.realized.results[0];
        let ir::ReturnContract::SharedInputEnvelopes(envelopes) = &result.returns else {
            panic!("latent carrier must retain an input envelope")
        };
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].input.position.depth, 1);
        let wrong = ProofContext {
            predicates: &world.empty,
            phase: FactsPhase::Closed,
        };
        assert!(world.validate_topology(proof, wrong).is_err());
        let concrete = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(Ty::Int(IntTy::I32)),
            }),
        };
        assert!(facts.must_copy(&concrete).unwrap());
        let too_deep = TraitCall {
            trait_ref: TraitRef {
                declaration: world.catalog.id(CatalogItem::Clone),
                self_ty: concrete,
                args: Arc::from([]),
            },
            ..call
        };
        let deeper = world
            .instantiate_trait_signature(&too_deep, &facts)
            .unwrap();
        assert_eq!(deeper.ref_contract.input_regions.len(), 3);
        assert_eq!(deeper.ref_contract.results.len(), 2);
        assert_eq!(ir::root_reference_layers(&deeper.params[0]).unwrap(), 3);
        // A declaration topology signature is not a checked recipe/body proof.
        assert_eq!(facts.phase(), FactsPhase::Universal);
    }
}
