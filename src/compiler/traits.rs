//! A single validated trait world, obligation engine and fallible facts authority.
//!
//! Raw schema callbacks are structural only. Neither raw schemas nor a caller's
//! claimed successful analysis can manufacture body facts or dispatch evidence.
use super::ir::{
    self, CallTarget, Callee, DeclId, DeclKind, GenericEnv, ParamEnv, ParamId, PlanBudget,
    Predicate, Projection, ReceiverMode, Signature, SnapshotId, TraitCall, TraitCx, TraitRef, Ty,
};
use std::sync::{Arc, Mutex};

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
    retain::<T>(b, 1)?;
    if values.len() == values.capacity() {
        charged(b, b.charge_work(values.len()))?;
    }
    values
        .try_reserve_exact(1)
        .map_err(|_| FactsError::resource(b, "trait metadata allocation failed"))?;
    values.push(value);
    Ok(())
}

#[derive(Clone, Debug)]
pub struct GenericSchema {
    pub env: Arc<GenericEnv>,
    pub signature: Signature,
    pub predicates: Arc<[Predicate]>,
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
}
#[derive(Clone, Debug)]
pub struct TraitItemHeader {
    pub declaration: DeclId,
    pub trait_decl: DeclId,
    pub receiver: ReceiverMode,
    pub schema: GenericSchema,
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
}
#[derive(Clone, Debug)]
pub struct InstantiatedSchema {
    pub signature: Signature,
    pub predicates: Arc<[Predicate]>,
}
pub trait SchemaCx {
    fn budget(&self) -> &PlanBudget;
    fn registry_snapshot(&self) -> SnapshotId;
    fn declaration_info(&self, id: DeclId) -> Result<SchemaDecl>;
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
    pub fn validate_ty(&self, ty: &Ty) -> Result<()> {
        self.validate_ty_with_budget(ty, self.budget())
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
        intrinsic_copy(ty, self.budget(), 0)
    }
    pub fn projected_ty<'a>(&self, root: &'a Ty, path: &[Projection]) -> Result<&'a Ty> {
        self.validate_ty(root)?;
        charged(self.budget(), self.budget().charge_work(path.len()))?;
        charged(self.budget(), ir::projected_ty(root, path))
    }
}
fn intrinsic_copy(ty: &Ty, b: &PlanBudget, depth: usize) -> Result<bool> {
    charged(b, b.check_depth(depth))?;
    charged(b, b.charge_work(1))?;
    match ty {
        Ty::Param(_) | Ty::Struct(_) | Ty::Enum(_) => Ok(false),
        Ty::Ref { kind, .. } => Ok(*kind == ir::BorrowKind::Shared),
        Ty::Tuple(parts) => {
            for t in parts.iter() {
                if !intrinsic_copy(t, b, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Ty::Array { element, .. } => intrinsic_copy(element, b, depth + 1),
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Never => Ok(true),
    }
}
#[derive(Clone, Debug)]
enum ProofOrigin {
    Assumption,
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
    complete: Vec<(TypeNode, usize)>,
    reservations: Vec<ir::PlanReservation>,
    visits: usize,
    maybe_unsized: Option<ParamId>,
    root_top: bool,
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
    fn lookup(&self, key: TypeNode, budget: &PlanBudget) -> Result<Option<usize>> {
        for (old, height) in &self.complete {
            charged(budget, budget.charge_work(1))?;
            if *old == key {
                return Ok(Some(*height));
            }
        }
        Ok(None)
    }
    fn insert(&mut self, key: TypeNode, height: usize, budget: &PlanBudget) -> Result<()> {
        let reservation = charged(
            budget,
            budget.reserve_bytes(
                2 * std::mem::size_of::<(TypeNode, usize)>()
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
    budget: PlanBudget,
    headers: HeaderSet,
    environments: Vec<Arc<PredicateEnv>>,
    empty: Arc<PredicateEnv>,
    memo: Mutex<Vec<MemoEntry>>,
    requirements: Vec<(DeclId, Arc<[Predicate]>)>,
}

impl ValidatedTraitWorld {
    pub fn build(
        headers: HeaderSet,
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
            budget: budget.clone(),
            headers,
            environments: Vec::new(),
            empty,
            memo: Mutex::new(Vec::new()),
            requirements: Vec::new(),
        };
        world.validate_headers()?;
        world.build_environments()?;
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
        self.validate_ty(&goal.self_ty, env, maybe_unsized, true, 0)
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
            root_top: top,
            ..TypeWalk::default()
        };
        self.validate_ty_inner(ty, env, depth, &mut walk)
            .map(|_| ())
    }
    fn validate_ty_inner(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
        walk: &mut TypeWalk,
    ) -> Result<usize> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        walk.visits = walk
            .visits
            .checked_add(1)
            .ok_or_else(|| FactsError::resource(&self.budget, "type walk visit count overflow"))?;
        let key = TypeWalk::key(ty);
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
        let top = depth == 0 && walk.root_top;
        let mut height = 0usize;
        let mut child = |t: &Ty| -> Result<()> {
            height = height.max(
                self.validate_ty_inner(t, env, depth + 1, walk)?
                    .saturating_add(1),
            );
            Ok(())
        };
        match ty {
            Ty::Param(param) => {
                charged(&self.budget, self.budget.charge_work(env.scopes().len()))?;
                if !env.contains(*param) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "type parameter outside authoritative environment",
                    ));
                }
                if Some(*param) == maybe_unsized && !top {
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
                    child(t)?;
                }
            }
            Ty::Array { element, len } => {
                if *len > ir::MAX_ARRAY_LEN {
                    return Err(FactsError::resource(
                        &self.budget,
                        "array length limit exceeded",
                    ));
                }
                child(element)?;
            }
            Ty::Ref { referent, .. } => {
                if maybe_unsized.is_some() || env.is_generic() {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: references in trait schemas or generic bodies",
                    ));
                }
                child(referent)?;
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
                    child(t)?;
                }
                for field in t.def().fields.iter() {
                    child(&field.ty)?;
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
                    child(t)?;
                }
                for variant in t.def().variants.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    for field in variant.fields.iter() {
                        child(&field.ty)?;
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
    fn validate_signature(
        &self,
        signature: &Signature,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
    ) -> Result<()> {
        // Prewalk and meter before the existing reference-contract validator can
        // construct its bounded structural scratch data.
        let mut walk = TypeWalk {
            maybe_unsized,
            root_top: true,
            ..TypeWalk::default()
        };
        for t in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            self.validate_ty_inner(t, env, 0, &mut walk)?;
        }
        let c = &signature.ref_contract;
        charged(
            &self.budget,
            self.budget.charge_work(
                c.param_regions
                    .len()
                    .saturating_add(c.outlives.len())
                    .saturating_add(usize::from(c.region_count).saturating_pow(2))
                    .saturating_add(1),
            ),
        )?;
        let scratch = walk
            .visits
            .checked_mul(192)
            .and_then(|s| s.checked_add(4096))
            .ok_or_else(|| FactsError::resource(&self.budget, "signature scratch size overflow"))?;
        let scratch = if matches!(signature.result, Ty::Ref { .. }) {
            scratch
                .saturating_add(
                    ir::MAX_INPUT_PATH_STEPS.saturating_mul(std::mem::size_of::<Projection>()),
                )
                .saturating_add(
                    ir::MAX_INPUT_PATHS.saturating_mul(std::mem::size_of::<ir::InputPath>()),
                )
        } else {
            scratch
        };
        let _scratch = charged(&self.budget, self.budget.reserve_bytes(scratch))?;
        charged(
            &self.budget,
            c.validate(&signature.params, &signature.result),
        )
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
        let ca = &a.ref_contract;
        let cb = &b.ref_contract;
        let size = ca
            .param_regions
            .len()
            .saturating_add(cb.param_regions.len())
            .saturating_add(ca.outlives.len())
            .saturating_add(cb.outlives.len());
        charged(&self.budget, self.budget.charge_work(size))?;
        for contract in [ca, cb] {
            if let ir::ReturnContract::SharedInputPaths(paths)
            | ir::ReturnContract::MutInputPaths(paths) = &contract.returns
            {
                for path in paths.iter() {
                    charged(
                        &self.budget,
                        self.budget
                            .charge_work(path.projection.len().saturating_add(1)),
                    )?;
                }
            }
        }
        Ok(ca == cb)
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
            self.validate_signature(
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
            self.validate_signature(&callable.schema.signature, &callable.schema.env, None)?;
            self.validate_predicates(&callable.schema.predicates, &callable.schema.env, None)?;
        }
        for implementation in self.headers.impls.iter() {
            self.validate_impl(implementation)?;
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
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: nonnominal trait implementation head",
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
        if h.items.len() != trait_header.items.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "trait impl has missing or extra required items",
            ));
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
            let expected = self.schema.instantiate_schema(
                &item.schema,
                std::slice::from_ref(&h.trait_ref.self_ty),
                &h.schema_env,
                &self.budget,
            )?;
            self.validate_signature(&expected.signature, &h.schema_env, None)?;
            self.validate_predicates(&expected.predicates, &h.schema_env, None)?;
            if !self.eq_signature(&expected.signature, &target.schema.signature)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait implementation signature does not match required declaration",
                ));
            }
            for predicate in target.schema.predicates.iter() {
                let mut allowed = false;
                for candidate in h.predicates.iter().chain(expected.predicates.iter()) {
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
    fn build_environments(&mut self) -> Result<()> {
        // Header schemas are now globally validated. These environments remain
        // inaccessible until the immutable world is successfully returned.
        for item in self.headers.items.iter() {
            let mut predicates = Vec::new();
            for p in item.schema.predicates.iter() {
                self.append_predicate(&mut predicates, p)?;
            }
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
    fn instantiate(
        &self,
        schema: &GenericSchema,
        args: &[Ty],
        facts: &ValidatedFacts,
    ) -> Result<InstantiatedSchema> {
        if args.len() != schema.env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "generic argument arity mismatch",
            ));
        }
        for arg in args {
            facts.validate_ty(arg)?;
            charged(&self.budget, ir::reject_references(arg, &self.budget))?;
        }
        let result = self
            .schema
            .instantiate_schema(schema, args, facts.env(), &self.budget)?;
        self.validate_signature(&result.signature, facts.env(), None)?;
        self.validate_predicates(&result.predicates, facts.env(), None)?;
        Ok(result)
    }
    fn require(
        &self,
        predicate: &Predicate,
        facts: &ValidatedFacts,
        active: &mut Vec<GoalKey>,
    ) -> Result<ProofEvidence> {
        match self.prove(predicate, facts, active)? {
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
        charged(&self.budget, self.budget.check_depth(active.len()))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let Predicate::Implements(goal) = predicate;
        self.validate_ref_in(goal, facts.env(), None)?;
        let key = GoalKey {
            environment: facts.predicates.index,
            phase: facts.phase(),
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
        facts: &ValidatedFacts,
        origin: ProofOrigin,
    ) -> Result<ProofOutcome> {
        retain::<ProofRecord>(&self.budget, 1)?;
        Ok(ProofOutcome::Proved(ProofEvidence(Arc::new(ProofRecord {
            world: self.id,
            environment: facts.predicates.index,
            phase: facts.phase(),
            goal: predicate.clone(),
            origin,
        }))))
    }
    fn prove_uncached(
        &self,
        predicate: &Predicate,
        facts: &ValidatedFacts,
        active: &mut Vec<GoalKey>,
    ) -> Result<ProofOutcome> {
        for assumption in facts.predicates.predicates.iter() {
            if self.eq_predicate(assumption, predicate)? {
                return self.evidence(predicate, facts, ProofOrigin::Assumption);
            }
        }
        let Predicate::Implements(goal) = predicate;
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
            let instantiated = self.instantiate(&schema, &args, facts)?;
            if !self.eq_ty(&instantiated.signature.result, &goal.self_ty, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "impl head substitution disagrees with matched goal",
                ));
            }
            let mut satisfied = true;
            for required in instantiated.predicates.iter() {
                match self.prove(required, facts, active)? {
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
        self.validate_trait_ref(&call.trait_ref, facts)?;
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
        self.require(
            &Predicate::Implements(call.trait_ref.clone()),
            facts,
            &mut Vec::new(),
        )?;
        let instantiated = self.instantiate(
            &item.schema,
            std::slice::from_ref(&call.trait_ref.self_ty),
            facts,
        )?;
        for predicate in instantiated.predicates.iter() {
            self.require(predicate, facts, &mut Vec::new())?;
        }
        Ok(instantiated.signature)
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
        let schema = self.callable_schema(target.declaration)?;
        let instantiated = self.instantiate(&schema, &target.type_args, facts)?;
        let mut evidence = Vec::new();
        for predicate in instantiated.predicates.iter() {
            push(
                &self.budget,
                &mut evidence,
                self.require(predicate, facts, &mut Vec::new())?,
            )?;
        }
        Ok(DirectCallProof {
            target: target.clone(),
            signature: instantiated.signature,
            evidence: evidence.into(),
        })
    }
    fn resolve_trait_call(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<ResolvedTraitCall> {
        let signature = self.instantiate_trait_signature(call, facts)?;
        let predicate = Predicate::Implements(call.trait_ref.clone());
        let evidence = self.require(&predicate, facts, &mut Vec::new())?;
        if facts.phase() == FactsPhase::Universal {
            return Ok(ResolvedTraitCall {
                callee: Callee::TraitItem(call.clone()),
                signature,
                _evidence: evidence,
            });
        }
        let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &evidence.0.origin
        else {
            return Err(FactsError::invalid(
                &self.budget,
                "closed dispatch cannot use caller assumption evidence",
            ));
        };
        if evidence.0.world != self.id
            || evidence.0.environment != facts.predicates.index
            || evidence.0.phase != facts.phase()
            || !self.eq_predicate(&evidence.0.goal, &predicate)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "dispatch evidence does not prove the exact goal",
            ));
        }
        let mut target = None;
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
                break;
            }
        }
        let target = target.ok_or_else(|| {
            FactsError::invalid(
                &self.budget,
                "selected implementation has no required item mapping",
            )
        })?;
        let direct = self.discharge_direct(&target, facts)?;
        if !self.eq_signature(&signature, &direct.signature)? {
            return Err(FactsError::invalid(
                &self.budget,
                "resolved implementation signature disagrees with trait declaration",
            ));
        }
        Ok(ResolvedTraitCall {
            callee: Callee::Direct(direct.target),
            signature,
            _evidence: evidence,
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
    use super::*;
    use ir::{BinderId, FactsCtx, NominalAppId, NominalAppInfo, SubstitutionCx};

    pub(super) struct FixtureSchema {
        registry: SnapshotId,
        budget: PlanBudget,
        declarations: Vec<SchemaDecl>,
        nominals: Vec<Ty>,
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
            nominals: Vec::new(),
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
        retain::<FixtureSchema>(budget, 1)?;
        Ok(Arc::new(schema))
    }
    impl SchemaCx for FixtureSchema {
        fn budget(&self) -> &PlanBudget {
            &self.budget
        }
        fn registry_snapshot(&self) -> SnapshotId {
            self.registry
        }
        fn declaration_info(&self, id: DeclId) -> Result<SchemaDecl> {
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
            let mut params = Vec::new();
            for p in &schema.signature.params {
                push(
                    budget,
                    &mut params,
                    charged(
                        budget,
                        ir::substitute_ty(p, &schema.env, args, caller, &mut cx, budget),
                    )?,
                )?;
            }
            let result = charged(
                budget,
                ir::substitute_ty(
                    &schema.signature.result,
                    &schema.env,
                    args,
                    caller,
                    &mut cx,
                    budget,
                ),
            )?;
            let mut predicates = Vec::new();
            for Predicate::Implements(goal) in schema.predicates.iter() {
                let self_ty = charged(
                    budget,
                    ir::substitute_ty(&goal.self_ty, &schema.env, args, caller, &mut cx, budget),
                )?;
                let mut trait_args = Vec::new();
                for a in goal.args.iter() {
                    push(
                        budget,
                        &mut trait_args,
                        charged(
                            budget,
                            ir::substitute_ty(a, &schema.env, args, caller, &mut cx, budget),
                        )?,
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
            nominals: Vec::new(),
        };
        for ty in types
            .iter()
            .chain(signature.params.iter())
            .chain(std::iter::once(&signature.result))
        {
            raw.gather(ty, 0)?;
        }
        let mut index = 0u32;
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
        let headers = HeaderSet {
            callables: Arc::from([callable]),
            ..empty_headers
        };
        let world = ValidatedTraitWorld::build(headers, Arc::new(raw), budget)?;
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
    use ir::{BinderId, FieldDef, IntTy, NominalAppId, RefContract, StructDef, StructTy};

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
            ref_contract: Arc::new(RefContract {
                param_regions: vec![None; params.len()].into(),
                ..RefContract::default()
            }),
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
        let id = |index| DeclId {
            snapshot: registry,
            index,
        };
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
                },
                TraitHeader {
                    declaration: id(2),
                    self_param: ParamId {
                        owner: value_self.binder(),
                        index: 0,
                    },
                    env: value_env,
                    items: Arc::from([id(3)]),
                },
            ]),
            items: Arc::from([TraitItemHeader {
                declaration: id(3),
                trait_decl: id(2),
                receiver: ReceiverMode::Value,
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
            budget,
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
        overlap.declaration.index = 11;
        implementations.push(overlap);
        c.headers.impls = implementations.into();
        assert!(matches!(c.build(),Err(e)if e.kind()==FactsErrorKind::Ambiguous));
        struct ErasedScope {
            raw: Arc<dyn SchemaCx + Send + Sync>,
            declaration: DeclId,
        }
        impl SchemaCx for ErasedScope {
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
}
