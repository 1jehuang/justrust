//! Checked scalar, aggregate and nominal bodies shared by analysis and codegen.
//!
//! IDs are local to one immutable function, not persistent cache identities.
//! Allocate a new LocalId for every binding, including shadowing. Names are for
//! diagnostics only. This structured representation is a migration boundary,
//! not yet an ownership-checking CFG or a target layout/ABI representation.
use super::frontend::{BinaryOp, UnaryOp};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
/// Combined explicit loan sites and formal reference-parameter origins.
pub const MAX_LOAN_SITES: usize = 4096;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local namespace for one validated declaration environment. IDs must
/// not be persisted as cache identities or reused by another environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SnapshotId(u64);

impl SnapshotId {
    pub fn allocate() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self::allocate_from(&NEXT)
    }

    fn allocate_from(next: &AtomicU64) -> Result<Self, String> {
        let mut id = next.load(Ordering::Relaxed);
        loop {
            let successor = id
                .checked_add(1)
                .ok_or("nominal type snapshot IDs exhausted")?;
            match next.compare_exchange_weak(id, successor, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Ok(Self(id)),
                Err(observed) => id = observed,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NominalAppId {
    pub snapshot: SnapshotId,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeclId {
    pub snapshot: SnapshotId,
    pub index: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeclKind {
    Function,
    Struct,
    Enum,
    Impl,
    AssociatedFunction,
    Trait,
    TraitItem,
    TraitImpl,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BinderId {
    pub declaration: DeclId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParamId {
    pub owner: BinderId,
    pub index: u16,
}
#[derive(Clone, Debug)]
pub struct ParamEnv {
    binder: BinderId,
    arity: usize,
}
impl ParamEnv {
    pub(super) fn new(binder: BinderId, arity: usize) -> Result<Self, String> {
        if arity > 32 {
            return Err("generic parameter limit exceeded".into());
        }
        Ok(Self { binder, arity })
    }
    pub fn binder(&self) -> BinderId {
        self.binder
    }
    pub fn arity(&self) -> usize {
        self.arity
    }
    pub fn contains(&self, param: ParamId) -> bool {
        param.owner == self.binder && usize::from(param.index) < self.arity
    }
}
/// Authoritative ordered binder scopes visible in a callable body.
#[derive(Clone, Debug)]
pub struct GenericEnv {
    scopes: Arc<[Arc<ParamEnv>]>,
    total_arity: usize,
}
impl GenericEnv {
    pub(super) fn new(scopes: &[Arc<ParamEnv>], budget: &PlanBudget) -> Result<Self, String> {
        if scopes.len() > 32 {
            return Err("generic scope limit exceeded".into());
        }
        budget.charge_work(scopes.len().saturating_mul(scopes.len() + 1))?;
        let mut total_arity = 0usize;
        for (index, scope) in scopes.iter().enumerate() {
            if scopes[..index]
                .iter()
                .any(|prior| prior.binder() == scope.binder())
            {
                return Err("duplicate generic binder scope".into());
            }
            total_arity = total_arity
                .checked_add(scope.arity())
                .ok_or("generic arity overflow")?;
            if total_arity > budget.limits().params {
                return Err("generic parameter limit exceeded".into());
            }
        }
        budget.retain_bytes(std::mem::size_of::<Self>() + std::mem::size_of_val(scopes))?;
        Ok(Self {
            scopes: scopes.into(),
            total_arity,
        })
    }
    pub fn scopes(&self) -> &[Arc<ParamEnv>] {
        &self.scopes
    }
    pub fn contains(&self, param: ParamId) -> bool {
        self.scopes.iter().any(|scope| scope.contains(param))
    }
    pub fn total_arity(&self) -> usize {
        self.total_arity
    }
    pub fn is_generic(&self) -> bool {
        self.total_arity != 0
    }
    pub fn ordered_params(&self) -> impl Iterator<Item = ParamId> + '_ {
        self.scopes.iter().flat_map(|scope| {
            (0..scope.arity()).map(move |index| ParamId {
                owner: scope.binder(),
                index: index as u16,
            })
        })
    }
    fn argument_index(&self, param: ParamId) -> Option<usize> {
        let mut offset = 0;
        for scope in self.scopes.iter() {
            if scope.contains(param) {
                return Some(offset + usize::from(param.index));
            }
            offset += scope.arity();
        }
        None
    }
}
#[derive(Clone, Copy, Debug)]
pub enum FactsCtx<'a> {
    Concrete,
    Universal(&'a GenericEnv),
    Validated(&'a super::traits::ValidatedFacts),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallTarget {
    pub declaration: DeclId,
    pub type_args: Arc<[Ty]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReceiverMode {
    None,
    Value,
    Borrowed { kind: BorrowKind },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImplicitBorrowKind {
    MutableReceiver,
    MutableArgumentReborrow { carrier: ExprId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BorrowTiming {
    #[default]
    Immediate,
    TwoPhase {
        activation: ExprId,
        argument: usize,
        reason: ImplicitBorrowKind,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum SignatureSubject<'a> {
    Body,
    Call(&'a Callee),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TraitRef {
    pub declaration: DeclId,
    pub self_ty: Ty,
    pub args: Arc<[Ty]>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Predicate {
    Implements(TraitRef),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TraitCall {
    pub trait_ref: TraitRef,
    pub item: DeclId,
    pub method_args: Arc<[Ty]>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Callee {
    Direct(CallTarget),
    TraitItem(TraitCall),
}
impl From<CallTarget> for Callee {
    fn from(target: CallTarget) -> Self {
        Self::Direct(target)
    }
}
impl Callee {
    pub fn type_arguments(&self) -> &[Ty] {
        match self {
            Self::Direct(target) => &target.type_args,
            Self::TraitItem(call) => &call.method_args,
        }
    }
    pub fn metadata_types(&self) -> impl Iterator<Item = &Ty> {
        let (self_ty, trait_args): (Option<&Ty>, &[Ty]) = match self {
            Self::Direct(_) => (None, &[]),
            Self::TraitItem(call) => (Some(&call.trait_ref.self_ty), &call.trait_ref.args),
        };
        self.type_arguments()
            .iter()
            .chain(self_ty)
            .chain(trait_args.iter())
    }
}
pub fn validate_callee(
    target: &Callee,
    signature: &Signature,
    facts: FactsCtx<'_>,
) -> Result<(), String> {
    if target.type_arguments().len() > 32 {
        return Err("generic call argument limit exceeded".into());
    }
    if let Callee::TraitItem(call) = target
        && call.trait_ref.args.len() > 32
    {
        return Err("trait argument limit exceeded".into());
    }
    for ty in target.metadata_types() {
        facts.validate_ty(ty)?;
    }
    if let Some(authority) = facts.validated() {
        if authority.phase() == super::traits::FactsPhase::Closed
            && matches!(target, Callee::TraitItem(_))
        {
            return Err("unresolved trait call in closed body".into());
        }
        authority
            .service()
            .validate_signature(SignatureSubject::Call(target), signature, authority)
            .map_err(|e| e.into_legacy_diagnostic(authority.budget()))?;
    } else if matches!(target, Callee::TraitItem(_)) {
        return Err("trait call requires validated facts".into());
    }
    Ok(())
}
pub fn validate_body_signature(
    signature: &Signature,
    facts: &super::traits::ValidatedFacts,
) -> Result<(), String> {
    if facts.phase() == super::traits::FactsPhase::Schema {
        return Err("trait schema cannot authorize executable body".into());
    }
    facts
        .service()
        .validate_signature(SignatureSubject::Body, signature, facts)
        .map_err(|e| e.into_legacy_diagnostic(facts.budget()))
}
pub trait TraitCx: Send + Sync {
    fn validate_signature(
        &self,
        subject: SignatureSubject<'_>,
        signature: &Signature,
        facts: &super::traits::ValidatedFacts,
    ) -> Result<(), super::traits::FactsError>;
    fn callee_receiver(
        &self,
        callee: &Callee,
        facts: &super::traits::ValidatedFacts,
    ) -> Result<ReceiverMode, super::traits::FactsError>;
    fn validate_trait_ref(
        &self,
        goal: &TraitRef,
        facts: &super::traits::ValidatedFacts,
    ) -> Result<(), super::traits::FactsError>;
    fn instantiate_trait_signature(
        &self,
        call: &TraitCall,
        facts: &super::traits::ValidatedFacts,
    ) -> Result<Signature, super::traits::FactsError>;
    fn prove_goal(
        &self,
        goal: &Predicate,
        facts: &super::traits::ValidatedFacts,
    ) -> Result<super::traits::ProofOutcome, super::traits::FactsError>;
    fn discharge_direct(
        &self,
        target: &CallTarget,
        facts: &super::traits::ValidatedFacts,
    ) -> Result<super::traits::DirectCallProof, super::traits::FactsError>;
    fn resolve_trait_call(
        &self,
        call: &TraitCall,
        facts: &super::traits::ValidatedFacts,
    ) -> Result<super::traits::ResolvedTraitCall, super::traits::FactsError>;
}
#[derive(Clone, Debug)]
pub struct GenericSignature {
    pub env: Arc<GenericEnv>,
    pub signature: Signature,
    pub predicates: Arc<super::traits::PredicateEnv>,
}
#[derive(Clone, Debug)]
pub struct NominalAppInfo {
    pub source_decl: DeclId,
    pub type_args: Arc<[Ty]>,
}
#[derive(Clone, Copy, Debug)]
pub struct PlanLimits {
    pub params: usize,
    pub applications: usize,
    pub depth: usize,
    pub nodes: usize,
    pub work: usize,
    pub bytes: usize,
}
impl Default for PlanLimits {
    fn default() -> Self {
        Self {
            params: 32,
            applications: 4096,
            depth: 128,
            nodes: 1_000_000,
            work: 32_000_000,
            bytes: 128 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Default)]
struct PlanUsage {
    applications: usize,
    nodes: usize,
    work: usize,
    bytes: usize,
    obligations: usize,
    diagnostic_bytes: usize,
}
#[derive(Clone, Debug)]
pub struct PlanBudget {
    limits: PlanLimits,
    usage: Arc<std::sync::Mutex<PlanUsage>>,
}
#[derive(Debug)]
pub struct PlanReservation {
    usage: Arc<std::sync::Mutex<PlanUsage>>,
    bytes: usize,
}
impl Drop for PlanReservation {
    fn drop(&mut self) {
        if let Ok(mut usage) = self.usage.lock() {
            usage.bytes -= self.bytes;
        }
    }
}
impl Default for PlanBudget {
    fn default() -> Self {
        Self::new(PlanLimits::default())
    }
}
impl PlanBudget {
    pub fn new(limits: PlanLimits) -> Self {
        let defaults = PlanLimits::default();
        let limits = PlanLimits {
            params: limits.params.min(defaults.params),
            applications: limits.applications.min(defaults.applications),
            depth: limits.depth.min(defaults.depth),
            nodes: limits.nodes.min(defaults.nodes),
            work: limits.work.min(defaults.work),
            bytes: limits.bytes.min(defaults.bytes),
        };
        Self {
            limits,
            usage: Arc::new(std::sync::Mutex::new(PlanUsage::default())),
        }
    }
    pub fn limits(&self) -> PlanLimits {
        self.limits
    }
    pub fn same_authority(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.usage, &other.usage)
    }
    fn charge(
        &self,
        amount: usize,
        select: fn(&mut PlanUsage) -> &mut usize,
        limit: usize,
    ) -> Result<(), String> {
        let mut usage = self
            .usage
            .lock()
            .map_err(|_| "generic planning budget poisoned")?;
        let counter = select(&mut usage);
        let next = counter
            .checked_add(amount)
            .ok_or("generic planning resource limit exceeded")?;
        if next > limit {
            return Err("generic planning resource limit exceeded".into());
        }
        *counter = next;
        Ok(())
    }
    pub fn charge_work(&self, amount: usize) -> Result<(), String> {
        self.charge(amount, |u| &mut u.work, self.limits.work)
    }
    pub fn charge_nodes(&self, amount: usize) -> Result<(), String> {
        self.charge(amount, |u| &mut u.nodes, self.limits.nodes)
    }
    pub fn charge_application(&self) -> Result<(), String> {
        self.charge(1, |u| &mut u.applications, self.limits.applications)
    }
    pub fn charge_obligation(&self) -> Result<(), String> {
        self.charge(1, |u| &mut u.obligations, 4096)
    }
    pub fn charge_diagnostic_bytes(&self, bytes: usize) -> Result<(), String> {
        self.charge(bytes, |u| &mut u.diagnostic_bytes, 64 * 1024)
    }
    pub fn reserve_bytes(&self, bytes: usize) -> Result<PlanReservation, String> {
        self.charge(bytes, |u| &mut u.bytes, self.limits.bytes)?;
        Ok(PlanReservation {
            usage: self.usage.clone(),
            bytes,
        })
    }
    /// Conservatively retain generated semantic storage until this plan ends.
    pub fn retain_bytes(&self, bytes: usize) -> Result<(), String> {
        self.charge(bytes, |u| &mut u.bytes, self.limits.bytes)
    }
    pub fn check_depth(&self, depth: usize) -> Result<(), String> {
        if depth > self.limits.depth {
            Err("generic planning depth limit exceeded".into())
        } else {
            Ok(())
        }
    }
}
pub trait SubstitutionCx {
    fn nominal_application(&self, id: NominalAppId) -> Result<NominalAppInfo, String>;
    fn intern_nominal(
        &mut self,
        declaration: DeclId,
        args: &[Ty],
        caller: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Ty, String>;
    fn instantiate_signature(
        &mut self,
        target: &CallTarget,
        caller: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Signature, String>;
}

pub fn substitute_ty(
    ty: &Ty,
    env: &GenericEnv,
    args: &[Ty],
    caller: FactsCtx<'_>,
    cx: &mut impl SubstitutionCx,
    budget: &PlanBudget,
) -> Result<Ty, String> {
    if args.len() != env.total_arity() {
        return Err("generic argument arity mismatch".into());
    }
    for argument in args {
        caller.validate_ty_with_budget(argument, budget)?;
        reject_references(argument, budget)?;
    }
    substitute_ty_walk(ty, env, args, caller, cx, budget, 0)
}

/// Unsealed structural substitution for nonexecuting trait-item schemas only.
/// The world must certify topology before publishing the resulting signature.
#[allow(clippy::too_many_arguments)]
pub(super) fn substitute_schema_ty(
    ty: &Ty,
    env: &GenericEnv,
    args: &[Ty],
    caller: FactsCtx<'_>,
    cx: &mut impl SubstitutionCx,
    registry: &dyn super::traits::SchemaCx,
    budget: &PlanBudget,
) -> Result<Ty, String> {
    if !registry.budget().same_authority(budget) || args.len() != env.total_arity() {
        return Err("schema substitution authority or arity mismatch".into());
    }
    budget.charge_work(env.scopes().len().saturating_mul(env.scopes().len() + 1))?;
    let mut item_owner = None;
    for scope in env.scopes() {
        let declaration = registry
            .declaration_info(scope.binder().declaration)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        if declaration.kind == DeclKind::TraitItem {
            if item_owner.is_some() {
                return Err("ambiguous trait-item schema environment".into());
            }
            let raw = registry
                .declaration_schema(declaration.declaration)
                .map_err(|error| error.into_legacy_diagnostic(budget))?;
            if raw.env.scopes().len() != env.scopes().len()
                || !raw
                    .env
                    .scopes()
                    .iter()
                    .zip(env.scopes())
                    .all(|(left, right)| {
                        left.binder() == right.binder() && left.arity() == right.arity()
                    })
            {
                return Err("trait-item schema environment mismatch".into());
            }
            item_owner = declaration.owner;
        }
    }
    let trait_owner = item_owner.ok_or("schema substitution requires a trait-item environment")?;
    for (parameter, argument) in env.ordered_params().zip(args) {
        caller.validate_ty_with_budget(argument, budget)?;
        let declaration = registry
            .declaration_info(parameter.owner.declaration)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        let is_self = declaration.kind == DeclKind::Trait
            && declaration.declaration == trait_owner
            && declaration.own_env.binder() == parameter.owner
            && declaration.own_env.contains(parameter)
            && parameter.index == 0;
        if is_self {
            reference_layers(argument)?;
            let mut inner = argument;
            while let Ty::Ref { referent, .. } = inner {
                budget.charge_work(1)?;
                inner = referent;
            }
            reject_references(inner, budget)?;
        } else {
            reject_references(argument, budget)?;
        }
    }
    substitute_ty_walk(ty, env, args, caller, cx, budget, 0)
}

#[allow(clippy::too_many_arguments)]
fn substitute_ty_walk(
    ty: &Ty,
    env: &GenericEnv,
    args: &[Ty],
    caller: FactsCtx<'_>,
    cx: &mut impl SubstitutionCx,
    budget: &PlanBudget,
    depth: usize,
) -> Result<Ty, String> {
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    budget.charge_nodes(1)?;
    Ok(match ty {
        Ty::Param(param) => {
            budget.charge_work(env.scopes().len())?;
            if let Some(index) = env.argument_index(*param) {
                // Simultaneous substitution: caller arguments are rigid and
                // are never visited again under the callee environment.
                args.get(index)
                    .ok_or("type parameter substitution index out of bounds")?
                    .clone()
            } else {
                if env
                    .scopes()
                    .iter()
                    .any(|scope| scope.binder() == param.owner)
                {
                    return Err("type parameter substitution index out of bounds".into());
                }
                caller.validate_ty_with_budget(ty, budget)?;
                Ty::Param(*param)
            }
        }
        Ty::Tuple(fields) => {
            budget.retain_bytes(
                fields
                    .len()
                    .checked_mul(std::mem::size_of::<Ty>())
                    .ok_or("substitution size overflow")?,
            )?;
            Ty::Tuple(
                fields
                    .iter()
                    .map(|ty| substitute_ty_walk(ty, env, args, caller, cx, budget, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            )
        }
        Ty::Array { element, len } => {
            budget.retain_bytes(std::mem::size_of::<Ty>() + 2 * std::mem::size_of::<usize>())?;
            Ty::Array {
                element: Arc::new(substitute_ty_walk(
                    element,
                    env,
                    args,
                    caller,
                    cx,
                    budget,
                    depth + 1,
                )?),
                len: *len,
            }
        }
        Ty::Ref { kind, referent } => {
            budget.retain_bytes(std::mem::size_of::<Ty>() + 2 * std::mem::size_of::<usize>())?;
            Ty::Ref {
                kind: *kind,
                referent: Arc::new(substitute_ty_walk(
                    referent,
                    env,
                    args,
                    caller,
                    cx,
                    budget,
                    depth + 1,
                )?),
            }
        }
        Ty::Struct(value) => {
            let app = cx.nominal_application(value.def().id)?;
            if app.type_args.len() > 32 {
                return Err("nominal argument limit exceeded".into());
            }
            let _scratch = budget.reserve_bytes(app.type_args.len() * std::mem::size_of::<Ty>())?;
            let arguments = app
                .type_args
                .iter()
                .map(|ty| substitute_ty_walk(ty, env, args, caller, cx, budget, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            cx.intern_nominal(app.source_decl, &arguments, caller, budget)?
        }
        Ty::Enum(value) => {
            let app = cx.nominal_application(value.def().id)?;
            if app.type_args.len() > 32 {
                return Err("nominal argument limit exceeded".into());
            }
            let _scratch = budget.reserve_bytes(app.type_args.len() * std::mem::size_of::<Ty>())?;
            let arguments = app
                .type_args
                .iter()
                .map(|ty| substitute_ty_walk(ty, env, args, caller, cx, budget, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            cx.intern_nominal(app.source_decl, &arguments, caller, budget)?
        }
        Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Never => ty.clone(),
    })
}

/// Instantiate only a universally checked exact source, then lower the returned
/// checked body anew so concrete Copy/Move decisions cannot reuse template tags.
pub fn instantiate_checked(
    template: &super::ownership::VerifiedTemplateCfg,
    args: &[Ty],
    closed_facts: &super::traits::ValidatedFacts,
    cx: &mut impl SubstitutionCx,
    budget: &PlanBudget,
) -> Result<CheckedFunction, String> {
    let env = template.env();
    if !template.budget().same_authority(budget) {
        return Err("generic instance changed planning budget authority".into());
    }
    let target = closed_facts
        .target()
        .ok_or("instance requires closed facts")?;
    if !closed_facts.budget().same_authority(budget)
        || !Arc::ptr_eq(closed_facts.world(), template.facts().world())
        || target.declaration != template.facts().declaration()
        || !closed_facts
            .world()
            .types_equal(&target.type_args, args)
            .map_err(|e| e.into_legacy_diagnostic(budget))?
    {
        return Err("instance facts do not authorize this exact template and arguments".into());
    }
    if args.len() != env.total_arity() {
        return Err("generic instance argument arity mismatch".into());
    }
    for ty in args {
        FactsCtx::from_validated(closed_facts).validate_ty_with_budget(ty, budget)?;
        reject_references(ty, budget)?;
    }
    let source = template.source();
    source.preflight_plan(FactsCtx::from_validated(template.facts()), budget)?;
    let mut output = source.as_ref().clone();
    let mut visitor = Instantiate {
        facts: FactsCtx::from_validated(closed_facts),
        env,
        args,
        cx,
        budget,
    };
    output.result = visitor.ty(&output.result)?;
    for local in &mut output.locals {
        local.ty = visitor.ty(&local.ty)?;
    }
    for expression in &mut output.exprs {
        budget.charge_work(1)?;
        expression.ty = visitor.ty(&expression.ty)?;
        match &mut expression.kind {
            ExprKind::Borrow { place, .. } => visitor.path(&mut place.projection)?,
            ExprKind::Struct { ty, .. } => {
                let Ty::Struct(value) = visitor.ty(&Ty::Struct(ty.clone()))? else {
                    return Err("struct application changed kind".into());
                };
                *ty = value;
            }
            ExprKind::EnumVariant { ty, .. } => visitor.enum_ty(ty)?,
            ExprKind::Match { arms, .. } => {
                for arm in arms {
                    match &mut arm.pattern {
                        MatchPattern::EnumVariant { ty, bindings, .. } => {
                            visitor.enum_ty(ty)?;
                            visitor.bindings(bindings)?;
                        }
                        MatchPattern::Wildcard
                        | MatchPattern::Binding(_)
                        | MatchPattern::Bool(_)
                        | MatchPattern::Int(_)
                        | MatchPattern::Unit => {}
                    }
                }
            }
            ExprKind::Call {
                target, signature, ..
            } => {
                *target = substitute_callee_metadata(
                    target,
                    env,
                    args,
                    closed_facts,
                    visitor.cx,
                    budget,
                )?;
                *signature = visitor.signature(signature)?;
            }
            ExprKind::If {
                then_block,
                else_block,
                ..
            } => {
                visitor.block(then_block, 0)?;
                visitor.block(else_block, 0)?;
            }
            ExprKind::Block(block) | ExprKind::Loop { body: block, .. } => {
                visitor.block(block, 0)?
            }
            ExprKind::IntCast { .. }
            | ExprKind::Deref { .. }
            | ExprKind::DynamicIndex { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::Index { .. }
            | ExprKind::Int(_)
            | ExprKind::Bool(_)
            | ExprKind::Local(_)
            | ExprKind::Tuple(_)
            | ExprKind::Field { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Break { .. }
            | ExprKind::Continue { .. }
            | ExprKind::Println(_) => {}
        }
    }
    visitor.block(&mut output.body, 0)?;
    close_checked_calls(&output, closed_facts, cx, budget)
        .map_err(|e| e.into_legacy_diagnostic(budget))
}

fn substitute_callee_metadata(
    callee: &Callee,
    env: &GenericEnv,
    args: &[Ty],
    closed_facts: &super::traits::ValidatedFacts,
    cx: &mut impl SubstitutionCx,
    budget: &PlanBudget,
) -> Result<Callee, String> {
    if closed_facts.phase() != super::traits::FactsPhase::Closed
        || !closed_facts.budget().same_authority(budget)
    {
        return Err("callee substitution requires exact closed planning authority".into());
    }
    let facts = FactsCtx::from_validated(closed_facts);
    let mut types = |values: &[Ty]| -> Result<Arc<[Ty]>, String> {
        if values.len() > 32 {
            return Err("callee type argument limit exceeded".into());
        }
        budget.retain_bytes(
            std::mem::size_of_val(values)
                .checked_mul(2)
                .and_then(|n| n.checked_add(4 * std::mem::size_of::<usize>()))
                .ok_or("callee argument storage overflow")?,
        )?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(values.len())
            .map_err(|_| "callee argument allocation resource limit")?;
        for ty in values {
            output.push(substitute_ty(ty, env, args, facts, cx, budget)?);
        }
        Ok(output.into())
    };
    let substituted = match callee {
        Callee::Direct(target) => Callee::Direct(CallTarget {
            declaration: target.declaration,
            type_args: types(&target.type_args)?,
        }),
        Callee::TraitItem(call) => {
            let method_args = types(&call.method_args)?;
            let trait_args = types(&call.trait_ref.args)?;
            let self_ty = substitute_ty(&call.trait_ref.self_ty, env, args, facts, cx, budget)?;
            Callee::TraitItem(TraitCall {
                trait_ref: TraitRef {
                    declaration: call.trait_ref.declaration,
                    self_ty,
                    args: trait_args,
                },
                item: call.item,
                method_args,
            })
        }
    };
    Ok(substituted)
}

pub fn substitute_callee(
    callee: &Callee,
    env: &GenericEnv,
    args: &[Ty],
    closed_facts: &super::traits::ValidatedFacts,
    cx: &mut impl SubstitutionCx,
    budget: &PlanBudget,
) -> Result<Callee, String> {
    let substituted = substitute_callee_metadata(callee, env, args, closed_facts, cx, budget)?;
    match substituted {
        Callee::Direct(target) => {
            closed_facts
                .service()
                .discharge_direct(&target, closed_facts)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            Ok(Callee::Direct(target))
        }
        Callee::TraitItem(call) => Ok(closed_facts
            .service()
            .resolve_trait_call(&call, closed_facts)
            .map_err(|e| e.into_legacy_diagnostic(budget))?
            .callee()
            .clone()),
    }
}

pub fn canonicalize_checked_calls(
    source: &CheckedFunction,
    facts: &super::traits::ValidatedFacts,
    _cx: &mut impl SubstitutionCx,
    budget: &PlanBudget,
) -> Result<CheckedFunction, super::traits::FactsError> {
    use super::traits::FactsError;
    if facts.phase() == super::traits::FactsPhase::Schema || !facts.budget().same_authority(budget)
    {
        return Err(FactsError::invalid(
            budget,
            "call closure requires closed facts and same planning authority",
        ));
    }
    source
        .preflight_plan(FactsCtx::from_validated(facts), budget)
        .map_err(|e| FactsError::from_message(budget, &e))?;
    let mut output = source.clone();
    for expression in &mut output.exprs {
        budget
            .charge_work(1)
            .map_err(|e| FactsError::from_message(budget, &e))?;
        if let ExprKind::Call {
            target, signature, ..
        } = &mut expression.kind
        {
            facts
                .service()
                .validate_signature(SignatureSubject::Call(target), signature, facts)?;
            budget
                .retain_bytes(
                    signature
                        .params
                        .len()
                        .checked_mul(2 * std::mem::size_of::<Ty>())
                        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Signature>()))
                        .ok_or_else(|| {
                            FactsError::resource(budget, "canonical signature storage overflow")
                        })?,
                )
                .map_err(|error| FactsError::from_message(budget, &error))?;
            let (callee, canonical) = match target {
                Callee::Direct(target) => {
                    let proof = facts.service().discharge_direct(target, facts)?;
                    (
                        Callee::Direct(proof.target().clone()),
                        proof.signature().clone(),
                    )
                }
                Callee::TraitItem(call) => {
                    let proof = facts.service().resolve_trait_call(call, facts)?;
                    (proof.callee().clone(), proof.signature().clone())
                }
            };
            if facts.phase() == super::traits::FactsPhase::Closed
                && !matches!(callee, Callee::Direct(_))
            {
                return Err(FactsError::invalid(
                    budget,
                    "closed call signature disagrees with canonical declaration",
                ));
            }
            *target = callee;
            *signature = canonical;
        }
    }
    output
        .validate_with_facts(FactsCtx::from_validated(facts))
        .map_err(|e| FactsError::from_message(budget, &e))?;
    Ok(output)
}

pub fn close_checked_calls(
    source: &CheckedFunction,
    facts: &super::traits::ValidatedFacts,
    cx: &mut impl SubstitutionCx,
    budget: &PlanBudget,
) -> Result<CheckedFunction, super::traits::FactsError> {
    if facts.phase() != super::traits::FactsPhase::Closed {
        return Err(super::traits::FactsError::invalid(
            budget,
            "call closure requires closed facts",
        ));
    }
    canonicalize_checked_calls(source, facts, cx, budget)
}

pub(super) fn reject_references(root: &Ty, budget: &PlanBudget) -> Result<(), String> {
    fn walk(ty: &Ty, budget: &PlanBudget, depth: usize) -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        match ty {
            Ty::Ref { .. } => return Err("reference generic arguments are unsupported".into()),
            Ty::Tuple(fields) => {
                for field in fields.iter() {
                    walk(field, budget, depth + 1)?;
                }
            }
            Ty::Array { element, .. } => walk(element, budget, depth + 1)?,
            Ty::Struct(value) => {
                for argument in value.def().type_args.iter() {
                    walk(argument, budget, depth + 1)?;
                }
                for field in value.def().fields.iter() {
                    walk(&field.ty, budget, depth + 1)?;
                }
            }
            Ty::Enum(value) => {
                for argument in value.def().type_args.iter() {
                    walk(argument, budget, depth + 1)?;
                }
                for variant in value.def().variants.iter() {
                    budget.charge_work(1)?;
                    for field in variant.fields.iter() {
                        walk(&field.ty, budget, depth + 1)?;
                    }
                }
            }
            Ty::Param(_) | Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Never => {}
        }
        Ok(())
    }
    walk(root, budget, 0)
}
struct Instantiate<'a, C> {
    facts: FactsCtx<'a>,
    env: &'a GenericEnv,
    args: &'a [Ty],
    cx: &'a mut C,
    budget: &'a PlanBudget,
}
impl<C: SubstitutionCx> Instantiate<'_, C> {
    fn ty(&mut self, ty: &Ty) -> Result<Ty, String> {
        substitute_ty(ty, self.env, self.args, self.facts, self.cx, self.budget)
    }
    fn enum_ty(&mut self, ty: &mut EnumTy) -> Result<(), String> {
        let Ty::Enum(value) = self.ty(&Ty::Enum(ty.clone()))? else {
            return Err("enum application changed kind".into());
        };
        *ty = value;
        Ok(())
    }
    fn signature(&mut self, signature: &Signature) -> Result<Signature, String> {
        self.budget.retain_bytes(
            signature
                .params
                .len()
                .checked_mul(2 * std::mem::size_of::<Ty>())
                .ok_or("signature storage overflow")?,
        )?;
        Ok(Signature {
            ref_contract: signature.ref_contract.clone(),
            params: signature
                .params
                .iter()
                .map(|ty| self.ty(ty))
                .collect::<Result<Vec<_>, _>>()?,
            result: self.ty(&signature.result)?,
        })
    }
    fn path(&mut self, path: &mut [Projection]) -> Result<(), String> {
        self.budget.charge_work(path.len())?;
        for projection in path {
            if let Projection::Downcast(variant) = projection {
                let application = self.cx.nominal_application(variant.owner)?;
                if application.type_args.len() > 32 {
                    return Err("nominal argument limit exceeded".into());
                }
                let _scratch = self
                    .budget
                    .reserve_bytes(application.type_args.len() * std::mem::size_of::<Ty>())?;
                let args = application
                    .type_args
                    .iter()
                    .map(|ty| self.ty(ty))
                    .collect::<Result<Vec<_>, _>>()?;
                let Ty::Enum(value) = self.cx.intern_nominal(
                    application.source_decl,
                    &args,
                    self.facts,
                    self.budget,
                )?
                else {
                    return Err("downcast application is not enum".into());
                };
                variant.owner = value.def().id;
                if variant.index >= value.def().variants.len() {
                    return Err("instantiated variant index out of bounds".into());
                }
            }
        }
        Ok(())
    }
    fn bindings(&mut self, bindings: &mut [PatternBinding]) -> Result<(), String> {
        for binding in bindings {
            self.path(&mut binding.projection)?;
        }
        Ok(())
    }
    fn block(&mut self, block: &mut Block, depth: usize) -> Result<(), String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(block.statements.len())?;
        for statement in &mut block.statements {
            match statement {
                Stmt::AssignOp { place, .. } | Stmt::AssignPlace { place, .. } => {
                    self.path(&mut place.projection)?
                }
                Stmt::AssignIndexOp { array, .. } | Stmt::AssignIndex { array, .. } => {
                    self.path(&mut array.projection)?
                }
                Stmt::LetPattern { ty, bindings, .. } => {
                    *ty = self.ty(ty)?;
                    self.bindings(bindings)?;
                }
                Stmt::While { body, .. } => self.block(body, depth + 1)?,
                Stmt::Declare { .. }
                | Stmt::Let { .. }
                | Stmt::Assign { .. }
                | Stmt::Expr(_)
                | Stmt::Return(_) => {}
            }
        }
        Ok(())
    }
}

pub const MAX_ENUM_VARIANTS: usize = 4096;
pub const MAX_MATCH_ARMS: usize = 4096;
pub const MAX_ARRAY_LEN: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntTy {
    I8,
    I16,
    I32,
    I64,
    Isize,
    U8,
    U16,
    U32,
    U64,
    Usize,
}

impl IntTy {
    pub const fn bits(self) -> u32 {
        match self {
            Self::I8 | Self::U8 => 8,
            Self::I16 | Self::U16 => 16,
            Self::I32 | Self::U32 => 32,
            Self::I64 | Self::Isize | Self::U64 | Self::Usize => 64,
        }
    }

    pub const fn is_signed(self) -> bool {
        matches!(
            self,
            Self::I8 | Self::I16 | Self::I32 | Self::I64 | Self::Isize
        )
    }

    pub const fn mask(self) -> u64 {
        u64::MAX >> (64 - self.bits())
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::Isize => "isize",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::Usize => "usize",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntValue {
    pub ty: IntTy,
    pub bits: u64,
}

impl IntValue {
    pub fn validate(self) -> Result<(), String> {
        if self.bits & !self.ty.mask() != 0 {
            Err("integer constant contains noncanonical high bits".into())
        } else {
            Ok(())
        }
    }

    pub const fn i32(value: i32) -> Self {
        Self {
            ty: IntTy::I32,
            bits: value as u32 as u64,
        }
    }

    pub const fn usize(value: u64) -> Self {
        Self {
            ty: IntTy::Usize,
            bits: value,
        }
    }
}

#[derive(Clone)]
pub struct EnumTy(Arc<EnumDef>);

impl EnumTy {
    pub(super) fn new(def: EnumDef) -> Self {
        Self(Arc::new(def))
    }

    pub fn def(&self) -> &EnumDef {
        &self.0
    }

    pub fn has_payload(&self) -> bool {
        self.0
            .variants
            .iter()
            .any(|variant| variant.shape != VariantShape::Unit)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.0.variants.is_empty() || self.0.variants.len() > MAX_ENUM_VARIANTS {
            return Err("enum variant count must be between 1 and 4096".into());
        }
        let mut fields = 0usize;
        for variant in self.0.variants.iter() {
            fields = fields
                .checked_add(variant.fields.len())
                .ok_or("enum payload field limit exceeded")?;
            if fields > MAX_PATTERN_BINDINGS {
                return Err("enum payload field limit exceeded".into());
            }
            if variant.shape == VariantShape::Unit && !variant.fields.is_empty() {
                return Err("unit enum variant cannot have payload fields".into());
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for EnumTy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = &self.0.name;
        let end = name
            .char_indices()
            .nth(128)
            .map_or(name.len(), |(at, _)| at);
        f.debug_struct("EnumTy")
            .field("id", &self.0.id)
            .field("name", &&name[..end])
            .finish_non_exhaustive()
    }
}

impl PartialEq for EnumTy {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}
impl Eq for EnumTy {}
impl Hash for EnumTy {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub id: NominalAppId,
    pub source_decl: DeclId,
    pub type_args: Arc<[Ty]>,
    pub name: String,
    pub module: String,
    pub public: bool,
    pub variants: Arc<[VariantDef]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VariantShape {
    Unit,
    Tuple,
    Struct,
}

#[derive(Clone, Debug)]
pub struct VariantDef {
    pub name: String,
    pub shape: VariantShape,
    pub fields: Arc<[FieldDef]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariantId {
    pub owner: NominalAppId,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Projection {
    Field(usize),
    Downcast(VariantId),
    ConstantIndex(usize),
    Deref,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BorrowKind {
    Shared,
    Mut,
}

/// Checks structural typing, not the runtime proof needed to read a payload.
pub fn projected_ty<'a>(root: &'a Ty, path: &[Projection]) -> Result<&'a Ty, String> {
    if path.len() > MAX_PATTERN_PROJECTION_DEPTH {
        return Err("projection depth exceeds limit 128".into());
    }
    let derefs = path
        .iter()
        .filter(|part| matches!(part, Projection::Deref))
        .count();
    if derefs != 0
        && (derefs
            != path
                .iter()
                .take_while(|part| matches!(part, Projection::Deref))
                .count()
            || derefs > reference_layers(root)?
            || path
                .iter()
                .any(|part| matches!(part, Projection::Downcast(_))))
    {
        return Err(
            "unsupported: dereference must be a leading typed chain and cannot contain downcasts"
                .into(),
        );
    }
    let mut ty = root;
    let mut path = path.iter();
    while let Some(projection) = path.next() {
        ty = match *projection {
            Projection::Deref => {
                let Ty::Ref { referent, .. } = ty else {
                    return Err("dereference requires reference type".into());
                };
                referent
            }
            Projection::ConstantIndex(index) => {
                let Ty::Array { element, len } = ty else {
                    return Err("constant index requires array type".into());
                };
                if *len > MAX_ARRAY_LEN || index >= *len {
                    return Err("array index out of bounds".into());
                }
                element
            }
            Projection::Field(index) => ty.field_ty(index).ok_or("invalid field projection")?,
            Projection::Downcast(variant) => {
                let Ty::Enum(owner) = ty else {
                    return Err("downcast requires enum type".into());
                };
                if owner.def().id != variant.owner {
                    return Err("downcast enum owner mismatch".into());
                }
                let def = owner
                    .def()
                    .variants
                    .get(variant.index)
                    .ok_or("downcast variant out of bounds")?;
                let Some(Projection::Field(index)) = path.next() else {
                    return Err("downcast must be immediately followed by field projection".into());
                };
                &def.fields
                    .get(*index)
                    .ok_or("invalid variant field projection")?
                    .ty
            }
        };
    }
    Ok(ty)
}

/// Immutable nominal handle. Equality and hashing use declaration identity,
/// never field shape, names or Arc's structural pointee equality.
#[derive(Clone)]
pub struct StructTy(Arc<StructDef>);

impl std::fmt::Debug for StructTy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Types are a shared DAG, not a tree to expand into diagnostics. Names
        // themselves can be long, so bound their diagnostic rendering as well.
        let name = &self.0.name;
        let end = name
            .char_indices()
            .nth(128)
            .map_or(name.len(), |(at, _)| at);
        f.debug_struct("StructTy")
            .field("id", &self.0.id)
            .field("name", &&name[..end])
            .finish_non_exhaustive()
    }
}

impl StructTy {
    /// The declaration builder must validate the entire by-value dependency DAG
    /// before constructing definitions in dependency-first order.
    pub(super) fn new(def: StructDef) -> Self {
        Self(Arc::new(def))
    }

    pub fn def(&self) -> &StructDef {
        &self.0
    }

    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.0.fields.iter().position(|field| field.name == name)
    }
}

impl PartialEq for StructTy {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Eq for StructTy {}

impl Hash for StructTy {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub id: NominalAppId,
    pub source_decl: DeclId,
    pub type_args: Arc<[Ty]>,
    /// Canonical module-qualified name, used for diagnostics, not identity.
    pub name: String,
    /// Defining module, used for owner/descendant field visibility checks.
    pub module: String,
    pub public: bool,
    /// Declaration order. No layout offsets or implicit Copy permission.
    pub fields: Arc<[FieldDef]>,
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub public: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId(pub usize);
/// Resolved control-flow target, allocated monotonically within one function.
/// Labels are not represented by this milestone's source language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoopId(pub usize);

/// Resolved semantic types. Inference variables never cross this boundary.
/// Tuple structure is immutable and shared cheaply across checked facts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Param(ParamId),
    Int(IntTy),
    Ref {
        kind: BorrowKind,
        referent: Arc<Ty>,
    },
    Bool,
    Unit,
    /// Nonempty structural tuple. The empty tuple is canonically Unit.
    /// Rust Copy semantics for admitted tuples do not make this host enum Copy.
    Tuple(Arc<[Ty]>),
    Array {
        element: Arc<Ty>,
        len: usize,
    },
    /// Nominal and always non-Copy in this slice, even with only scalar fields.
    Struct(StructTy),
    Enum(EnumTy),
    /// The semantic never type, distinct from runtime completion information.
    Never,
}

impl Ty {
    pub fn field_count(&self) -> Option<usize> {
        match self {
            Self::Tuple(fields) => Some(fields.len()),
            Self::Struct(ty) => Some(ty.def().fields.len()),
            _ => None,
        }
    }

    pub fn field_ty(&self, index: usize) -> Option<&Ty> {
        match self {
            Self::Tuple(fields) => fields.get(index),
            Self::Struct(ty) => ty.def().fields.get(index).map(|field| &field.ty),
            _ => None,
        }
    }

    pub fn is_copy(&self) -> bool {
        match self {
            Self::Ref { kind, .. } => *kind == BorrowKind::Shared,
            Self::Int(_) | Self::Bool | Self::Unit | Self::Never => true,
            Self::Struct(_) | Self::Enum(_) | Self::Param(_) => false,
            Self::Tuple(_) | Self::Array { .. } => {
                let mut pending = vec![self];
                let mut seen = std::collections::HashSet::new();
                let mut arrays = std::collections::HashSet::new();
                while let Some(ty) = pending.pop() {
                    match ty {
                        Self::Ref {
                            kind: BorrowKind::Mut,
                            ..
                        } => return false,
                        Self::Struct(_) | Self::Enum(_) | Self::Param(_) => return false,
                        Self::Tuple(fields) if seen.insert(fields.as_ptr()) => {
                            pending.extend(fields.iter());
                        }
                        Self::Array { element, .. } if arrays.insert(Arc::as_ptr(element)) => {
                            // Zero length does not grant implicit Copy permission.
                            pending.push(element);
                        }
                        _ => {}
                    }
                }
                true
            }
        }
    }
}

impl<'f> FactsCtx<'f> {
    pub fn from_validated(facts: &'f super::traits::ValidatedFacts) -> Self {
        Self::Validated(facts)
    }
    pub fn validated(self) -> Option<&'f super::traits::ValidatedFacts> {
        match self {
            Self::Validated(facts) => Some(facts),
            _ => None,
        }
    }
    pub fn validate_ty(self, root: &Ty) -> Result<(), String> {
        if let Self::Validated(facts) = self {
            return facts
                .validate_ty(root)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()));
        }
        self.validate_ty_inner(root, None)
    }
    pub fn validate_ty_with_budget(self, root: &Ty, budget: &PlanBudget) -> Result<(), String> {
        if let Self::Validated(facts) = self {
            return facts
                .validate_ty_with_budget(root, budget)
                .map_err(|e| e.into_legacy_diagnostic(budget));
        }
        self.validate_ty_inner(root, Some(budget))
    }
    fn validate_ty_inner(self, root: &Ty, budget: Option<&PlanBudget>) -> Result<(), String> {
        fn walk(
            facts: FactsCtx<'_>,
            ty: &Ty,
            depth: usize,
            work: &mut usize,
            budget: Option<&PlanBudget>,
        ) -> Result<(), String> {
            *work += 1;
            if *work > MAX_PATTERN_PROJECTION_WORK || depth > 128 {
                return Err("type facts resource limit exceeded".into());
            }
            if let Some(budget) = budget {
                budget.check_depth(depth)?;
                budget.charge_work(1)?;
            }
            match ty {
                Ty::Param(param) => {
                    let FactsCtx::Universal(env) = facts else {
                        return Err("type parameter is outside its authoritative binder".into());
                    };
                    *work += env.scopes().len();
                    if *work > MAX_PATTERN_PROJECTION_WORK {
                        return Err("type facts resource limit exceeded".into());
                    }
                    if let Some(budget) = budget {
                        budget.charge_work(env.scopes().len())?;
                    }
                    if !env.contains(*param) {
                        return Err("type parameter is outside its authoritative binder".into());
                    }
                }
                Ty::Ref { referent, .. } => {
                    if depth != 0 {
                        return Err("nested or aggregate references are unsupported".into());
                    }
                    let layers = reference_layers(ty)?;
                    let mut inner = referent.as_ref();
                    for _ in 1..layers {
                        let Ty::Ref { referent, .. } = inner else {
                            unreachable!()
                        };
                        inner = referent;
                        *work += 1;
                        if let Some(budget) = budget {
                            budget.charge_work(1)?;
                        }
                    }
                    walk(facts, inner, depth + layers, work, budget)?;
                }
                Ty::Tuple(fields) => {
                    for child in fields.iter() {
                        walk(facts, child, depth + 1, work, budget)?;
                    }
                }
                Ty::Array { element, len } => {
                    if *len > MAX_ARRAY_LEN {
                        return Err("array length limit exceeded".into());
                    }
                    walk(facts, element, depth + 1, work, budget)?;
                }
                Ty::Struct(value) => {
                    if value.def().type_args.len() > 32 {
                        return Err("nominal argument limit exceeded".into());
                    }
                    for child in value.def().type_args.iter() {
                        walk(facts, child, depth + 1, work, budget)?;
                    }
                    for field in value.def().fields.iter() {
                        walk(facts, &field.ty, depth + 1, work, budget)?;
                    }
                }
                Ty::Enum(value) => {
                    if value.def().type_args.len() > 32
                        || value.def().variants.len() > MAX_ENUM_VARIANTS
                    {
                        return Err("nominal metadata limit exceeded".into());
                    }
                    for child in value.def().type_args.iter() {
                        walk(facts, child, depth + 1, work, budget)?;
                    }
                    for variant in value.def().variants.iter() {
                        *work += 1;
                        if *work > MAX_PATTERN_PROJECTION_WORK {
                            return Err("type facts resource limit exceeded".into());
                        }
                        if let Some(budget) = budget {
                            budget.charge_work(1)?;
                        }
                        for field in variant.fields.iter() {
                            walk(facts, &field.ty, depth + 1, work, budget)?;
                        }
                    }
                }
                Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Never => {}
            }
            Ok(())
        }
        walk(self, root, 0, &mut 0, budget)
    }
    pub fn must_copy(self, ty: &Ty) -> Result<bool, String> {
        if let Self::Validated(facts) = self {
            return facts
                .must_copy(ty)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()));
        }
        self.validate_ty(ty)?;
        Ok(ty.is_copy())
    }
    pub fn projected_ty<'a>(self, root: &'a Ty, path: &[Projection]) -> Result<&'a Ty, String> {
        if let Self::Validated(facts) = self {
            return facts
                .projected_ty(root, path)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()));
        }
        self.validate_ty(root)?;
        projected_ty(root, path)
    }
}

/// Static normal-completion summary, independent of semantic value type.
/// Returns is conservative: it does not prove an exit is dynamically reachable
/// or that evaluation terminates. Never does not erase an explicit tail's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Returns,
    Never,
}

#[derive(Clone, Debug)]
pub struct LocalDecl {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub ref_contract: Arc<RefContract>,
    pub params: Vec<Ty>,
    pub result: Ty,
}

#[derive(Clone, Debug)]
pub struct CheckedFunction {
    pub ref_contract: Arc<RefContract>,
    pub name: String,
    /// Ordered parameter bindings, distinct entries in `locals`.
    pub params: Vec<LocalId>,
    pub result: Ty,
    pub locals: Vec<LocalDecl>,
    pub exprs: Vec<TypedExpr>,
    pub body: Block,
}

pub const MAX_REGION_PARAMS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionParamId(pub u16);

pub const MAX_INPUT_PATHS: usize = 4096;
pub const MAX_INPUT_PATH_STEPS: usize = 131_072;
pub const MAX_INPUT_PATH_OCCURRENCES: usize = 131_072;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReferencePosition {
    pub depth: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InputReference {
    pub param: usize,
    pub position: ReferencePosition,
}

/// A latent reference-value channel belonging to one rigid type parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ParameterCarrier {
    pub parameter: ParamId,
    pub relative: ReferencePosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InputCarrierTail {
    pub param: usize,
    pub prefix: u8,
    pub parameter: ParamId,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResultCarrierTail {
    pub prefix: u8,
    pub parameter: ParamId,
    pub inputs: Arc<[InputCarrierTail]>,
}

/// Data-only declaration schema. This does not authorize topology realization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureOriginSchema {
    pub fixed: Arc<RefContract>,
    pub input_tails: Arc<[InputCarrierTail]>,
    pub result_tails: Arc<[ResultCarrierTail]>,
}

pub fn derive_signature_origin_schema(
    signature: &Signature,
    env: &GenericEnv,
    budget: &PlanBudget,
) -> Result<Arc<SignatureOriginSchema>, String> {
    fn tail(
        ty: &Ty,
        env: &GenericEnv,
        budget: &PlanBudget,
    ) -> Result<Option<(u8, ParamId)>, String> {
        let layers = reference_layers(ty)?;
        let mut inner = ty;
        for _ in 0..layers {
            budget.charge_work(1)?;
            let Ty::Ref { referent, .. } = inner else {
                return Err("signature-origin reference prefix mismatch".into());
            };
            inner = referent;
        }
        budget.charge_work(1 + env.scopes().len())?;
        if let Ty::Param(parameter) = inner {
            if !env.contains(*parameter) {
                return Err("signature-origin parameter is outside declaration environment".into());
            }
            Ok(Some((layers as u8, *parameter)))
        } else {
            Ok(None)
        }
    }
    if signature.params.len() > MAX_LOAN_SITES {
        return Err("signature-origin parameter limit exceeded".into());
    }
    let capacity = signature.params.len();
    // Vec-to-Arc conversion and the result's filtered copy may coexist.
    let storage = capacity
        .checked_mul(3 * std::mem::size_of::<InputCarrierTail>())
        .and_then(|bytes| {
            bytes.checked_add(
                std::mem::size_of::<SignatureOriginSchema>()
                    + 2 * std::mem::size_of::<ResultCarrierTail>()
                    + 12 * std::mem::size_of::<usize>(),
            )
        })
        .ok_or("signature-origin storage overflow")?;
    budget.retain_bytes(storage)?;
    budget.charge_work(capacity)?;
    let mut inputs = Vec::new();
    inputs
        .try_reserve_exact(capacity)
        .map_err(|_| "signature-origin allocation failed")?;
    for (param, ty) in signature.params.iter().enumerate() {
        if let Some((prefix, parameter)) = tail(ty, env, budget)? {
            inputs.push(InputCarrierTail {
                param,
                prefix,
                parameter,
            });
        }
    }
    let results: Arc<[ResultCarrierTail]> =
        if let Some((prefix, parameter)) = tail(&signature.result, env, budget)? {
            budget.charge_work(inputs.len())?;
            let mut matching = Vec::new();
            matching
                .try_reserve_exact(inputs.len())
                .map_err(|_| "signature-origin allocation failed")?;
            matching.extend(
                inputs
                    .iter()
                    .copied()
                    .filter(|input| input.parameter == parameter),
            );
            Arc::from([ResultCarrierTail {
                prefix,
                parameter,
                inputs: matching.into(),
            }])
        } else {
            Arc::from([])
        };
    Ok(Arc::new(SignatureOriginSchema {
        fixed: signature.ref_contract.clone(),
        input_tails: inputs.into(),
        result_tails: results,
    }))
}

/// Number of root reference-value layers, not memory projections.
pub fn reference_layers(ty: &Ty) -> Result<usize, String> {
    let mut current = ty;
    let mut count = 0;
    let mut mutable = false;
    while let Ty::Ref { kind, referent } = current {
        count += 1;
        mutable |= *kind == BorrowKind::Mut;
        if count > 2 || (count > 1 && mutable) {
            return Err(
                "unsupported: nested or aggregate references: mutable nesting or depth greater than two".into(),
            );
        }
        current = referent;
    }
    Ok(count)
}

/// Returns the reference kind and its authoritative referent at a value position.
pub fn reference_at(ty: &Ty, position: ReferencePosition) -> Result<(BorrowKind, &Ty), String> {
    let layers = reference_layers(ty)?;
    if usize::from(position.depth) >= layers {
        return Err("reference position is outside typed carrier".into());
    }
    let mut current = ty;
    for _ in 0..position.depth {
        let Ty::Ref { referent, .. } = current else {
            unreachable!()
        };
        current = referent;
    }
    let Ty::Ref { kind, referent } = current else {
        unreachable!()
    };
    Ok((*kind, referent))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputPath {
    pub input: InputReference,
    pub projection: Arc<[Projection]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputEnvelope {
    pub input: InputReference,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ReturnContract {
    #[default]
    NoReference,
    SharedInputPaths(Arc<[InputPath]>),
    MutInputPaths(Arc<[InputPath]>),
    SharedInputEnvelopes(Arc<[InputEnvelope]>),
    MutInputEnvelopes(Arc<[InputEnvelope]>),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RefContract {
    pub region_count: u16,
    pub input_regions: Arc<[InputRegion]>,
    pub results: Arc<[ResultReference]>,
    pub outlives: Arc<[(RegionParamId, RegionParamId)]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputRegion {
    pub input: InputReference,
    pub region: RegionParamId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultReference {
    pub position: ReferencePosition,
    pub region: RegionParamId,
    pub returns: ReturnContract,
}

fn contract_rows(contract: &RefContract, budget: &PlanBudget) -> Result<[u64; 64], String> {
    let count = usize::from(contract.region_count);
    if count > MAX_REGION_PARAMS
        || contract.outlives.len() > 4096
        || contract.input_regions.len() > MAX_PATTERN_PROJECTION_WORK
        || contract.results.len() > 2
    {
        return Err("reference region resource limit exceeded".into());
    }
    budget.charge_work(
        contract.input_regions.len()
            + contract.results.len()
            + contract.outlives.len()
            + count * count * count
            + 64,
    )?;
    let mut previous = None;
    for row in contract.input_regions.iter() {
        if row.input.position.depth > 1 || previous.is_some_and(|old| old >= row.input) {
            return Err("noncanonical input reference position order".into());
        }
        previous = Some(row.input);
    }
    let mut previous = None;
    for row in contract.results.iter() {
        if row.position.depth > 1 || previous.is_some_and(|old| old >= row.position) {
            return Err("noncanonical result reference position order".into());
        }
        previous = Some(row.position);
    }
    for region in contract
        .input_regions
        .iter()
        .map(|row| row.region)
        .chain(contract.results.iter().map(|row| row.region))
    {
        if usize::from(region.0) >= count {
            return Err("reference region is outside declaration".into());
        }
    }
    let mut rows = [0u64; 64];
    for (index, row) in rows.iter_mut().enumerate().take(count) {
        *row = 1u64 << index;
    }
    for (longer, shorter) in contract.outlives.iter() {
        if usize::from(longer.0) >= count || usize::from(shorter.0) >= count {
            return Err("outlives region is outside declaration".into());
        }
        rows[usize::from(longer.0)] |= 1u64 << shorter.0;
    }
    close_region_rows(&mut rows, count);
    Ok(rows)
}

fn close_region_rows(rows: &mut [u64; 64], count: usize) {
    for via in 0..count {
        for from in 0..count {
            if rows[from] & (1u64 << via) != 0 {
                rows[from] |= rows[via];
            }
        }
    }
}

fn stored_candidate_work(contract: &RefContract, budget: &PlanBudget) -> Result<(), String> {
    let mut candidates = 0usize;
    let mut steps = 0usize;
    for result in contract.results.iter() {
        match &result.returns {
            ReturnContract::NoReference => {}
            ReturnContract::SharedInputPaths(paths) | ReturnContract::MutInputPaths(paths) => {
                candidates = candidates
                    .checked_add(paths.len())
                    .ok_or("input candidate overflow")?;
                for path in paths.iter() {
                    if path.projection.len() > MAX_PATTERN_PROJECTION_DEPTH {
                        return Err("input path depth resource limit exceeded".into());
                    }
                    steps = steps
                        .checked_add(path.projection.len())
                        .ok_or("input path step overflow")?;
                    if steps > MAX_INPUT_PATH_STEPS {
                        return Err("input path step resource limit exceeded".into());
                    }
                    budget.charge_work(1 + path.projection.len())?;
                    if path.input.position.depth > 1
                        || path.projection.iter().any(|step| {
                            !matches!(step, Projection::Field(_) | Projection::ConstantIndex(_))
                        })
                    {
                        return Err("forbidden input path projection or reference position".into());
                    }
                }
            }
            ReturnContract::SharedInputEnvelopes(paths)
            | ReturnContract::MutInputEnvelopes(paths) => {
                candidates = candidates
                    .checked_add(paths.len())
                    .ok_or("input candidate overflow")?;
                budget.charge_work(paths.len())?;
                if paths.iter().any(|path| path.input.position.depth > 1) {
                    return Err("invalid input reference position".into());
                }
            }
        }
        if candidates > MAX_INPUT_PATHS {
            return Err("reference return candidate limit exceeded".into());
        }
    }
    Ok(())
}

/// Pure metadata normalization. It grants neither declaration nor body authority.
pub fn normalize_ref_contract(
    raw: &RefContract,
    budget: &PlanBudget,
) -> Result<Arc<RefContract>, String> {
    let _scratch = budget.reserve_bytes(64 * 12)?;
    let mut rows = contract_rows(raw, budget)?;
    stored_candidate_work(raw, budget)?;
    for pair in raw.input_regions.windows(2) {
        if pair[0].input.param == pair[1].input.param
            && pair[0].input.position.depth == 0
            && pair[1].input.position.depth == 1
        {
            rows[usize::from(pair[1].region.0)] |= 1u64 << pair[0].region.0;
        }
    }
    if let [outer, inner] = raw.results.as_ref()
        && outer.position.depth == 0
        && inner.position.depth == 1
    {
        rows[usize::from(inner.region.0)] |= 1u64 << outer.region.0;
    }
    budget.charge_work(usize::from(raw.region_count).pow(3))?;
    close_region_rows(&mut rows, usize::from(raw.region_count));
    let mut map = [None; 64];
    let mut old = [0usize; 64];
    let mut count = 0usize;
    for region in raw
        .input_regions
        .iter()
        .map(|row| row.region)
        .chain(raw.results.iter().map(|row| row.region))
    {
        budget.charge_work(1)?;
        let index = usize::from(region.0);
        if map[index].is_none() {
            map[index] = Some(RegionParamId(count as u16));
            old[count] = index;
            count += 1;
        }
    }
    budget.charge_work(count * count * 2 + raw.input_regions.len() + raw.results.len())?;
    let edges = (0..count)
        .map(|from| {
            (0..count)
                .filter(|to| *to != from && rows[old[from]] & (1u64 << old[*to]) != 0)
                .count()
        })
        .sum::<usize>();
    budget.retain_bytes(
        std::mem::size_of::<RefContract>()
            + raw.input_regions.len() * 2 * std::mem::size_of::<InputRegion>()
            + raw.results.len() * 2 * std::mem::size_of::<ResultReference>()
            + edges * 2 * std::mem::size_of::<(RegionParamId, RegionParamId)>()
            + 8 * std::mem::size_of::<usize>(),
    )?;
    let mut outlives = Vec::new();
    outlives
        .try_reserve_exact(edges)
        .map_err(|_| "region normalization allocation failed")?;
    for from in 0..count {
        for to in 0..count {
            if from != to && rows[old[from]] & (1u64 << old[to]) != 0 {
                outlives.push((RegionParamId(from as u16), RegionParamId(to as u16)));
            }
        }
    }
    let mut input_regions = Vec::new();
    input_regions
        .try_reserve_exact(raw.input_regions.len())
        .map_err(|_| "region normalization allocation failed")?;
    for row in raw.input_regions.iter() {
        input_regions.push(InputRegion {
            input: row.input,
            region: map[usize::from(row.region.0)].expect("used region"),
        });
    }
    let mut results = Vec::new();
    results
        .try_reserve_exact(raw.results.len())
        .map_err(|_| "region normalization allocation failed")?;
    for row in raw.results.iter() {
        results.push(ResultReference {
            position: row.position,
            region: map[usize::from(row.region.0)].expect("used region"),
            returns: row.returns.clone(),
        });
    }
    Ok(Arc::new(RefContract {
        region_count: count as u16,
        input_regions: input_regions.into(),
        results: results.into(),
        outlives: outlives.into(),
    }))
}

fn ref_type_walk(
    ty: &Ty,
    root: bool,
    budget: &PlanBudget,
    depth: usize,
    visits: &mut usize,
) -> Result<bool, String> {
    fn walk(
        ty: &Ty,
        root: bool,
        budget: &PlanBudget,
        depth: usize,
        visits: &mut usize,
        memo: &mut std::collections::HashMap<(usize, bool), (bool, usize)>,
    ) -> Result<(bool, usize), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        *visits += 1;
        if *visits > MAX_PATTERN_PROJECTION_WORK {
            return Err("reference type work limit exceeded".into());
        }
        let key = (std::ptr::from_ref(ty) as usize, root);
        if let Some(&(opaque, height)) = memo.get(&key) {
            budget.check_depth(
                depth
                    .checked_add(height)
                    .ok_or("reference type depth overflow")?,
            )?;
            return Ok((opaque, height));
        }
        let mut opaque = matches!(ty, Ty::Param(_));
        let mut height = 0;
        let mut child = |value: &Ty| -> Result<(), String> {
            let (is_opaque, child_height) = walk(value, false, budget, depth + 1, visits, memo)?;
            opaque |= is_opaque;
            height = height.max(child_height + 1);
            Ok(())
        };
        match ty {
            Ty::Ref { referent, .. } => {
                if !root {
                    return Err("nested or aggregate references are unsupported".into());
                }
                reference_layers(ty)?;
                let (is_opaque, child_height) =
                    walk(referent, true, budget, depth + 1, visits, memo)?;
                opaque |= is_opaque;
                height = height.max(child_height + 1);
            }
            Ty::Tuple(fields) => {
                for field in fields.iter() {
                    child(field)?;
                }
            }
            Ty::Array { element, len } => {
                if *len > MAX_ARRAY_LEN {
                    return Err("array length resource limit exceeded".into());
                }
                child(element)?;
            }
            Ty::Struct(value) => {
                for argument in value.def().type_args.iter() {
                    child(argument)?;
                }
                for field in value.def().fields.iter() {
                    child(&field.ty)?;
                }
            }
            Ty::Enum(value) => {
                for argument in value.def().type_args.iter() {
                    child(argument)?;
                }
                for variant in value.def().variants.iter() {
                    budget.charge_work(1)?;
                    for field in variant.fields.iter() {
                        child(&field.ty)?;
                    }
                }
            }
            Ty::Param(_) | Ty::Int(_) | Ty::Bool | Ty::Unit | Ty::Never => {}
        }
        budget.retain_bytes(4 * std::mem::size_of::<((usize, bool), (bool, usize))>())?;
        memo.try_reserve(1)
            .map_err(|_| "reference type memo allocation failed")?;
        memo.insert(key, (opaque, height));
        Ok((opaque, height))
    }
    walk(
        ty,
        root,
        budget,
        depth,
        visits,
        &mut std::collections::HashMap::new(),
    )
    .map(|(opaque, _)| opaque)
}

fn ref_types_equal(
    left: &Ty,
    right: &Ty,
    budget: &PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    fn walk(
        left: &Ty,
        right: &Ty,
        budget: &PlanBudget,
        depth: usize,
        memo: &mut std::collections::HashMap<(usize, usize), (bool, usize)>,
    ) -> Result<(bool, usize), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        let key = (
            std::ptr::from_ref(left) as usize,
            std::ptr::from_ref(right) as usize,
        );
        if let Some(&(equal, height)) = memo.get(&key) {
            budget.check_depth(
                depth
                    .checked_add(height)
                    .ok_or("reference equality depth overflow")?,
            )?;
            return Ok((equal, height));
        }
        let mut height = 0;
        let mut child = |left: &Ty, right: &Ty| -> Result<bool, String> {
            let (equal, child_height) = walk(left, right, budget, depth + 1, memo)?;
            height = height.max(child_height + 1);
            Ok(equal)
        };
        let equal = match (left, right) {
            (Ty::Tuple(a), Ty::Tuple(b)) => {
                if a.len() != b.len() {
                    false
                } else {
                    let mut equal = true;
                    for (a, b) in a.iter().zip(b.iter()) {
                        equal &= child(a, b)?;
                    }
                    equal
                }
            }
            (Ty::Array { element: a, len: x }, Ty::Array { element: b, len: y }) => {
                x == y && child(a, b)?
            }
            (
                Ty::Ref {
                    kind: a,
                    referent: x,
                },
                Ty::Ref {
                    kind: b,
                    referent: y,
                },
            ) => a == b && child(x, y)?,
            (Ty::Struct(a), Ty::Struct(b)) => a.def().id == b.def().id,
            (Ty::Enum(a), Ty::Enum(b)) => a.def().id == b.def().id,
            (Ty::Param(a), Ty::Param(b)) => a == b,
            (Ty::Int(a), Ty::Int(b)) => a == b,
            (Ty::Bool, Ty::Bool) | (Ty::Unit, Ty::Unit) | (Ty::Never, Ty::Never) => true,
            _ => false,
        };
        budget.retain_bytes(4 * std::mem::size_of::<((usize, usize), (bool, usize))>())?;
        memo.try_reserve(1)
            .map_err(|_| "reference equality memo allocation failed")?;
        memo.insert(key, (equal, height));
        Ok((equal, height))
    }
    walk(
        left,
        right,
        budget,
        depth,
        &mut std::collections::HashMap::new(),
    )
    .map(|(equal, _)| equal)
}

/// Exact typed position coverage. The erased form is structural only here:
/// canonical declaration authority must authorize it before production use.
fn validate_position_rows(
    params: &[Ty],
    result: &Ty,
    contract: &RefContract,
    budget: &PlanBudget,
) -> Result<(), String> {
    let rows = contract_rows(contract, budget)?;
    let mut visits = 0;
    ref_type_walk(result, true, budget, 0, &mut visits)?;
    for param in params {
        ref_type_walk(param, true, budget, 0, &mut visits)?;
    }
    let erased = contract.region_count == 0
        && contract.input_regions.is_empty()
        && contract.results.is_empty()
        && contract.outlives.is_empty();
    if erased && reference_layers(result)? == 0 {
        return Ok(());
    }
    let mut cursor = 0;
    for (param, ty) in params.iter().enumerate() {
        for depth in 0..reference_layers(ty)? {
            budget.charge_work(1)?;
            let input = InputReference {
                param,
                position: ReferencePosition { depth: depth as u8 },
            };
            let row = contract.input_regions.get(cursor).ok_or(
                "invalid noescape reference contract: missing reference parameter region position",
            )?;
            if row.input != input {
                return Err("invalid noescape reference contract: missing reference parameter region at expected position".into());
            }
            if depth != 0 {
                let outer = &contract.input_regions[cursor - 1];
                if rows[usize::from(row.region.0)] & (1u64 << outer.region.0) == 0 {
                    return Err("nested reference requires inner outlives outer".into());
                }
            }
            cursor += 1;
        }
    }
    if cursor != contract.input_regions.len() {
        return Err("extra reference parameter region position".into());
    }
    let layers = reference_layers(result)?;
    if contract.results.len() != layers {
        return Err(
            "invalid noescape reference contract: reference result position coverage mismatch"
                .into(),
        );
    }
    for (depth, row) in contract.results.iter().enumerate() {
        if usize::from(row.position.depth) != depth {
            return Err(
                "invalid noescape reference contract: reference result position coverage mismatch"
                    .into(),
            );
        }
        if depth != 0
            && rows[usize::from(row.region.0)] & (1u64 << contract.results[depth - 1].region.0) == 0
        {
            return Err("nested reference requires inner outlives outer".into());
        }
    }
    Ok(())
}

/// Derive declaration authority only on rigid schemas, never on an instance.
pub fn derive_return_contracts(
    params: &[Ty],
    result: &Ty,
    regions: &RefContract,
    budget: &PlanBudget,
) -> Result<Arc<[ResultReference]>, String> {
    validate_position_rows(params, result, regions, budget)?;
    let rows = contract_rows(regions, budget)?;
    budget.retain_bytes(
        regions.results.len() * 2 * std::mem::size_of::<ResultReference>()
            + 4 * std::mem::size_of::<usize>(),
    )?;
    let mut results = Vec::new();
    results
        .try_reserve_exact(regions.results.len())
        .map_err(|_| "reference result allocation failed")?;
    let mut counts = (0usize, 0usize, 0usize);
    for result_row in regions.results.iter() {
        let (output_kind, output) = reference_at(result, result_row.position)?;
        let _scratch = budget.reserve_bytes(
            regions
                .input_regions
                .len()
                .checked_mul(2 * std::mem::size_of::<InputReference>())
                .ok_or("input candidate storage overflow")?,
        )?;
        let mut eligible = Vec::new();
        eligible
            .try_reserve_exact(regions.input_regions.len().min(MAX_INPUT_PATHS))
            .map_err(|_| "input candidate allocation failed")?;
        let mut visits = 0;
        let mut opaque = ref_type_walk(output, true, budget, 0, &mut visits)?;
        for row in regions.input_regions.iter() {
            budget.charge_work(1)?;
            let (kind, referent) = reference_at(
                params
                    .get(row.input.param)
                    .ok_or("invalid input parameter")?,
                row.input.position,
            )?;
            if (output_kind == BorrowKind::Shared || kind == BorrowKind::Mut)
                && rows[usize::from(row.region.0)] & (1u64 << result_row.region.0) != 0
            {
                if eligible.len() == MAX_INPUT_PATHS {
                    return Err("reference return candidate limit exceeded".into());
                }
                eligible.push(row.input);
                opaque |= ref_type_walk(referent, true, budget, 0, &mut visits)?;
            }
        }
        let returns = if opaque {
            counts.2 = counts
                .2
                .checked_add(eligible.len())
                .ok_or("input candidate overflow")?;
            if counts.2 > MAX_INPUT_PATHS {
                return Err("reference return candidate limit exceeded".into());
            }
            budget.retain_bytes(
                eligible.len() * 2 * std::mem::size_of::<InputEnvelope>()
                    + 4 * std::mem::size_of::<usize>(),
            )?;
            let mut candidates = Vec::new();
            candidates
                .try_reserve_exact(eligible.len())
                .map_err(|_| "input envelope allocation failed")?;
            for input in eligible {
                candidates.push(InputEnvelope { input });
            }
            if output_kind == BorrowKind::Mut {
                ReturnContract::MutInputEnvelopes(candidates.into())
            } else {
                ReturnContract::SharedInputEnvelopes(candidates.into())
            }
        } else {
            let _path_scratch = budget
                .reserve_bytes(MAX_PATTERN_PROJECTION_DEPTH * std::mem::size_of::<Projection>())?;
            let mut path = Vec::new();
            path.try_reserve_exact(MAX_PATTERN_PROJECTION_DEPTH)
                .map_err(|_| "input path allocation failed")?;
            let mut candidates = Vec::new();
            for input in eligible {
                let (_, referent) = reference_at(&params[input.param], input.position)?;
                collect_input_paths(
                    referent,
                    output,
                    input,
                    &mut path,
                    &mut candidates,
                    &mut counts,
                    budget,
                )?;
            }
            if output_kind == BorrowKind::Mut {
                ReturnContract::MutInputPaths(candidates.into())
            } else {
                ReturnContract::SharedInputPaths(candidates.into())
            }
        };
        results.push(ResultReference {
            position: result_row.position,
            region: result_row.region,
            returns,
        });
    }
    Ok(results.into())
}

fn collect_input_paths(
    ty: &Ty,
    output: &Ty,
    input: InputReference,
    path: &mut Vec<Projection>,
    out: &mut Vec<InputPath>,
    counts: &mut (usize, usize, usize),
    budget: &PlanBudget,
) -> Result<(), String> {
    budget.check_depth(path.len())?;
    budget.charge_work(1)?;
    counts.1 += 1;
    if counts.1 > MAX_INPUT_PATH_OCCURRENCES {
        return Err("input path occurrence resource limit exceeded".into());
    }
    if ref_types_equal(ty, output, budget, 0)? {
        counts.2 = counts.2.checked_add(1).ok_or("input candidate overflow")?;
        counts.0 = counts
            .0
            .checked_add(path.len())
            .ok_or("input path storage overflow")?;
        if counts.2 > MAX_INPUT_PATHS {
            return Err("input path candidate limit exceeded".into());
        }
        if counts.0 > MAX_INPUT_PATH_STEPS {
            return Err("input path step resource limit exceeded".into());
        }
        budget.retain_bytes(
            3 * std::mem::size_of::<InputPath>()
                + path.len() * 2 * std::mem::size_of::<Projection>()
                + 4 * std::mem::size_of::<usize>(),
        )?;
        budget.charge_work(path.len())?;
        if out.len() == out.capacity() {
            // Geometric growth bounds total relocation work by the final size.
            // Charge each relocated row before the allocator can move it.
            budget.charge_work(out.len())?;
            let additional = out.capacity().max(1).min(MAX_INPUT_PATHS - out.len());
            out.try_reserve_exact(additional)
                .map_err(|_| "input path allocation failed")?;
        }
        out.push(InputPath {
            input,
            projection: path.clone().into(),
        });
    }
    let count = if let Ty::Array { len, .. } = ty {
        *len
    } else {
        ty.field_count().unwrap_or(0)
    };
    for index in 0..count {
        if path.len() >= MAX_PATTERN_PROJECTION_DEPTH {
            return Err("input path depth resource limit exceeded".into());
        }
        let (step, child) = if let Ty::Array { element, .. } = ty {
            (Projection::ConstantIndex(index), element.as_ref())
        } else {
            (
                Projection::Field(index),
                ty.field_ty(index).ok_or("invalid input field")?,
            )
        };
        path.push(step);
        collect_input_paths(child, output, input, path, out, counts, budget)?;
        path.pop();
    }
    Ok(())
}

/// Structural stored validation only, after canonical world subject agreement.
/// This does NOT rederive declaration origin authority after specialization.
pub(super) fn validate_stored_ref_contract(
    params: &[Ty],
    result: &Ty,
    contract: &RefContract,
    budget: &PlanBudget,
) -> Result<(), String> {
    validate_position_rows(params, result, contract, budget)?;
    stored_candidate_work(contract, budget)?;
    let rows = contract_rows(contract, budget)?;
    for result_row in contract.results.iter() {
        let (kind, output) = reference_at(result, result_row.position)?;
        let eligible = |input: InputReference| -> Result<&Ty, String> {
            budget.charge_work(
                1 + usize::BITS as usize - contract.input_regions.len().leading_zeros() as usize,
            )?;
            let (input_kind, referent) = reference_at(
                params
                    .get(input.param)
                    .ok_or("invalid return input index")?,
                input.position,
            )?;
            let index = contract
                .input_regions
                .binary_search_by_key(&input, |row| row.input)
                .map_err(|_| "missing reference input region")?;
            let region = contract.input_regions[index].region;
            if (kind == BorrowKind::Mut && input_kind != BorrowKind::Mut)
                || rows[usize::from(region.0)] & (1u64 << result_row.region.0) == 0
            {
                return Err("return input kind or region mismatch".into());
            }
            Ok(referent)
        };
        match &result_row.returns {
            ReturnContract::SharedInputPaths(paths) | ReturnContract::MutInputPaths(paths) => {
                if matches!(result_row.returns, ReturnContract::MutInputPaths(_))
                    != (kind == BorrowKind::Mut)
                {
                    return Err("reference return kind mismatch".into());
                }
                for path in paths.iter() {
                    let selected = projected_ty(eligible(path.input)?, &path.projection)?;
                    if !ref_types_equal(selected, output, budget, 0)? {
                        return Err("return input path type mismatch".into());
                    }
                }
            }
            ReturnContract::SharedInputEnvelopes(paths)
            | ReturnContract::MutInputEnvelopes(paths) => {
                if matches!(result_row.returns, ReturnContract::MutInputEnvelopes(_))
                    != (kind == BorrowKind::Mut)
                {
                    return Err("reference return kind mismatch".into());
                }
                let mut previous = None;
                for path in paths.iter() {
                    eligible(path.input)?;
                    if previous.is_some_and(|old| old >= path.input) {
                        return Err("noncanonical return envelope order".into());
                    }
                    previous = Some(path.input);
                }
            }
            ReturnContract::NoReference => {
                return Err("reference result requires return contract".into());
            }
        }
    }
    Ok(())
}

impl RefContract {
    /// Bounded structural convenience for fixtures, not a declaration proof.
    pub fn closure(&self) -> Result<Vec<u64>, String> {
        let budget = PlanBudget::new(PlanLimits {
            work: MAX_PATTERN_PROJECTION_WORK,
            ..PlanLimits::default()
        });
        let rows = contract_rows(self, &budget)?;
        Ok(rows[..usize::from(self.region_count)].to_vec())
    }
    pub fn outlives_region(
        &self,
        longer: RegionParamId,
        shorter: RegionParamId,
    ) -> Result<bool, String> {
        let rows = self.closure()?;
        if usize::from(longer.0) >= rows.len() || usize::from(shorter.0) >= rows.len() {
            return Err("invalid reference contract query region ID".into());
        }
        Ok(rows[usize::from(longer.0)] & (1u64 << shorter.0) != 0)
    }
    /// Legacy first-result exact-path convenience, used by structural fixtures.
    pub fn eligible_input_paths(
        &self,
        params: &[Ty],
        result: &Ty,
    ) -> Result<Vec<InputPath>, String> {
        let budget = PlanBudget::new(PlanLimits {
            work: MAX_PATTERN_PROJECTION_WORK,
            ..PlanLimits::default()
        });
        let expected = derive_return_contracts(params, result, self, &budget)?;
        match expected.first().map(|row| &row.returns) {
            Some(
                ReturnContract::SharedInputPaths(paths) | ReturnContract::MutInputPaths(paths),
            ) => Ok(paths.to_vec()),
            _ => Err("exact reference return contract requires reference result".into()),
        }
    }
    pub fn validate(&self, params: &[Ty], result: &Ty) -> Result<(), String> {
        self.validation_work(params, result).map(|_| ())
    }
    /// Legacy structural completeness, never used as production signature authority.
    pub fn validation_work(&self, params: &[Ty], result: &Ty) -> Result<usize, String> {
        let budget = PlanBudget::new(PlanLimits {
            work: MAX_PATTERN_PROJECTION_WORK,
            ..PlanLimits::default()
        });
        validate_stored_ref_contract(params, result, self, &budget)?;
        if self.results.iter().any(|row| {
            matches!(
                row.returns,
                ReturnContract::SharedInputEnvelopes(_) | ReturnContract::MutInputEnvelopes(_)
            )
        }) {
            return Err("envelope contract requires canonical signature authority".into());
        }
        let expected = derive_return_contracts(params, result, self, &budget)?;
        // The stored metadata scan above has charged every compared candidate/step.
        if self.results.as_ref() != expected.as_ref() {
            return Err(
                "reference return candidates must equal complete declaration-derived parameters"
                    .into(),
            );
        }
        let work = budget
            .usage
            .lock()
            .map_err(|_| "generic planning budget poisoned")?
            .work;
        Ok(work)
    }
}

#[derive(Clone, Debug)]
pub struct TypedExpr {
    /// Actual semantic type, independent of whether evaluation completes.
    /// For example, `{ return; 0 }` has type I32 and completion Never, while
    /// `{ return; }` has type Never and completion Never. Never is not a
    /// placeholder for every expression that cannot complete normally.
    pub ty: Ty,
    pub completion: Completion,
    pub kind: ExprKind,
}
impl TypedExpr {
    /// Normal-completion summary only, not a semantic type query. In particular,
    /// None discards the actual type of an unreachable but explicitly typed tail.
    /// Operator selection, coercion and inference must inspect `ty` separately.
    pub fn flow(&self) -> Option<Ty> {
        match self.completion {
            Completion::Returns => Some(self.ty.clone()),
            Completion::Never => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub tail: Option<ExprId>,
}

/// A resolved local-rooted storage location. Projections retain source order
/// and distinguish fields, variant payloads and literal array indices.
/// An empty projection denotes the whole local. No byte offsets or target ABI.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    pub local: LocalId,
    pub projection: Vec<Projection>,
}

pub const MAX_PATTERN_BINDINGS: usize = 65_536;
pub const MAX_PATTERN_PROJECTION_DEPTH: usize = 128;
pub const MAX_PATTERN_PROJECTION_WORK: usize = 1_000_000;

/// One binding from an irrefutable pattern. The projection is relative to the
/// initializer place, not an eagerly consumed whole-value snapshot. Ignored
/// wildcard/rest fields have no entry. Entries retain source pattern order.
#[derive(Clone, Debug)]
pub struct PatternBinding {
    pub local: LocalId,
    pub projection: Vec<Projection>,
}

fn pattern_types_equal(a: &Ty, b: &Ty, work: &mut usize) -> Result<bool, String> {
    let mut pending = vec![(a, b, 0)];
    while let Some((a, b, depth)) = pending.pop() {
        *work = work
            .checked_add(1)
            .ok_or("pattern projection work limit exceeded")?;
        if *work > MAX_PATTERN_PROJECTION_WORK {
            return Err("pattern projection work limit exceeded".into());
        }
        if depth > MAX_PATTERN_PROJECTION_DEPTH {
            return Err("pattern type comparison depth exceeds limit 128".into());
        }
        match (a, b) {
            (
                Ty::Ref {
                    kind: ak,
                    referent: a,
                },
                Ty::Ref {
                    kind: bk,
                    referent: b,
                },
            ) if ak == bk => {
                if !Arc::ptr_eq(a, b) {
                    pending.push((a, b, depth + 1));
                }
            }
            (Ty::Int(a), Ty::Int(b)) if a == b => {}
            (Ty::Param(a), Ty::Param(b)) if a == b => {}
            (Ty::Bool, Ty::Bool) | (Ty::Unit, Ty::Unit) | (Ty::Never, Ty::Never) => {}
            (Ty::Struct(a), Ty::Struct(b)) if a == b => {}
            (Ty::Enum(a), Ty::Enum(b)) if a == b => {}
            (
                Ty::Array {
                    element: a,
                    len: an,
                },
                Ty::Array {
                    element: b,
                    len: bn,
                },
            ) if an == bn => {
                if !Arc::ptr_eq(a, b) {
                    pending.push((a, b, depth + 1));
                }
            }
            (Ty::Tuple(a), Ty::Tuple(b)) if a.len() == b.len() => {
                if Arc::ptr_eq(a, b) {
                    continue;
                }
                let queued = pending
                    .len()
                    .checked_add(a.len())
                    .ok_or("pattern projection work limit exceeded")?;
                if queued > MAX_PATTERN_PROJECTION_WORK - *work {
                    return Err("pattern projection work limit exceeded".into());
                }
                pending.extend(a.iter().zip(b.iter()).map(|(a, b)| (a, b, depth + 1)));
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

#[derive(Clone, Debug)]
pub enum Stmt {
    AssignOp {
        place: Place,
        op: BinaryOp,
        value: ExprId,
    },
    AssignIndexOp {
        array: Place,
        index: ExprId,
        op: BinaryOp,
        value: ExprId,
    },
    AssignIndex {
        array: Place,
        index: ExprId,
        value: ExprId,
    },
    /// Starts a fresh binding's lifetime without initializing its value.
    Declare {
        local: LocalId,
    },
    Let {
        local: LocalId,
        value: ExprId,
    },
    LetPattern {
        value: ExprId,
        /// Fully resolved effective pattern type, including an annotation or
        /// concrete pattern hint when the initializer itself has type Never.
        ty: Ty,
        bindings: Vec<PatternBinding>,
    },
    Assign {
        local: LocalId,
        value: ExprId,
    },
    AssignPlace {
        place: Place,
        value: ExprId,
    },
    Expr(ExprId),
    Return(Option<ExprId>),
    While {
        id: LoopId,
        condition: ExprId,
        body: Block,
    },
}

fn validate_compound_types(op: BinaryOp, target: &Ty, value: &TypedExpr) -> Result<(), String> {
    let value_ty = &value.ty;
    let valid = match op {
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
            matches!(target, Ty::Int(_)) && target == value_ty
        }
        BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
            matches!(target, Ty::Int(_) | Ty::Bool) && target == value_ty
        }
        BinaryOp::Shl | BinaryOp::Shr => {
            if let (Ty::Int(integer), ExprKind::Int(count), Completion::Returns) =
                (target, &value.kind, value.completion)
                && count.bits >= u64::from(integer.bits())
            {
                return Err(
                    "constant compound shift count is negative or exceeds LHS width".into(),
                );
            }
            matches!(target, Ty::Int(_)) && matches!(value_ty, Ty::Int(_))
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("invalid compound assignment operator or operand types".into())
    }
}

#[derive(Clone, Debug)]
pub enum MatchPattern {
    Wildcard,
    Binding(LocalId),
    Bool(bool),
    Int(IntValue),
    Unit,
    EnumVariant {
        ty: EnumTy,
        variant: usize,
        bindings: Vec<PatternBinding>,
    },
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub body: ExprId,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    IntCast {
        value: ExprId,
        to: IntTy,
    },
    Borrow {
        kind: BorrowKind,
        place: Place,
        timing: BorrowTiming,
    },
    Deref {
        value: ExprId,
    },
    DynamicIndex {
        base: ExprId,
        index: ExprId,
    },
    Array(Vec<ExprId>),
    Repeat {
        value: ExprId,
        len: usize,
    },
    Index {
        base: ExprId,
        index: usize,
    },
    EnumVariant {
        ty: EnumTy,
        variant: usize,
        fields: Vec<(usize, ExprId)>,
    },
    Match {
        scrutinee: ExprId,
        arms: Vec<MatchArm>,
    },
    Int(IntValue),
    Bool(bool),
    /// Read a resolved local into this expression's independent value snapshot.
    Local(LocalId),
    /// Elements are evaluated left to right into a new value snapshot.
    Tuple(Vec<ExprId>),
    /// Initializers retain source evaluation order. Each index names a field in
    /// declaration order; lowering permutes snapshots, never source evaluation.
    Struct {
        ty: StructTy,
        fields: Vec<(usize, ExprId)>,
    },
    /// Projects from an arbitrary expression snapshot, not a live local alias.
    Field {
        base: ExprId,
        index: usize,
    },
    Unary {
        op: UnaryOp,
        value: ExprId,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Call {
        target: Callee,
        name: String,
        args: Vec<ExprId>,
        /// Validated at typing time, shared by call lowering and ABI selection.
        signature: Signature,
    },
    If {
        condition: ExprId,
        then_block: Block,
        else_block: Block,
    },
    Block(Block),
    /// Repeats the body until an associated break produces a value.
    Loop {
        id: LoopId,
        body: Block,
    },
    /// Evaluates the operand before transferring to the resolved target's exit.
    /// This expression itself never completes normally, even for a unit break.
    Break {
        target: LoopId,
        value: Option<ExprId>,
    },
    /// Transfers to the target's head, including condition evaluation for while.
    Continue {
        target: LoopId,
    },
    Println(String),
}

impl CheckedFunction {
    pub fn preflight_plan(&self, facts: FactsCtx<'_>, budget: &PlanBudget) -> Result<(), String> {
        fn storage(budget: &PlanBudget, count: usize, width: usize) -> Result<(), String> {
            budget.charge_work(count)?;
            budget.retain_bytes(
                count
                    .checked_mul(width)
                    .ok_or("generic planning allocation overflow")?,
            )
        }
        fn path(path: &[Projection], budget: &PlanBudget) -> Result<(), String> {
            budget.check_depth(path.len())?;
            storage(budget, path.len(), std::mem::size_of::<Projection>())
        }
        fn bindings(bindings: &[PatternBinding], budget: &PlanBudget) -> Result<(), String> {
            storage(
                budget,
                bindings.len(),
                std::mem::size_of::<PatternBinding>(),
            )?;
            for binding in bindings {
                path(&binding.projection, budget)?;
            }
            Ok(())
        }
        fn block(
            value: &Block,
            facts: FactsCtx<'_>,
            budget: &PlanBudget,
            depth: usize,
        ) -> Result<(), String> {
            budget.check_depth(depth)?;
            budget.charge_nodes(value.statements.len())?;
            storage(budget, value.statements.len(), std::mem::size_of::<Stmt>())?;
            for statement in &value.statements {
                match statement {
                    Stmt::AssignOp { place, .. } | Stmt::AssignPlace { place, .. } => {
                        path(&place.projection, budget)?
                    }
                    Stmt::AssignIndexOp { array, .. } | Stmt::AssignIndex { array, .. } => {
                        path(&array.projection, budget)?
                    }
                    Stmt::LetPattern {
                        ty,
                        bindings: values,
                        ..
                    } => {
                        facts.validate_ty_with_budget(ty, budget)?;
                        bindings(values, budget)?;
                    }
                    Stmt::While { body, .. } => block(body, facts, budget, depth + 1)?,
                    Stmt::Declare { .. }
                    | Stmt::Let { .. }
                    | Stmt::Assign { .. }
                    | Stmt::Expr(_)
                    | Stmt::Return(_) => {}
                }
            }
            Ok(())
        }
        facts.validate_ty_with_budget(&self.result, budget)?;
        budget.charge_nodes(
            self.locals
                .len()
                .checked_add(self.exprs.len())
                .ok_or("generic planning node overflow")?,
        )?;
        storage(budget, self.locals.len(), std::mem::size_of::<LocalDecl>())?;
        storage(budget, self.exprs.len(), std::mem::size_of::<TypedExpr>())?;
        storage(budget, self.params.len(), std::mem::size_of::<LocalId>())?;
        storage(budget, self.name.len(), 1)?;
        for local in &self.locals {
            facts.validate_ty_with_budget(&local.ty, budget)?;
            storage(budget, local.name.len(), 1)?;
        }
        for expression in &self.exprs {
            facts.validate_ty_with_budget(&expression.ty, budget)?;
            match &expression.kind {
                ExprKind::Borrow { place, .. } => path(&place.projection, budget)?,
                ExprKind::Struct { ty, fields } => {
                    facts.validate_ty_with_budget(&Ty::Struct(ty.clone()), budget)?;
                    storage(budget, fields.len(), std::mem::size_of::<(usize, ExprId)>())?;
                }
                ExprKind::EnumVariant { ty, fields, .. } => {
                    facts.validate_ty_with_budget(&Ty::Enum(ty.clone()), budget)?;
                    storage(budget, fields.len(), std::mem::size_of::<(usize, ExprId)>())?;
                }
                ExprKind::Match { arms, .. } => {
                    storage(budget, arms.len(), std::mem::size_of::<MatchArm>())?;
                    for arm in arms {
                        if let MatchPattern::EnumVariant {
                            ty,
                            bindings: values,
                            ..
                        } = &arm.pattern
                        {
                            facts.validate_ty_with_budget(&Ty::Enum(ty.clone()), budget)?;
                            bindings(values, budget)?;
                        }
                    }
                }
                ExprKind::Call {
                    target,
                    signature,
                    args,
                    name,
                } => {
                    storage(budget, name.len(), 1)?;
                    storage(budget, args.len(), std::mem::size_of::<ExprId>())?;
                    storage(budget, signature.params.len(), std::mem::size_of::<Ty>())?;
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
                }
                ExprKind::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    block(then_block, facts, budget, 0)?;
                    block(else_block, facts, budget, 0)?;
                }
                ExprKind::Block(value) | ExprKind::Loop { body: value, .. } => {
                    block(value, facts, budget, 0)?
                }
                ExprKind::Tuple(values) | ExprKind::Array(values) => {
                    storage(budget, values.len(), std::mem::size_of::<ExprId>())?
                }
                ExprKind::Println(text) => storage(budget, text.len(), 1)?,
                ExprKind::IntCast { .. }
                | ExprKind::Deref { .. }
                | ExprKind::DynamicIndex { .. }
                | ExprKind::Repeat { .. }
                | ExprKind::Index { .. }
                | ExprKind::Int(_)
                | ExprKind::Bool(_)
                | ExprKind::Local(_)
                | ExprKind::Field { .. }
                | ExprKind::Unary { .. }
                | ExprKind::Binary { .. }
                | ExprKind::Break { .. }
                | ExprKind::Continue { .. } => {}
            }
        }
        block(&self.body, facts, budget, 0)
    }
    pub(super) fn validate_match(
        &self,
        scrutinee: ExprId,
        arms: &[MatchArm],
        parameters: &std::collections::HashSet<LocalId>,
    ) -> Result<(), String> {
        let ty = &self
            .exprs
            .get(scrutinee.0)
            .ok_or("invalid match scrutinee")?
            .ty;
        if arms.len() > MAX_MATCH_ARMS || (arms.is_empty() && *ty != Ty::Never) {
            return Err(
                "match arm count must be between 1 and 4096 unless scrutinee is never".into(),
            );
        }
        if let Ty::Enum(ty) = ty {
            ty.validate()?;
        }
        let mut bindings = std::collections::HashSet::new();
        for arm in arms {
            self.exprs.get(arm.body.0).ok_or("invalid match arm body")?;
            let valid = match &arm.pattern {
                MatchPattern::Wildcard => true,
                MatchPattern::Binding(local) => {
                    let decl = self
                        .locals
                        .get(local.0)
                        .ok_or("invalid match binding local")?;
                    if parameters.contains(local) || !bindings.insert(*local) {
                        return Err("match bindings must be distinct fresh locals".into());
                    }
                    let mut work = 0;
                    *ty == Ty::Never || pattern_types_equal(ty, &decl.ty, &mut work)?
                }
                MatchPattern::Bool(_) => matches!(ty, Ty::Bool | Ty::Never),
                MatchPattern::Int(value) => {
                    value.validate()?;
                    *ty == Ty::Int(value.ty) || *ty == Ty::Never
                }
                MatchPattern::Unit => matches!(ty, Ty::Unit | Ty::Never),
                MatchPattern::EnumVariant {
                    ty: pattern_ty,
                    variant,
                    bindings: payload_bindings,
                } => {
                    pattern_ty.validate()?;
                    self.validate_pattern_bindings_with_params(
                        &Ty::Enum(pattern_ty.clone()),
                        payload_bindings,
                        parameters,
                    )?;
                    let expected = Projection::Downcast(VariantId {
                        owner: pattern_ty.def().id,
                        index: *variant,
                    });
                    for binding in payload_bindings {
                        if binding.projection.first() != Some(&expected)
                            || !bindings.insert(binding.local)
                        {
                            return Err("invalid or repeated enum payload binding".into());
                        }
                    }
                    *variant < pattern_ty.def().variants.len()
                        && (*ty == Ty::Never || *ty == Ty::Enum(pattern_ty.clone()))
                }
            };
            if !valid {
                return Err("match pattern type or variant mismatch".into());
            }
        }
        Ok(())
    }

    /// Bound and check the flattened plan without revisiting source patterns,
    /// visibility, names or inference. No overlapping bindings are admitted by
    /// this slice, which deliberately excludes `@` patterns.
    #[cfg(test)]
    fn validate_pattern_bindings(
        &self,
        ty: &Ty,
        bindings: &[PatternBinding],
    ) -> Result<(), String> {
        if bindings.is_empty() {
            return Ok(());
        }
        let parameters = self.params.iter().copied().collect();
        self.validate_pattern_bindings_with_params(ty, bindings, &parameters)
    }

    pub(super) fn validate_pattern_bindings_with_params(
        &self,
        ty: &Ty,
        bindings: &[PatternBinding],
        parameters: &std::collections::HashSet<LocalId>,
    ) -> Result<(), String> {
        if bindings.len() > MAX_PATTERN_BINDINGS {
            return Err("pattern binding limit exceeded".into());
        }
        let mut work = bindings.len();
        let mut locals = std::collections::HashSet::new();
        let mut paths = Vec::with_capacity(bindings.len());
        for binding in bindings {
            if binding.projection.len() > MAX_PATTERN_PROJECTION_DEPTH {
                return Err("pattern projection depth exceeds limit 128".into());
            }
            work = work
                .checked_add(binding.projection.len())
                .ok_or("pattern projection work limit exceeded")?;
            if work > MAX_PATTERN_PROJECTION_WORK {
                return Err("pattern projection work limit exceeded".into());
            }
            let local = self
                .locals
                .get(binding.local.0)
                .ok_or("invalid pattern binding local")?;
            if !locals.insert(binding.local) || parameters.contains(&binding.local) {
                return Err("pattern binding locals must be distinct fresh locals".into());
            }
            let projected = projected_ty(ty, &binding.projection)?;
            if !pattern_types_equal(&local.ty, projected, &mut work)? {
                return Err("pattern binding local type mismatch".into());
            }
            paths.push(binding.projection.as_slice());
        }
        paths.sort_unstable();
        if paths.windows(2).any(|pair| pair[1].starts_with(pair[0])) {
            return Err("overlapping pattern binding projections".into());
        }
        Ok(())
    }

    /// Optional structural debugging check. This is not source type checking,
    /// borrow checking, or a codegen prerequisite. Normal compilation consumes
    /// the immutable result produced by the checker without revalidating it.
    ///
    /// Checks reference bounds, distinct parameters, call arity and flattened
    /// pattern plan consistency. Deliberately
    /// does not establish lexical scope, reachability, expression flow, or loop
    /// target validity or projection typing. Those are established once by
    /// semantic analysis.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_with_facts(FactsCtx::Concrete)
    }
    pub fn validate_with_facts(&self, facts: FactsCtx<'_>) -> Result<(), String> {
        facts.validate_ty(&self.result)?;
        for local in &self.locals {
            facts.validate_ty(&local.ty)?;
        }
        for expression in &self.exprs {
            facts.validate_ty(&expression.ty)?;
        }
        let local = |id: LocalId| {
            if id.0 < self.locals.len() {
                Ok(())
            } else {
                Err(format!("invalid local {} in {}", id.0, self.name))
            }
        };
        let expr = |id: ExprId| {
            if id.0 < self.exprs.len() {
                Ok(())
            } else {
                Err(format!("invalid expression {} in {}", id.0, self.name))
            }
        };
        let mut parameters = std::collections::HashSet::new();
        for &id in &self.params {
            local(id)?;
            if !parameters.insert(id) {
                return Err(format!(
                    "duplicate parameter local {} in {}",
                    id.0, self.name
                ));
            }
        }
        if self.params.len() >= MAX_PATTERN_PROJECTION_WORK {
            return Err("reference contract parameter resource limit exceeded".into());
        }
        let parameter_types: Vec<_> = self
            .params
            .iter()
            .map(|id| self.locals[id.0].ty.clone())
            .collect();
        if let Some(authority) = facts.validated() {
            validate_body_signature(
                &Signature {
                    ref_contract: self.ref_contract.clone(),
                    params: parameter_types.clone(),
                    result: self.result.clone(),
                },
                authority,
            )?;
        }
        if facts.validated().is_none() {
            self.ref_contract.validate(&parameter_types, &self.result)?;
        }
        // Traverse nested blocks iteratively so malformed/deep fixtures cannot
        // exhaust the host call stack inside this diagnostic helper.
        let mut blocks = vec![&self.body];
        for (borrow_index, value) in self.exprs.iter().enumerate() {
            match &value.kind {
                ExprKind::IntCast { value: source, to } => {
                    expr(*source)?;
                    let source = &self.exprs[source.0];
                    if !matches!(source.ty, Ty::Int(_)) {
                        return Err("integer cast requires integer source".into());
                    }
                    if value.ty != Ty::Int(*to) {
                        return Err("integer cast result type mismatch".into());
                    }
                    if value.completion != source.completion {
                        return Err("integer cast completion mismatch".into());
                    }
                }
                ExprKind::Borrow {
                    kind,
                    place,
                    timing,
                } => {
                    let root = &self
                        .locals
                        .get(place.local.0)
                        .ok_or("invalid borrow local")?
                        .ty;
                    let referent = projected_ty(root, &place.projection)?;
                    let expected = Ty::Ref {
                        kind: *kind,
                        referent: Arc::new(referent.clone()),
                    };
                    if !pattern_types_equal(&value.ty, &expected, &mut 0)? {
                        return Err("borrow result type mismatch".into());
                    }
                    if let BorrowTiming::TwoPhase {
                        activation,
                        argument,
                        reason,
                    } = timing
                    {
                        if *kind != BorrowKind::Mut {
                            return Err("two-phase borrow must be mutable".into());
                        }
                        let call = self
                            .exprs
                            .get(activation.0)
                            .ok_or("invalid borrow activation expression")?;
                        let ExprKind::Call {
                            target,
                            signature,
                            args,
                            ..
                        } = &call.kind
                        else {
                            return Err("borrow activation must be a call".into());
                        };
                        if args.get(*argument) != Some(&ExprId(borrow_index))
                            || signature.params.get(*argument) != Some(&expected)
                        {
                            return Err(
                                "two-phase borrow must be the exact mutable call argument".into()
                            );
                        }
                        match reason {
                            ImplicitBorrowKind::MutableReceiver => {
                                let authority = facts
                                    .validated()
                                    .ok_or("mutable receiver requires canonical authority")?;
                                if *argument != 0
                                    || authority
                                        .service()
                                        .callee_receiver(target, authority)
                                        .map_err(|e| e.into_legacy_diagnostic(authority.budget()))?
                                        != (ReceiverMode::Borrowed {
                                            kind: BorrowKind::Mut,
                                        })
                                {
                                    return Err(
                                        "two-phase receiver is not a canonical mutable receiver"
                                            .into(),
                                    );
                                }
                            }
                            ImplicitBorrowKind::MutableArgumentReborrow { carrier } => {
                                let carrier = self
                                    .exprs
                                    .get(carrier.0)
                                    .ok_or("invalid reborrow carrier")?;
                                let ExprKind::Local(local) = carrier.kind else {
                                    return Err(
                                        "implicit mutable reborrow requires original local carrier"
                                            .into(),
                                    );
                                };
                                if !matches!(
                                    carrier.ty,
                                    Ty::Ref {
                                        kind: BorrowKind::Mut,
                                        ..
                                    }
                                ) || place.local != local
                                    || place.projection.as_slice() != [Projection::Deref]
                                {
                                    return Err(
                                        "implicit mutable reborrow carrier provenance mismatch"
                                            .into(),
                                    );
                                }
                            }
                        }
                    }
                }
                ExprKind::Deref { value: source } => {
                    expr(*source)?;
                    let Ty::Ref { referent, .. } = &self.exprs[source.0].ty else {
                        return Err("dereference requires reference type".into());
                    };
                    if !pattern_types_equal(&value.ty, referent, &mut 0)? {
                        return Err("dereference result type mismatch".into());
                    }
                }
                ExprKind::DynamicIndex { base, index } => {
                    expr(*base)?;
                    expr(*index)?;
                    if self.exprs[index.0].ty != Ty::Int(IntTy::Usize)
                        && self.exprs[index.0].ty != Ty::Never
                    {
                        return Err("dynamic array index requires usize".into());
                    }
                    let Ty::Array { element, len } = &self.exprs[base.0].ty else {
                        return Err("dynamic indexing requires array type".into());
                    };
                    if *len > MAX_ARRAY_LEN {
                        return Err("array length exceeds limit 4096".into());
                    }
                    let mut work = 0;
                    if !pattern_types_equal(&value.ty, element, &mut work)? {
                        return Err("dynamic index result type mismatch".into());
                    }
                }
                ExprKind::Array(elements) => {
                    let Ty::Array { len, .. } = &value.ty else {
                        return Err("array expression requires array type".into());
                    };
                    if *len > MAX_ARRAY_LEN || elements.len() != *len {
                        return Err("array expression length mismatch".into());
                    }
                    for &id in elements {
                        expr(id)?;
                    }
                }
                ExprKind::Repeat {
                    value: operand,
                    len,
                } => {
                    expr(*operand)?;
                    let Ty::Array {
                        element,
                        len: count,
                    } = &value.ty
                    else {
                        return Err("array repeat requires array type".into());
                    };
                    if len != count
                        || *len > MAX_ARRAY_LEN
                        || (*len > 1 && !facts.must_copy(element)?)
                    {
                        return Err("invalid array repeat type, length or Copy requirement".into());
                    }
                }
                ExprKind::Index { base, index } => {
                    expr(*base)?;
                    projected_ty(&self.exprs[base.0].ty, &[Projection::ConstantIndex(*index)])?;
                }
                ExprKind::EnumVariant {
                    ty,
                    variant,
                    fields,
                } => {
                    ty.validate()?;
                    if *variant >= ty.def().variants.len() || value.ty != Ty::Enum(ty.clone()) {
                        return Err("invalid enum variant expression".into());
                    }
                    let def = &ty.def().variants[*variant];
                    if fields.len() != def.fields.len() {
                        return Err("enum initializer field count mismatch".into());
                    }
                    let mut seen = std::collections::HashSet::new();
                    for &(index, value) in fields {
                        if index >= def.fields.len() || !seen.insert(index) {
                            return Err("invalid enum initializer field".into());
                        }
                        expr(value)?;
                    }
                }
                ExprKind::Match { scrutinee, arms } => {
                    expr(*scrutinee)?;
                    self.validate_match(*scrutinee, arms, &parameters)?;
                    for arm in arms {
                        expr(arm.body)?;
                    }
                }
                ExprKind::Local(id) => local(*id)?,
                ExprKind::Tuple(elements) => {
                    for &id in elements {
                        expr(id)?;
                    }
                }
                ExprKind::Struct { ty, fields } => {
                    facts.validate_ty(&Ty::Struct(ty.clone()))?;
                    if value.ty != Ty::Struct(ty.clone()) {
                        return Err("struct expression type mismatch".into());
                    }
                    for &(_, id) in fields {
                        expr(id)?;
                    }
                }
                ExprKind::Field { base, .. } => expr(*base)?,
                ExprKind::Unary { value, .. } => expr(*value)?,
                ExprKind::Binary { op, left, right } => {
                    expr(*left)?;
                    expr(*right)?;
                    if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
                        let left = &self.exprs[left.0];
                        let right = &self.exprs[right.0];
                        if !matches!(left.ty, Ty::Int(_))
                            || !matches!(right.ty, Ty::Int(_))
                            || value.ty != left.ty
                        {
                            return Err(
                                "shift requires integer operands and exact LHS result type".into(),
                            );
                        }
                        let completion = if left.completion == Completion::Returns
                            && right.completion == Completion::Returns
                        {
                            Completion::Returns
                        } else {
                            Completion::Never
                        };
                        if value.completion != completion {
                            return Err("shift completion mismatch".into());
                        }
                    }
                    if matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor) {
                        let left = &self.exprs[left.0];
                        let right = &self.exprs[right.0];
                        if !matches!(left.ty, Ty::Int(_) | Ty::Bool)
                            || left.ty != right.ty
                            || value.ty != left.ty
                        {
                            return Err(
                                "bitwise operator requires identical integer or bool types".into(),
                            );
                        }
                        let completion = if left.completion == Completion::Returns
                            && right.completion == Completion::Returns
                        {
                            Completion::Returns
                        } else {
                            Completion::Never
                        };
                        if value.completion != completion {
                            return Err("bitwise operator completion mismatch".into());
                        }
                    }
                }
                ExprKind::Call {
                    target,
                    name,
                    args,
                    signature,
                } => {
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
                    validate_callee(target, signature, facts)?;
                    if facts.validated().is_none() {
                        signature
                            .ref_contract
                            .validate(&signature.params, &signature.result)?;
                    }
                    if args.len() != signature.params.len() {
                        return Err(format!("invalid call arity for {name} in {}", self.name));
                    }
                    for &id in args {
                        expr(id)?;
                    }
                }
                ExprKind::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    expr(*condition)?;
                    blocks.push(then_block);
                    blocks.push(else_block);
                }
                ExprKind::Block(block) | ExprKind::Loop { body: block, .. } => blocks.push(block),
                ExprKind::Break {
                    value: Some(value), ..
                } => expr(*value)?,
                ExprKind::Break { value: None, .. } | ExprKind::Continue { .. } => {}
                ExprKind::Int(integer) => {
                    integer.validate()?;
                    if value.ty != Ty::Int(integer.ty) {
                        return Err("integer expression type mismatch".into());
                    }
                }
                ExprKind::Bool(_) | ExprKind::Println(_) => {}
            }
        }
        while let Some(block) = blocks.pop() {
            if let Some(id) = block.tail {
                expr(id)?;
            }
            for statement in &block.statements {
                match statement {
                    Stmt::AssignOp { place, op, value } => {
                        local(place.local)?;
                        expr(*value)?;
                        let target =
                            projected_ty(&self.locals[place.local.0].ty, &place.projection)?;
                        validate_compound_types(*op, target, &self.exprs[value.0])?;
                    }
                    Stmt::AssignIndexOp {
                        array,
                        index,
                        op,
                        value,
                    } => {
                        local(array.local)?;
                        expr(*index)?;
                        expr(*value)?;
                        if array.projection.contains(&Projection::Deref) {
                            return Err(
                                "compound runtime indexing through dereference is unsupported"
                                    .into(),
                            );
                        }
                        let array_ty =
                            projected_ty(&self.locals[array.local.0].ty, &array.projection)?;
                        let Ty::Array { element, .. } = array_ty else {
                            return Err("compound indexed assignment requires array type".into());
                        };
                        if self.exprs[index.0].ty != Ty::Int(IntTy::Usize) {
                            return Err("compound array index requires usize".into());
                        }
                        validate_compound_types(*op, element, &self.exprs[value.0])?;
                    }
                    Stmt::AssignIndex {
                        array,
                        index,
                        value,
                    } => {
                        local(array.local)?;
                        expr(*index)?;
                        expr(*value)?;
                        let array_ty =
                            projected_ty(&self.locals[array.local.0].ty, &array.projection)?;
                        if !matches!(array_ty, Ty::Array { .. }) {
                            return Err("dynamic indexed assignment requires array type".into());
                        }
                        if self.exprs[index.0].ty != Ty::Int(IntTy::Usize)
                            && self.exprs[index.0].ty != Ty::Never
                        {
                            return Err("dynamic array index requires usize".into());
                        }
                    }
                    Stmt::Declare { local: id } => {
                        local(*id)?;
                        if parameters.contains(id) {
                            return Err("declaration local aliases parameter".into());
                        }
                    }
                    Stmt::Let { local: id, value } | Stmt::Assign { local: id, value } => {
                        local(*id)?;
                        expr(*value)?;
                    }
                    Stmt::LetPattern {
                        value,
                        ty,
                        bindings,
                    } => {
                        expr(*value)?;
                        self.validate_pattern_bindings_with_params(ty, bindings, &parameters)?;
                    }
                    Stmt::AssignPlace { place, value } => {
                        local(place.local)?;
                        expr(*value)?;
                    }
                    Stmt::Expr(id) | Stmt::Return(Some(id)) => expr(*id)?,
                    Stmt::Return(None) => {}
                    Stmt::While {
                        condition, body, ..
                    } => {
                        expr(*condition)?;
                        blocks.push(body);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nested_reference_positions_normalize_and_reject_malformed_rows() {
        let budget = PlanBudget::default();
        let shared = |ty| Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let outer = ReferencePosition { depth: 0 };
        let inner = ReferencePosition { depth: 1 };
        let input = |position| InputReference { param: 0, position };
        let raw = RefContract {
            region_count: 2,
            input_regions: Arc::from([
                InputRegion {
                    input: input(outer),
                    region: RegionParamId(0),
                },
                InputRegion {
                    input: input(inner),
                    region: RegionParamId(1),
                },
            ]),
            results: Arc::from([ResultReference {
                position: outer,
                region: RegionParamId(1),
                returns: ReturnContract::NoReference,
            }]),
            outlives: Arc::from([]),
        };
        let mut contract = normalize_ref_contract(&raw, &budget)
            .unwrap()
            .as_ref()
            .clone();
        assert!(
            contract
                .outlives_region(RegionParamId(1), RegionParamId(0))
                .unwrap()
        );
        assert!(
            !contract
                .outlives_region(RegionParamId(0), RegionParamId(1))
                .unwrap()
        );
        let params = [shared(shared(Ty::Int(IntTy::I32)))];
        let result = shared(Ty::Int(IntTy::I32));
        contract.results = derive_return_contracts(&params, &result, &contract, &budget).unwrap();
        assert_eq!(
            contract.results[0].returns,
            ReturnContract::SharedInputPaths(Arc::from([InputPath {
                input: input(inner),
                projection: Arc::from([])
            }]))
        );
        validate_stored_ref_contract(&params, &result, &contract, &budget).unwrap();
        assert_eq!(
            reference_at(&params[0], inner).unwrap().1,
            &Ty::Int(IntTy::I32)
        );
        assert_eq!(
            projected_ty(&params[0], &[Projection::Deref, Projection::Deref]).unwrap(),
            &Ty::Int(IntTy::I32)
        );
        for rows in [
            vec![contract.input_regions[0].clone()],
            vec![
                contract.input_regions[1].clone(),
                contract.input_regions[0].clone(),
            ],
            vec![
                contract.input_regions[0].clone(),
                contract.input_regions[0].clone(),
            ],
        ] {
            let malformed = RefContract {
                input_regions: rows.into(),
                ..contract.clone()
            };
            assert!(validate_stored_ref_contract(&params, &result, &malformed, &budget).is_err());
        }
        let missing_wf = RefContract {
            outlives: Arc::from([]),
            ..contract.clone()
        };
        assert!(validate_stored_ref_contract(&params, &result, &missing_wf, &budget).is_err());
        let mut wrong = contract.clone();
        Arc::make_mut(&mut wrong.results)[0].position = inner;
        assert!(validate_stored_ref_contract(&params, &result, &wrong, &budget).is_err());
        Arc::make_mut(&mut wrong.results)[0].position = outer;
        Arc::make_mut(&mut wrong.results)[0].returns = ReturnContract::NoReference;
        assert!(validate_stored_ref_contract(&params, &result, &wrong, &budget).is_err());
        assert!(reference_layers(&shared(shared(shared(Ty::Bool)))).is_err());
        assert!(
            reference_layers(&shared(Ty::Ref {
                kind: BorrowKind::Mut,
                referent: Arc::new(Ty::Bool)
            }))
            .is_err()
        );
        let tiny = PlanBudget::new(PlanLimits {
            work: 1,
            ..PlanLimits::default()
        });
        assert!(normalize_ref_contract(&raw, &tiny).is_err());
    }

    #[test]
    fn nested_reference_result_rows_preserve_rigid_envelopes_and_combined_caps() {
        let budget = PlanBudget::default();
        let opaque = Ty::Param(ParamId {
            owner: BinderId {
                declaration: test_decl(),
            },
            index: 0,
        });
        let shared = |ty| Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let input = |depth| InputReference {
            param: 0,
            position: ReferencePosition { depth },
        };
        let raw = RefContract {
            region_count: 2,
            input_regions: Arc::from([
                InputRegion {
                    input: input(0),
                    region: RegionParamId(0),
                },
                InputRegion {
                    input: input(1),
                    region: RegionParamId(1),
                },
            ]),
            results: Arc::from([
                ResultReference {
                    position: ReferencePosition { depth: 0 },
                    region: RegionParamId(0),
                    returns: ReturnContract::NoReference,
                },
                ResultReference {
                    position: ReferencePosition { depth: 1 },
                    region: RegionParamId(1),
                    returns: ReturnContract::NoReference,
                },
            ]),
            outlives: Arc::from([]),
        };
        let mut contract = normalize_ref_contract(&raw, &budget)
            .unwrap()
            .as_ref()
            .clone();
        let template = shared(shared(opaque));
        contract.results = derive_return_contracts(
            std::slice::from_ref(&template),
            &template,
            &contract,
            &budget,
        )
        .unwrap();
        assert_eq!(
            contract.results[0].returns,
            ReturnContract::SharedInputEnvelopes(Arc::from([
                InputEnvelope { input: input(0) },
                InputEnvelope { input: input(1) }
            ]))
        );
        assert_eq!(
            contract.results[1].returns,
            ReturnContract::SharedInputEnvelopes(Arc::from([InputEnvelope { input: input(1) }]))
        );
        let concrete = shared(shared(Ty::Int(IntTy::I32)));
        validate_stored_ref_contract(
            std::slice::from_ref(&concrete),
            &concrete,
            &contract,
            &budget,
        )
        .unwrap();
        assert_ne!(
            derive_return_contracts(
                std::slice::from_ref(&concrete),
                &concrete,
                &contract,
                &budget
            )
            .unwrap(),
            contract.results
        );
        let mut oversized = contract.clone();
        Arc::make_mut(&mut oversized.results)[0].returns = ReturnContract::SharedInputEnvelopes(
            vec![InputEnvelope { input: input(0) }; MAX_INPUT_PATHS].into(),
        );
        assert!(
            validate_stored_ref_contract(
                std::slice::from_ref(&concrete),
                &concrete,
                &oversized,
                &budget
            )
            .unwrap_err()
            .contains("candidate limit")
        );
    }

    fn fixture_input(param: usize) -> super::InputReference {
        super::InputReference {
            param,
            position: super::ReferencePosition { depth: 0 },
        }
    }
    fn fixture_input_regions(
        regions: std::sync::Arc<[Option<super::RegionParamId>]>,
    ) -> std::sync::Arc<[super::InputRegion]> {
        regions
            .iter()
            .enumerate()
            .filter_map(|(param, region)| {
                region.map(|region| super::InputRegion {
                    input: fixture_input(param),
                    region,
                })
            })
            .collect::<Vec<_>>()
            .into()
    }
    fn fixture_parameter_regions(
        contract: &super::RefContract,
    ) -> Vec<Option<super::RegionParamId>> {
        contract
            .input_regions
            .iter()
            .map(|row| Some(row.region))
            .collect()
    }
    fn fixture_result_regions(
        region: Option<super::RegionParamId>,
        returns: super::ReturnContract,
    ) -> std::sync::Arc<[super::ResultReference]> {
        region
            .map(|region| super::ResultReference {
                position: super::ReferencePosition { depth: 0 },
                region,
                returns,
            })
            .into_iter()
            .collect::<Vec<_>>()
            .into()
    }
    fn fixture_set_returns(contract: &mut super::RefContract, returns: super::ReturnContract) {
        if let Some(row) = std::sync::Arc::make_mut(&mut contract.results).first_mut() {
            row.returns = returns;
        } else {
            assert!(matches!(returns, super::ReturnContract::NoReference));
        }
    }
    fn fixture_derive_return_contract(
        params: &[super::Ty],
        result: &super::Ty,
        contract: &super::RefContract,
        budget: &super::PlanBudget,
    ) -> Result<super::ReturnContract, String> {
        let rows = super::derive_return_contracts(params, result, contract, budget)?;
        Ok(rows
            .first()
            .map_or(super::ReturnContract::NoReference, |row| {
                row.returns.clone()
            }))
    }

    #[test]
    fn generic_reference_regions_normalize_mediators_and_preserve_named_noescape() {
        use super::*;
        let budget = PlanBudget::default();
        let mut raw = RefContract {
            results: fixture_result_regions(
                Some(RegionParamId(0)),
                ReturnContract::SharedInputPaths(Arc::from([])),
            ),
            input_regions: fixture_input_regions(Arc::from([
                Some(RegionParamId(2)),
                Some(RegionParamId(2)),
            ])),
            region_count: 4,
            outlives: Arc::from([
                (RegionParamId(2), RegionParamId(3)),
                (RegionParamId(3), RegionParamId(0)),
            ]),
        };
        let normalized = normalize_ref_contract(&raw, &budget).unwrap();
        assert_eq!(normalized.region_count, 2);
        assert_eq!(
            fixture_parameter_regions(&normalized).as_slice(),
            &[Some(RegionParamId(0)), Some(RegionParamId(0))]
        );
        assert_eq!(
            normalized.results.first().map(|row| row.region),
            Some(RegionParamId(1))
        );
        assert_eq!(
            normalized.outlives.as_ref(),
            &[(RegionParamId(0), RegionParamId(1))]
        );
        let reference = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Int(IntTy::I32)),
        };
        let params = [reference.clone(), reference.clone()];
        let ReturnContract::SharedInputPaths(paths) =
            fixture_derive_return_contract(&params, &reference, &normalized, &budget).unwrap()
        else {
            panic!("concrete declarations retain exact paths");
        };
        assert_eq!(
            paths
                .iter()
                .map(|path| path.input.param)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        raw.results = Arc::from([]);
        fixture_set_returns(&mut raw, ReturnContract::NoReference);
        let normalized = normalize_ref_contract(&raw, &budget).unwrap();
        assert_eq!(normalized.region_count, 1);
        assert_eq!(
            fixture_derive_return_contract(&params, &Ty::Unit, &normalized, &budget).unwrap(),
            ReturnContract::NoReference
        );
        validate_stored_ref_contract(&params, &Ty::Unit, &normalized, &budget).unwrap();
        let tiny = PlanBudget::new(PlanLimits {
            work: 1,
            ..PlanLimits::default()
        });
        assert!(normalize_ref_contract(&raw, &tiny).is_err());
        let mut shared = Ty::Int(IntTy::I32);
        for _ in 0..24 {
            let node = Arc::from([shared.clone(), shared]);
            shared = Ty::Tuple(node);
        }
        let erased = RefContract {
            input_regions: fixture_input_regions(Arc::from([None])),
            ..RefContract::default()
        };
        assert_eq!(
            fixture_derive_return_contract(&[shared], &Ty::Unit, &erased, &budget).unwrap(),
            ReturnContract::NoReference
        );
    }

    #[test]
    fn generic_reference_envelopes_are_rigid_and_transport_without_rederivation() {
        use super::*;
        let budget = PlanBudget::default();
        let opaque = Ty::Param(ParamId {
            owner: BinderId {
                declaration: test_decl(),
            },
            index: 0,
        });
        let reference = |ty| Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let scalar = Ty::Int(IntTy::I32);
        let mut contract = RefContract {
            results: fixture_result_regions(Some(RegionParamId(0)), ReturnContract::NoReference),
            input_regions: fixture_input_regions(Arc::from([
                Some(RegionParamId(0)),
                Some(RegionParamId(0)),
            ])),
            region_count: 1,
            outlives: Arc::from([]),
        };
        contract.results = derive_return_contracts(
            &[reference(opaque.clone()), reference(scalar.clone())],
            &reference(scalar.clone()),
            &contract,
            &budget,
        )
        .unwrap();
        assert_eq!(
            contract.results[0].returns,
            ReturnContract::SharedInputEnvelopes(Arc::from([
                InputEnvelope {
                    input: fixture_input(0),
                },
                InputEnvelope {
                    input: fixture_input(1),
                }
            ]))
        );
        validate_stored_ref_contract(
            &[reference(scalar.clone()), reference(scalar.clone())],
            &reference(scalar.clone()),
            &contract,
            &budget,
        )
        .unwrap();
        assert!(
            contract
                .validate(
                    &[reference(scalar.clone()), reference(scalar.clone())],
                    &reference(scalar.clone())
                )
                .is_err()
        );
        contract.input_regions = fixture_input_regions(Arc::from([Some(RegionParamId(0))]));
        contract.results = derive_return_contracts(
            &[reference(scalar.clone())],
            &reference(opaque),
            &contract,
            &budget,
        )
        .unwrap();
        assert_eq!(
            contract.results[0].returns,
            ReturnContract::SharedInputEnvelopes(Arc::from([InputEnvelope {
                input: fixture_input(0),
            }]))
        );
        let nested = Ty::Array {
            element: Arc::new(reference(scalar)),
            len: 0,
        };
        assert!(
            fixture_derive_return_contract(
                &[nested],
                &Ty::Unit,
                &RefContract {
                    input_regions: fixture_input_regions(Arc::from([None])),
                    ..RefContract::default()
                },
                &budget
            )
            .is_err()
        );
    }
    #[test]
    fn trait_goal_and_diagnostic_limits_share_authority_without_hidden_reservations() {
        let budget = PlanBudget::default();
        let alias = budget.clone();
        for _ in 0..4096 {
            alias.charge_obligation().unwrap();
        }
        assert!(budget.charge_obligation().is_err());
        budget.charge_diagnostic_bytes(65_535).unwrap();
        alias.charge_diagnostic_bytes(1).unwrap();
        assert!(budget.charge_diagnostic_bytes(1).is_err());
        let tiny = PlanBudget::new(PlanLimits {
            bytes: 1,
            ..PlanLimits::default()
        });
        let reservation = tiny.reserve_bytes(1).unwrap();
        assert!(tiny.reserve_bytes(1).is_err());
        drop(reservation);
        tiny.reserve_bytes(1).unwrap();
    }

    #[test]
    fn canonical_origin_tails_preserve_binders_prefixes_and_bounded_storage() {
        let budget = PlanBudget::default();
        let scope = Arc::new(
            ParamEnv::new(
                BinderId {
                    declaration: test_decl(),
                },
                2,
            )
            .unwrap(),
        );
        let env = GenericEnv::new(std::slice::from_ref(&scope), &budget).unwrap();
        let parameter = |index| ParamId {
            owner: scope.binder(),
            index,
        };
        let reference = |ty| Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let fixed = Arc::new(RefContract::default());
        let signature = Signature {
            ref_contract: fixed.clone(),
            params: vec![
                reference(Ty::Param(parameter(0))),
                Ty::Param(parameter(1)),
                Ty::Param(parameter(0)),
            ],
            result: Ty::Param(parameter(0)),
        };
        let schema = derive_signature_origin_schema(&signature, &env, &budget).unwrap();
        assert!(Arc::ptr_eq(&schema.fixed, &fixed));
        assert_eq!(schema.input_tails.len(), 3);
        assert_eq!(schema.input_tails[0].prefix, 1);
        assert_eq!(schema.result_tails[0].prefix, 0);
        assert_eq!(
            schema.result_tails[0].inputs.as_ref(),
            [schema.input_tails[0], schema.input_tails[2]]
        );
        assert_ne!(
            schema.input_tails[0].parameter,
            schema.input_tails[1].parameter
        );
        let mut nested = signature.clone();
        nested.result = reference(reference(Ty::Param(parameter(1))));
        let result = derive_signature_origin_schema(&nested, &env, &budget).unwrap();
        assert_eq!(result.result_tails[0].prefix, 2);
        assert_eq!(
            result.result_tails[0].inputs.as_ref(),
            [schema.input_tails[1]]
        );
        nested.result = Ty::Tuple(Arc::from([Ty::Param(parameter(0))]));
        assert!(
            derive_signature_origin_schema(&nested, &env, &budget)
                .unwrap()
                .result_tails
                .is_empty()
        );
        nested.result = Ty::Param(ParamId {
            owner: BinderId {
                declaration: test_decl(),
            },
            index: 0,
        });
        assert!(derive_signature_origin_schema(&nested, &env, &budget).is_err());
        nested.params = vec![Ty::Unit; MAX_LOAN_SITES + 1];
        assert!(derive_signature_origin_schema(&nested, &env, &budget).is_err());
        let tiny = PlanBudget::new(PlanLimits {
            bytes: 0,
            ..PlanLimits::default()
        });
        assert!(derive_signature_origin_schema(&signature, &env, &tiny).is_err());
    }

    #[test]
    fn associated_generic_environments_preserve_scope_authority_and_total_limits() {
        let budget = PlanBudget::default();
        let scope = |arity| {
            Arc::new(
                ParamEnv::new(
                    BinderId {
                        declaration: test_decl(),
                    },
                    arity,
                )
                .unwrap(),
            )
        };
        let implementation = scope(1);
        let method = scope(0);
        let env = GenericEnv::new(&[implementation.clone(), method.clone()], &budget).unwrap();
        assert_eq!(env.scopes().len(), 2);
        assert_eq!(env.total_arity(), 1);
        assert!(env.is_generic());
        let params = env.ordered_params().collect::<Vec<_>>();
        assert_eq!(
            params,
            vec![ParamId {
                owner: implementation.binder(),
                index: 0
            }]
        );
        assert!(!env.contains(ParamId {
            owner: method.binder(),
            index: 0
        }));
        assert!(GenericEnv::new(&[implementation.clone(), implementation], &budget).is_err());
        assert!(GenericEnv::new(&[scope(16), scope(17)], &budget).is_err());
        assert_eq!(
            GenericEnv::new(&[scope(16), scope(16)], &budget)
                .unwrap()
                .total_arity(),
            32
        );
        let nongeneric = GenericEnv::new(&[scope(0), scope(0)], &budget).unwrap();
        assert!(!nongeneric.is_generic());
        assert_eq!(nongeneric.scopes().len(), 2);
        let tiny = PlanBudget::new(PlanLimits {
            bytes: 0,
            ..PlanLimits::default()
        });
        assert!(GenericEnv::new(&[method], &tiny).is_err());
        assert!(GenericEnv::new(&(0..33).map(|_| scope(0)).collect::<Vec<_>>(), &budget).is_err());
    }

    #[test]
    fn associated_substitution_is_simultaneous_across_rigid_impl_and_method_binders() {
        struct NoNominals;
        impl SubstitutionCx for NoNominals {
            fn nominal_application(&self, _: NominalAppId) -> Result<NominalAppInfo, String> {
                Err("unexpected nominal".into())
            }
            fn intern_nominal(
                &mut self,
                _: DeclId,
                _: &[Ty],
                _: FactsCtx<'_>,
                _: &PlanBudget,
            ) -> Result<Ty, String> {
                Err("unexpected nominal".into())
            }
            fn instantiate_signature(
                &mut self,
                _: &CallTarget,
                _: FactsCtx<'_>,
                _: &PlanBudget,
            ) -> Result<Signature, String> {
                Err("unexpected call".into())
            }
        }
        let budget = PlanBudget::default();
        let scopes = [test_decl(), test_decl()]
            .map(|declaration| Arc::new(ParamEnv::new(BinderId { declaration }, 1).unwrap()));
        let env = GenericEnv::new(&scopes, &budget).unwrap();
        let params = env.ordered_params().map(Ty::Param).collect::<Vec<_>>();
        let input = Ty::Tuple(
            vec![
                params[0].clone(),
                Ty::Array {
                    element: Arc::new(params[1].clone()),
                    len: 0,
                },
            ]
            .into(),
        );
        let actuals = [params[1].clone(), params[0].clone()];
        let result = substitute_ty(
            &input,
            &env,
            &actuals,
            FactsCtx::Universal(&env),
            &mut NoNominals,
            &budget,
        )
        .unwrap();
        assert_eq!(
            result,
            Ty::Tuple(
                vec![
                    params[1].clone(),
                    Ty::Array {
                        element: Arc::new(params[0].clone()),
                        len: 0
                    }
                ]
                .into()
            )
        );
        assert!(FactsCtx::Concrete.validate_ty(&result).is_err());
        assert!(
            substitute_ty(
                &input,
                &env,
                &actuals[..1],
                FactsCtx::Universal(&env),
                &mut NoNominals,
                &budget
            )
            .is_err()
        );
        let invalid = Ty::Param(ParamId {
            owner: scopes[0].binder(),
            index: 1,
        });
        assert!(
            substitute_ty(
                &invalid,
                &env,
                &actuals,
                FactsCtx::Universal(&env),
                &mut NoNominals,
                &budget
            )
            .is_err()
        );
        let concrete = substitute_ty(
            &input,
            &env,
            &[Ty::Int(IntTy::U64), Ty::Bool],
            FactsCtx::Concrete,
            &mut NoNominals,
            &budget,
        )
        .unwrap();
        assert!(FactsCtx::Concrete.validate_ty(&concrete).is_ok());
    }

    #[test]
    fn generic_facts_are_rigid_noncopy_and_planning_limits_share_authority() {
        let binder = BinderId {
            declaration: test_decl(),
        };
        let env = GenericEnv::new(
            &[Arc::new(ParamEnv::new(binder, 1).unwrap())],
            &PlanBudget::default(),
        )
        .unwrap();
        let param = Ty::Param(ParamId {
            owner: binder,
            index: 0,
        });
        assert!(FactsCtx::Universal(&env).validate_ty(&param).is_ok());
        assert!(FactsCtx::Concrete.validate_ty(&param).is_err());
        assert!(!FactsCtx::Universal(&env).must_copy(&param).unwrap());
        assert!(
            !Ty::Array {
                element: Arc::new(param.clone()),
                len: 0
            }
            .is_copy()
        );
        let other_scope = ParamEnv::new(
            BinderId {
                declaration: test_decl(),
            },
            1,
        )
        .unwrap();
        let other = GenericEnv::new(&[Arc::new(other_scope)], &PlanBudget::default()).unwrap();
        assert!(FactsCtx::Universal(&other).validate_ty(&param).is_err());
        let hidden = Ty::Array {
            element: Arc::new(Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(Ty::Int(IntTy::I32)),
            }),
            len: 0,
        };
        assert!(FactsCtx::Universal(&env).validate_ty(&hidden).is_err());
        let budget = PlanBudget::new(PlanLimits {
            applications: 4,
            bytes: 16,
            ..PlanLimits::default()
        });
        let clone = budget.clone();
        assert!(budget.same_authority(&clone));
        assert!(!budget.same_authority(&PlanBudget::default()));
        let workers = (0..8)
            .map(|_| {
                let budget = budget.clone();
                std::thread::spawn(move || budget.charge_application().is_ok())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .filter(|success| *success)
                .count(),
            4
        );
        let reservation = budget.reserve_bytes(16).unwrap();
        assert!(clone.reserve_bytes(1).is_err());
        drop(reservation);
        assert!(clone.reserve_bytes(16).is_ok());
        assert!(budget.check_depth(129).is_err());
    }
    fn test_decl() -> super::DeclId {
        super::super::traits::test_support::source_decl(
            super::SnapshotId::allocate().unwrap(),
            0,
            &PlanBudget::default(),
        )
        .unwrap()
    }
    fn test_target() -> super::Callee {
        super::Callee::Direct(super::CallTarget {
            declaration: test_decl(),
            type_args: std::sync::Arc::from([]),
        })
    }
    #[test]
    fn compounds_validate_closed_domains_dead_statements_and_indices() {
        let ops = [
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
        ];
        for op in ops {
            let mut function = fixture();
            function.exprs.push(TypedExpr {
                ty: Ty::Int(IntTy::I32),
                completion: Completion::Returns,
                kind: ExprKind::Int(IntValue::i32(1)),
            });
            function.body.statements.push(Stmt::AssignOp {
                place: Place {
                    local: LocalId(0),
                    projection: vec![],
                },
                op,
                value: ExprId(1),
            });
            function.validate().unwrap();
            function.exprs[1] = TypedExpr {
                ty: Ty::Int(IntTy::U64),
                completion: Completion::Returns,
                kind: ExprKind::Int(IntValue {
                    ty: IntTy::U64,
                    bits: 1,
                }),
            };
            assert_eq!(
                function.validate().is_ok(),
                matches!(op, BinaryOp::Shl | BinaryOp::Shr)
            );
            function.locals[0].ty = Ty::Bool;
            function.exprs[1] = TypedExpr {
                ty: Ty::Bool,
                completion: Completion::Returns,
                kind: ExprKind::Bool(true),
            };
            assert_eq!(
                function.validate().is_ok(),
                matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor)
            );
        }
        let mut function = fixture();
        function.body.statements = vec![
            Stmt::Return(None),
            Stmt::AssignOp {
                place: Place {
                    local: LocalId(0),
                    projection: vec![],
                },
                op: BinaryOp::Eq,
                value: ExprId(0),
            },
        ];
        for op in [
            BinaryOp::Eq,
            BinaryOp::Ne,
            BinaryOp::Lt,
            BinaryOp::Le,
            BinaryOp::Gt,
            BinaryOp::Ge,
            BinaryOp::And,
            BinaryOp::Or,
        ] {
            let Stmt::AssignOp { op: actual, .. } = &mut function.body.statements[1] else {
                unreachable!()
            };
            *actual = op;
            assert!(function.validate().unwrap_err().contains("compound"));
        }
        function.locals[0].ty = Ty::Array {
            element: Arc::new(Ty::Int(IntTy::I32)),
            len: 2,
        };
        function.exprs[0] = TypedExpr {
            ty: Ty::Int(IntTy::I32),
            completion: Completion::Returns,
            kind: ExprKind::Int(IntValue::i32(1)),
        };
        function.exprs.push(TypedExpr {
            ty: Ty::Int(IntTy::Usize),
            completion: Completion::Returns,
            kind: ExprKind::Int(IntValue::usize(0)),
        });
        function.body.statements[1] = Stmt::AssignIndexOp {
            array: Place {
                local: LocalId(0),
                projection: vec![],
            },
            index: ExprId(1),
            op: BinaryOp::Shl,
            value: ExprId(0),
        };
        function.validate().unwrap();
        function.exprs[1] = TypedExpr {
            ty: Ty::Int(IntTy::U64),
            completion: Completion::Returns,
            kind: ExprKind::Int(IntValue {
                ty: IntTy::U64,
                bits: 0,
            }),
        };
        assert!(function.validate().unwrap_err().contains("usize"));
        function.exprs[1] = TypedExpr {
            ty: Ty::Int(IntTy::Usize),
            completion: Completion::Returns,
            kind: ExprKind::Int(IntValue::usize(0)),
        };
        function.exprs[0].kind = ExprKind::Int(IntValue::i32(32));
        assert!(function.validate().unwrap_err().contains("shift count"));
        function.exprs[0].completion = Completion::Never;
        function.validate().unwrap();
        let Stmt::AssignIndexOp { index, .. } = &mut function.body.statements[1] else {
            unreachable!()
        };
        *index = ExprId(usize::MAX);
        assert!(function.validate().is_err());
    }

    #[test]
    fn shifts_validate_independent_integer_types_and_eager_completion() {
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
        for op in [BinaryOp::Shl, BinaryOp::Shr] {
            for left in types {
                for right in types {
                    let mut function = fixture();
                    function.exprs[0] = TypedExpr {
                        ty: Ty::Int(left),
                        completion: Completion::Returns,
                        kind: ExprKind::Int(IntValue { ty: left, bits: 1 }),
                    };
                    function.exprs.push(TypedExpr {
                        ty: Ty::Int(right),
                        completion: Completion::Returns,
                        kind: ExprKind::Int(IntValue { ty: right, bits: 0 }),
                    });
                    function.exprs.push(TypedExpr {
                        ty: Ty::Int(left),
                        completion: Completion::Returns,
                        kind: ExprKind::Binary {
                            op,
                            left: ExprId(0),
                            right: ExprId(1),
                        },
                    });
                    function.validate().unwrap();
                    function.exprs[2].ty = Ty::Bool;
                    assert!(function.validate().is_err());
                    function.exprs[2].ty = Ty::Int(left);
                    for endpoint in [0, 1] {
                        function.exprs[endpoint].completion = Completion::Never;
                        assert!(function.validate().unwrap_err().contains("completion"));
                        function.exprs[2].completion = Completion::Never;
                        function.validate().unwrap();
                        function.exprs[endpoint].completion = Completion::Returns;
                        function.exprs[2].completion = Completion::Returns;
                    }
                    for endpoint in [0, 1] {
                        let saved = function.exprs[endpoint].clone();
                        for ty in [
                            Ty::Bool,
                            Ty::Unit,
                            Ty::Never,
                            Ty::Tuple(Arc::from([Ty::Bool])),
                            Ty::Ref {
                                kind: BorrowKind::Shared,
                                referent: Arc::new(Ty::Bool),
                            },
                        ] {
                            function.exprs[endpoint] = TypedExpr {
                                ty,
                                completion: Completion::Returns,
                                kind: ExprKind::Bool(false),
                            };
                            assert!(function.validate().is_err());
                        }
                        function.exprs[endpoint] = saved;
                    }
                }
            }
        }
    }

    #[test]
    fn bitwise_validates_exact_types_dead_operands_and_eager_completion() {
        for op in [BinaryOp::BitAnd, BinaryOp::BitOr, BinaryOp::BitXor] {
            let mut function = fixture();
            function.exprs.push(TypedExpr {
                ty: Ty::Int(IntTy::I32),
                completion: Completion::Returns,
                kind: ExprKind::Binary {
                    op,
                    left: ExprId(0),
                    right: ExprId(0),
                },
            });
            let binary = function.exprs.len() - 1;
            function.validate().unwrap();
            function.exprs[binary].ty = Ty::Int(IntTy::U32);
            assert!(function.validate().is_err());
            function.exprs[binary].ty = Ty::Int(IntTy::I32);
            function.exprs[0].completion = Completion::Never;
            assert!(function.validate().unwrap_err().contains("completion"));
            function.exprs[binary].completion = Completion::Never;
            function.validate().unwrap();
            function.exprs[0] = TypedExpr {
                ty: Ty::Bool,
                completion: Completion::Returns,
                kind: ExprKind::Bool(false),
            };
            function.exprs[binary].ty = Ty::Bool;
            function.exprs[binary].completion = Completion::Returns;
            function.validate().unwrap();
            for ty in [
                Ty::Unit,
                Ty::Never,
                Ty::Tuple(Arc::from([Ty::Bool])),
                Ty::Ref {
                    kind: BorrowKind::Shared,
                    referent: Arc::new(Ty::Bool),
                },
            ] {
                function.exprs[0].ty = ty.clone();
                function.exprs[binary].ty = ty;
                assert!(function.validate().is_err());
            }
            function.exprs[0] = TypedExpr {
                ty: Ty::Int(IntTy::U8),
                completion: Completion::Returns,
                kind: ExprKind::Int(IntValue {
                    ty: IntTy::U8,
                    bits: 256,
                }),
            };
            function.exprs[binary].ty = Ty::Int(IntTy::U8);
            assert!(function.validate().unwrap_err().contains("noncanonical"));
        }
    }

    #[test]
    fn casts_validate_dead_endpoints_edges_and_typed_noncompletion() {
        let mut function = fixture();
        function.exprs.push(TypedExpr {
            ty: Ty::Int(IntTy::U8),
            completion: Completion::Returns,
            kind: ExprKind::IntCast {
                value: ExprId(0),
                to: IntTy::U8,
            },
        });
        function.validate().unwrap();
        for source in [
            Ty::Bool,
            Ty::Unit,
            Ty::Never,
            Ty::Tuple(Arc::from([Ty::Int(IntTy::I32)])),
            Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(Ty::Int(IntTy::I32)),
            },
        ] {
            let mut invalid = function.clone();
            invalid.exprs[0].ty = source;
            assert!(invalid.validate().unwrap_err().contains("integer source"));
        }
        let mut invalid = function.clone();
        invalid.exprs[1].ty = Ty::Int(IntTy::U64);
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("result type mismatch")
        );
        invalid = function.clone();
        invalid.exprs[1].kind = ExprKind::IntCast {
            value: ExprId(99),
            to: IntTy::U8,
        };
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        invalid = function.clone();
        invalid.exprs[0].kind = ExprKind::Int(IntValue {
            ty: IntTy::I32,
            bits: 1u64 << 32,
        });
        assert!(invalid.validate().unwrap_err().contains("noncanonical"));
        invalid = function.clone();
        invalid.exprs[1].completion = Completion::Never;
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("completion mismatch")
        );
        function.exprs.push(TypedExpr {
            ty: Ty::Int(IntTy::I32),
            completion: Completion::Returns,
            kind: ExprKind::Int(IntValue::i32(7)),
        });
        function.exprs[0].kind = ExprKind::Block(Block {
            statements: vec![Stmt::Return(None)],
            tail: Some(ExprId(2)),
        });
        function.exprs[0].completion = Completion::Never;
        function.exprs[1].completion = Completion::Never;
        function.validate().unwrap();
        assert_eq!(function.exprs[1].ty, Ty::Int(IntTy::U8));
    }

    #[test]
    fn integer_descriptors_constants_and_patterns_preserve_exact_identity() {
        let types = [
            (IntTy::I8, 8, true, "i8"),
            (IntTy::I16, 16, true, "i16"),
            (IntTy::I32, 32, true, "i32"),
            (IntTy::I64, 64, true, "i64"),
            (IntTy::Isize, 64, true, "isize"),
            (IntTy::U8, 8, false, "u8"),
            (IntTy::U16, 16, false, "u16"),
            (IntTy::U32, 32, false, "u32"),
            (IntTy::U64, 64, false, "u64"),
            (IntTy::Usize, 64, false, "usize"),
        ];
        for (ty, bits, signed, name) in types {
            assert_eq!(ty.bits(), bits);
            assert_eq!(ty.is_signed(), signed);
            assert_eq!(ty.name(), name);
            assert_eq!(ty.mask(), u64::MAX >> (64 - bits));
            assert!(Ty::Int(ty).is_copy());
            for raw in [0, ty.mask(), 1u64 << (bits - 1)] {
                let integer = IntValue { ty, bits: raw };
                integer.validate().unwrap();
                let mut function = fixture();
                function.exprs[0].ty = Ty::Int(ty);
                function.exprs[0].kind = ExprKind::Int(integer);
                function.validate().unwrap();
                let arms = [MatchArm {
                    pattern: MatchPattern::Int(integer),
                    body: ExprId(0),
                }];
                function
                    .validate_match(ExprId(0), &arms, &Default::default())
                    .unwrap();
                let wrong = if ty == IntTy::I8 {
                    IntTy::U8
                } else {
                    IntTy::I8
                };
                assert!(!pattern_types_equal(&Ty::Int(ty), &Ty::Int(wrong), &mut 0).unwrap());
                function.exprs[0].ty = Ty::Int(wrong);
                assert!(function.validate().is_err());
                assert!(
                    function
                        .validate_match(ExprId(0), &arms, &Default::default())
                        .is_err()
                );
            }
            if bits < 64 {
                let invalid = IntValue {
                    ty,
                    bits: 1u64 << bits,
                };
                assert!(invalid.validate().is_err());
                let mut function = fixture();
                function.exprs.push(TypedExpr {
                    ty: Ty::Int(ty),
                    completion: Completion::Returns,
                    kind: ExprKind::Int(invalid),
                });
                assert!(function.validate().unwrap_err().contains("noncanonical"));
                let arms = [MatchArm {
                    pattern: MatchPattern::Int(invalid),
                    body: ExprId(0),
                }];
                assert!(
                    function
                        .validate_match(ExprId(0), &arms, &Default::default())
                        .unwrap_err()
                        .contains("noncanonical")
                );
            }
        }
        assert_eq!(IntValue::i32(-1).bits, u32::MAX as u64);
        assert_eq!(IntValue::usize(u64::MAX).bits, u64::MAX);
        assert!(
            !pattern_types_equal(&Ty::Int(IntTy::U64), &Ty::Int(IntTy::Usize), &mut 0).unwrap()
        );
        assert!(
            !pattern_types_equal(&Ty::Int(IntTy::I64), &Ty::Int(IntTy::Isize), &mut 0).unwrap()
        );
    }
    #[test]
    fn projected_input_path_resources_bound_occurrences_and_buffers() {
        let reference = |ty| Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let contract = RefContract {
            results: fixture_result_regions(
                Some(RegionParamId(0)),
                ReturnContract::SharedInputPaths(Arc::from([])),
            ),
            input_regions: fixture_input_regions(Arc::from([Some(RegionParamId(0))])),
            region_count: 1,
            outlives: Arc::from([]),
        };
        let output = reference(Ty::Int(IntTy::I32));
        let array = Ty::Array {
            element: Arc::new(Ty::Int(IntTy::I32)),
            len: MAX_INPUT_PATHS,
        };
        let candidates = contract
            .eligible_input_paths(&[reference(array.clone())], &output)
            .unwrap();
        assert_eq!(candidates.len(), MAX_INPUT_PATHS);
        assert_eq!(
            candidates.last().unwrap().projection.as_ref(),
            &[Projection::ConstantIndex(MAX_INPUT_PATHS - 1)]
        );
        let too_many = Ty::Tuple(Arc::from([array.clone(), Ty::Int(IntTy::I32)]));
        assert!(
            contract
                .eligible_input_paths(&[reference(too_many)], &output)
                .unwrap_err()
                .contains("candidate limit")
        );
        let mut many_steps = array;
        for _ in 0..64 {
            many_steps = Ty::Tuple(Arc::from([many_steps]));
        }
        assert!(
            contract
                .eligible_input_paths(&[reference(many_steps)], &output)
                .unwrap_err()
                .contains("step resource")
        );
        let mut dag = Ty::Bool;
        for _ in 0..16 {
            dag = Ty::Tuple(Arc::from([dag.clone(), dag]));
        }
        assert!(
            contract
                .eligible_input_paths(&[reference(dag.clone())], &output)
                .unwrap()
                .is_empty()
        );
        dag = Ty::Tuple(Arc::from([dag.clone(), dag]));
        assert!(
            contract
                .eligible_input_paths(&[reference(dag)], &output)
                .unwrap_err()
                .contains("occurrence resource")
        );
        let oversized = InputPath {
            input: fixture_input(0),
            projection: vec![Projection::Field(0); 128].into(),
        };
        let malformed = RefContract {
            results: fixture_result_regions(
                contract.results.first().map(|row| row.region),
                ReturnContract::SharedInputPaths(
                    vec![oversized; MAX_INPUT_PATH_STEPS / 128 + 1].into(),
                ),
            ),
            ..contract.clone()
        };
        assert!(
            malformed
                .validate(std::slice::from_ref(&output), &output)
                .unwrap_err()
                .contains("step resource")
        );
        let malformed = RefContract {
            results: fixture_result_regions(
                contract.results.first().map(|row| row.region),
                ReturnContract::SharedInputPaths(
                    vec![
                        InputPath {
                            input: fixture_input(usize::MAX),
                            projection: Arc::from([]),
                        };
                        MAX_INPUT_PATHS + 1
                    ]
                    .into(),
                ),
            ),
            ..contract
        };
        assert!(
            malformed
                .validate(std::slice::from_ref(&output), &output)
                .unwrap_err()
                .contains("candidate limit")
        );
    }

    #[test]
    fn projected_input_paths_preserve_occurrences_privacy_and_exact_metadata() {
        let snapshot = SnapshotId::allocate().unwrap();
        let pair = Ty::Tuple(Arc::from([Ty::Int(IntTy::I32), Ty::Int(IntTy::I32)]));
        let enumeration = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id: NominalAppId { snapshot, index: 7 },
            name: "E".into(),
            module: String::new(),
            public: false,
            variants: Arc::from([VariantDef {
                name: "V".into(),
                shape: VariantShape::Tuple,
                fields: Arc::from([FieldDef {
                    name: "0".into(),
                    ty: Ty::Int(IntTy::I32),
                    public: false,
                }]),
            }]),
        });
        let root = Ty::Struct(nominal(
            NominalAppId { snapshot, index: 0 },
            "Private",
            vec![
                pair.clone(),
                Ty::Array {
                    element: Arc::new(pair),
                    len: 2,
                },
                Ty::Array {
                    element: Arc::new(Ty::Int(IntTy::I32)),
                    len: 0,
                },
                Ty::Enum(enumeration.clone()),
            ],
        ));
        let reference = |ty| Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let params = vec![
            Ty::Bool,
            reference(root.clone()),
            reference(Ty::Int(IntTy::I32)),
        ];
        let result = reference(Ty::Int(IntTy::I32));
        let mut contract = RefContract {
            results: fixture_result_regions(
                Some(RegionParamId(0)),
                ReturnContract::SharedInputPaths(Arc::from([])),
            ),
            input_regions: fixture_input_regions(Arc::from([
                None,
                Some(RegionParamId(0)),
                Some(RegionParamId(0)),
            ])),
            region_count: 1,
            outlives: Arc::from([]),
        };
        let candidate = |param, projection: Vec<Projection>| InputPath {
            input: fixture_input(param),
            projection: projection.into(),
        };
        let expected = vec![
            candidate(1, vec![Projection::Field(0), Projection::Field(0)]),
            candidate(1, vec![Projection::Field(0), Projection::Field(1)]),
            candidate(
                1,
                vec![
                    Projection::Field(1),
                    Projection::ConstantIndex(0),
                    Projection::Field(0),
                ],
            ),
            candidate(
                1,
                vec![
                    Projection::Field(1),
                    Projection::ConstantIndex(0),
                    Projection::Field(1),
                ],
            ),
            candidate(
                1,
                vec![
                    Projection::Field(1),
                    Projection::ConstantIndex(1),
                    Projection::Field(0),
                ],
            ),
            candidate(
                1,
                vec![
                    Projection::Field(1),
                    Projection::ConstantIndex(1),
                    Projection::Field(1),
                ],
            ),
            candidate(2, vec![]),
        ];
        assert_eq!(
            contract.eligible_input_paths(&params, &result).unwrap(),
            expected
        );
        fixture_set_returns(
            &mut contract,
            ReturnContract::SharedInputPaths(expected.clone().into()),
        );
        contract.validate(&params, &result).unwrap();
        for mut malformed in [
            expected[..6].to_vec(),
            {
                let mut paths = expected.clone();
                paths.swap(0, 1);
                paths
            },
            {
                let mut paths = expected.clone();
                paths.push(paths[0].clone());
                paths
            },
        ] {
            fixture_set_returns(
                &mut contract,
                ReturnContract::SharedInputPaths(std::mem::take(&mut malformed).into()),
            );
            assert!(contract.validate(&params, &result).is_err());
        }
        for path in [
            vec![Projection::Deref],
            vec![Projection::Downcast(VariantId {
                owner: enumeration.def().id,
                index: 0,
            })],
            vec![Projection::Field(99)],
            vec![Projection::ConstantIndex(0)],
        ] {
            fixture_set_returns(
                &mut contract,
                ReturnContract::SharedInputPaths(Arc::from([candidate(1, path)])),
            );
            assert!(contract.validate(&params, &result).is_err());
        }
        let Ty::Struct(root_type) = &root else {
            unreachable!()
        };
        let other = Ty::Struct(nominal(
            NominalAppId { snapshot, index: 1 },
            "Private",
            root_type
                .def()
                .fields
                .iter()
                .map(|field| field.ty.clone())
                .collect(),
        ));
        assert!(
            contract
                .eligible_input_paths(&params, &reference(other))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            contract
                .eligible_input_paths(&params, &reference(root))
                .unwrap(),
            vec![candidate(1, vec![])]
        );
        assert_eq!(
            contract
                .eligible_input_paths(&params, &reference(Ty::Enum(enumeration)))
                .unwrap(),
            vec![candidate(1, vec![Projection::Field(3)])]
        );
    }

    fn empty_input_paths(params: &[usize]) -> Arc<[InputPath]> {
        params
            .iter()
            .map(|param| InputPath {
                input: fixture_input(*param),
                projection: Arc::from([]),
            })
            .collect::<Vec<_>>()
            .into()
    }
    #[test]
    fn mixed_reference_candidates_preserve_capability_regions_and_complete_paths() {
        let reference = |kind, ty| Ty::Ref {
            kind,
            referent: Arc::new(ty),
        };
        let pair = Ty::Tuple(Arc::from([Ty::Int(IntTy::I32), Ty::Bool]));
        let params = vec![
            Ty::Bool,
            reference(BorrowKind::Shared, pair.clone()),
            reference(BorrowKind::Mut, pair),
            reference(BorrowKind::Shared, Ty::Int(IntTy::I32)),
            reference(BorrowKind::Mut, Ty::Bool),
        ];
        let path = |param| InputPath {
            input: fixture_input(param),
            projection: Arc::from([Projection::Field(0)]),
        };
        let mut contract = RefContract {
            results: fixture_result_regions(
                Some(RegionParamId(0)),
                ReturnContract::SharedInputPaths(Arc::from([path(1), path(2)])),
            ),
            input_regions: fixture_input_regions(Arc::from([
                None,
                Some(RegionParamId(0)),
                Some(RegionParamId(0)),
                Some(RegionParamId(1)),
                Some(RegionParamId(0)),
            ])),
            region_count: 2,
            outlives: Arc::from([]),
        };
        let shared_result = reference(BorrowKind::Shared, Ty::Int(IntTy::I32));
        assert_eq!(
            contract
                .eligible_input_paths(&params, &shared_result)
                .unwrap(),
            vec![path(1), path(2)]
        );
        contract.validate(&params, &shared_result).unwrap();
        for candidates in [
            vec![path(1)],
            vec![path(2), path(1)],
            vec![path(1), path(2), path(2)],
        ] {
            let forged = RefContract {
                results: fixture_result_regions(
                    contract.results.first().map(|row| row.region),
                    ReturnContract::SharedInputPaths(candidates.into()),
                ),
                ..contract.clone()
            };
            assert!(forged.validate(&params, &shared_result).is_err());
        }
        let mutable_result = reference(BorrowKind::Mut, Ty::Int(IntTy::I32));
        fixture_set_returns(
            &mut contract,
            ReturnContract::MutInputPaths(Arc::from([path(2)])),
        );
        assert_eq!(
            contract
                .eligible_input_paths(&params, &mutable_result)
                .unwrap(),
            vec![path(2)]
        );
        contract.validate(&params, &mutable_result).unwrap();
        for candidates in [vec![path(1)], vec![path(1), path(2)], vec![]] {
            let forged = RefContract {
                results: fixture_result_regions(
                    contract.results.first().map(|row| row.region),
                    ReturnContract::MutInputPaths(candidates.into()),
                ),
                ..contract.clone()
            };
            assert!(forged.validate(&params, &mutable_result).is_err());
        }
        contract.outlives = Arc::from([(RegionParamId(1), RegionParamId(0))]);
        assert_eq!(
            contract
                .eligible_input_paths(&params, &mutable_result)
                .unwrap(),
            vec![path(2)]
        );
        assert_eq!(
            contract
                .eligible_input_paths(&params, &shared_result)
                .unwrap(),
            vec![
                path(1),
                path(2),
                InputPath {
                    input: fixture_input(3),
                    projection: Arc::from([]),
                }
            ]
        );
        contract.input_regions = fixture_input_regions(Arc::from([
            None,
            None,
            Some(RegionParamId(0)),
            Some(RegionParamId(1)),
            Some(RegionParamId(0)),
        ]));
        assert!(
            contract
                .eligible_input_paths(&params, &mutable_result)
                .unwrap_err()
                .contains("missing reference parameter region")
        );
    }

    #[test]
    fn mutable_inputs_to_shared_results_keep_existing_candidate_resource_caps() {
        let reference = |kind, ty| Ty::Ref {
            kind,
            referent: Arc::new(ty),
        };
        let array = Ty::Array {
            element: Arc::new(Ty::Int(IntTy::I32)),
            len: MAX_INPUT_PATHS,
        };
        let shared_result = reference(BorrowKind::Shared, Ty::Int(IntTy::I32));
        let mut contract = RefContract {
            results: fixture_result_regions(
                Some(RegionParamId(0)),
                ReturnContract::SharedInputPaths(Arc::from([])),
            ),
            input_regions: fixture_input_regions(Arc::from([Some(RegionParamId(0))])),
            region_count: 1,
            outlives: Arc::from([]),
        };
        let params = [reference(BorrowKind::Mut, array.clone())];
        let candidates = contract
            .eligible_input_paths(&params, &shared_result)
            .unwrap();
        assert_eq!(candidates.len(), MAX_INPUT_PATHS);
        assert_eq!(
            candidates[MAX_INPUT_PATHS - 1].projection.as_ref(),
            [Projection::ConstantIndex(MAX_INPUT_PATHS - 1)]
        );
        fixture_set_returns(
            &mut contract,
            ReturnContract::SharedInputPaths(candidates.into()),
        );
        contract.validate(&params, &shared_result).unwrap();
        let overflow = [reference(
            BorrowKind::Mut,
            Ty::Tuple(Arc::from([array.clone(), Ty::Int(IntTy::I32)])),
        )];
        assert!(
            contract
                .eligible_input_paths(&overflow, &shared_result)
                .unwrap_err()
                .contains("candidate limit")
        );
        let shared_params = [reference(BorrowKind::Shared, array)];
        let mutable_result = reference(BorrowKind::Mut, Ty::Int(IntTy::I32));
        fixture_set_returns(&mut contract, ReturnContract::MutInputPaths(Arc::from([])));
        contract.validate(&shared_params, &mutable_result).unwrap();
        assert!(
            contract
                .eligible_input_paths(&shared_params, &mutable_result)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn mutable_reference_contracts_require_exact_kinds_and_complete_candidates() {
        let mutable = |ty| Ty::Ref {
            kind: BorrowKind::Mut,
            referent: Arc::new(ty),
        };
        let params = vec![
            Ty::Bool,
            mutable(Ty::Bool),
            mutable(Ty::Int(IntTy::I32)),
            mutable(Ty::Int(IntTy::I32)),
        ];
        let result = mutable(Ty::Int(IntTy::I32));
        let contract = RefContract {
            results: fixture_result_regions(
                Some(RegionParamId(1)),
                ReturnContract::MutInputPaths(empty_input_paths(&[2, 3])),
            ),
            input_regions: fixture_input_regions(Arc::from([
                None,
                Some(RegionParamId(0)),
                Some(RegionParamId(0)),
                Some(RegionParamId(1)),
            ])),
            region_count: 2,
            outlives: Arc::from([(RegionParamId(0), RegionParamId(1))]),
        };
        contract.validate(&params, &result).unwrap();
        assert_eq!(
            contract.eligible_input_paths(&params, &result).unwrap(),
            empty_input_paths(&[2, 3]).to_vec()
        );
        assert!(contract.validation_work(&params, &result).unwrap() > params.len());
        for candidates in [
            vec![2],
            vec![1, 2, 3],
            vec![3, 2],
            vec![2, 2, 3],
            vec![usize::MAX],
        ] {
            let malformed = RefContract {
                results: fixture_result_regions(
                    contract.results.first().map(|row| row.region),
                    ReturnContract::MutInputPaths(empty_input_paths(&candidates)),
                ),
                ..contract.clone()
            };
            assert!(malformed.validate(&params, &result).is_err());
        }
        let forged = RefContract {
            results: fixture_result_regions(
                contract.results.first().map(|row| row.region),
                ReturnContract::SharedInputPaths(empty_input_paths(&[2, 3])),
            ),
            ..contract.clone()
        };
        assert!(
            forged
                .validate(&params, &result)
                .unwrap_err()
                .contains("kind mismatch")
        );
        let shared_result = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Int(IntTy::I32)),
        };
        assert!(
            contract
                .validate(&params, &shared_result)
                .unwrap_err()
                .contains("kind mismatch")
        );
        let mut mixed = params.clone();
        mixed[1] = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Bool),
        };
        contract.validate(&mixed, &result).unwrap();
        RefContract::default().validate(&mixed, &Ty::Unit).unwrap();
        let divergent = RefContract {
            results: fixture_result_regions(
                Some(RegionParamId(0)),
                ReturnContract::MutInputPaths(empty_input_paths(&[])),
            ),
            input_regions: fixture_input_regions(Arc::from([])),
            region_count: 1,
            outlives: Arc::from([]),
        };
        divergent.validate(&[], &result).unwrap();
    }

    #[test]
    fn reference_contract_closure_candidates_and_malformed_metadata() {
        use super::*;
        let shared = |ty| Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let params = vec![
            Ty::Bool,
            shared(Ty::Bool),
            shared(Ty::Int(IntTy::I32)),
            shared(Ty::Int(IntTy::I32)),
        ];
        let result = shared(Ty::Int(IntTy::I32));
        let contract = RefContract {
            results: fixture_result_regions(
                Some(RegionParamId(2)),
                ReturnContract::SharedInputPaths(empty_input_paths(&[2, 3])),
            ),
            input_regions: fixture_input_regions(Arc::from([
                None,
                Some(RegionParamId(0)),
                Some(RegionParamId(0)),
                Some(RegionParamId(1)),
            ])),
            region_count: 3,
            outlives: Arc::from([
                (RegionParamId(0), RegionParamId(1)),
                (RegionParamId(1), RegionParamId(2)),
            ]),
        };
        contract.validate(&params, &result).unwrap();
        assert!(
            contract
                .outlives_region(RegionParamId(0), RegionParamId(2))
                .unwrap()
        );
        assert!(
            !contract
                .outlives_region(RegionParamId(2), RegionParamId(0))
                .unwrap()
        );
        assert_eq!(
            contract.eligible_input_paths(&params, &result).unwrap(),
            empty_input_paths(&[2, 3]).to_vec()
        );
        assert!(contract.validation_work(&params, &result).unwrap() >= params.len());
        let mut invalid = contract.clone();
        fixture_set_returns(
            &mut invalid,
            ReturnContract::SharedInputPaths(empty_input_paths(&[2])),
        );
        assert!(invalid.validate(&params, &result).is_err());
        fixture_set_returns(
            &mut invalid,
            ReturnContract::SharedInputPaths(empty_input_paths(&[3, 2])),
        );
        assert!(invalid.validate(&params, &result).is_err());
        fixture_set_returns(
            &mut invalid,
            ReturnContract::SharedInputPaths(empty_input_paths(&[2, 2, 3])),
        );
        assert!(invalid.validate(&params, &result).is_err());
        for region in [3, u16::MAX] {
            let mut invalid = contract.clone();
            Arc::make_mut(&mut invalid.results)[0].region = RegionParamId(region);
            assert!(invalid.validate(&params, &result).is_err());
            assert!(
                contract
                    .outlives_region(RegionParamId(region), RegionParamId(0))
                    .is_err()
            );
        }
        let mut invalid = contract.clone();
        invalid.region_count = 65;
        assert!(invalid.closure().is_err());
        invalid = contract.clone();
        invalid.outlives = Arc::from([(RegionParamId(0), RegionParamId(63))]);
        assert!(invalid.closure().is_err());
        invalid = contract.clone();
        invalid.input_regions = fixture_input_regions(Arc::from([Some(RegionParamId(0))]));
        assert!(invalid.validate(&params, &result).is_err());
        let mut no_edges = contract.clone();
        no_edges.outlives = Arc::from([]);
        fixture_set_returns(
            &mut no_edges,
            ReturnContract::SharedInputPaths(empty_input_paths(&[])),
        );
        no_edges.validate(&params, &result).unwrap();
        assert!(
            !no_edges
                .outlives_region(RegionParamId(0), RegionParamId(2))
                .unwrap()
        );
        assert!(
            contract
                .validate(
                    &params,
                    &Ty::Ref {
                        kind: BorrowKind::Mut,
                        referent: Arc::new(Ty::Int(IntTy::I32))
                    }
                )
                .is_err()
        );
        assert!(RefContract::default().validate(&params, &result).is_err());
        RefContract::default().validate(&params, &Ty::Unit).unwrap();
        let canonical = RefContract {
            input_regions: fixture_input_regions(vec![None; params.len()].into()),
            ..Default::default()
        };
        canonical.validate(&params, &Ty::Unit).unwrap();
        let mut nested = params.clone();
        nested[2] = shared(shared(Ty::Int(IntTy::I32)));
        assert!(contract.validate(&nested, &result).is_err());
    }

    #[test]
    fn reference_copy_and_projection_contract() {
        use super::*;
        let shared = Ty::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Ty::Tuple(Arc::from([Ty::Int(IntTy::I32)]))),
        };
        let mutable = Ty::Ref {
            kind: BorrowKind::Mut,
            referent: Arc::new(Ty::Int(IntTy::I32)),
        };
        assert!(shared.is_copy());
        assert!(!mutable.is_copy());
        assert_eq!(
            projected_ty(&shared, &[Projection::Deref, Projection::Field(0)]).unwrap(),
            &Ty::Int(IntTy::I32)
        );
        assert!(projected_ty(&shared, &[Projection::Deref, Projection::Deref]).is_err());
        assert!(projected_ty(&Ty::Int(IntTy::I32), &[Projection::Deref]).is_err());
        assert!(!Ty::Tuple(Arc::from([mutable])).is_copy());
    }
    use super::*;

    #[test]
    fn arrays_are_structural_indexed_and_copy_depends_on_element_even_when_empty() {
        let scalar = Ty::Array {
            element: Arc::new(Ty::Int(IntTy::I32)),
            len: 2,
        };
        assert!(scalar.is_copy());
        assert_eq!(scalar.field_count(), None);
        assert_eq!(scalar.field_ty(0), None);
        assert_eq!(
            projected_ty(&scalar, &[Projection::ConstantIndex(1)]).unwrap(),
            &Ty::Int(IntTy::I32)
        );
        assert!(projected_ty(&scalar, &[Projection::Field(0)]).is_err());
        assert!(projected_ty(&scalar, &[Projection::ConstantIndex(2)]).is_err());
        let empty = Ty::Array {
            element: Arc::new(Ty::Int(IntTy::I32)),
            len: 0,
        };
        assert!(projected_ty(&empty, &[Projection::ConstantIndex(0)]).is_err());
        let noncopy = Ty::Struct(nominal(
            NominalAppId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            "S",
            vec![Ty::Int(IntTy::I32)],
        ));
        for len in [0, 1, 2] {
            assert!(
                !Ty::Array {
                    element: Arc::new(noncopy.clone()),
                    len
                }
                .is_copy()
            );
        }
        let mut work = 0;
        assert!(
            pattern_types_equal(
                &scalar,
                &Ty::Array {
                    element: Arc::new(Ty::Int(IntTy::I32)),
                    len: 2
                },
                &mut work
            )
            .unwrap()
        );
        assert!(!pattern_types_equal(&scalar, &empty, &mut work).unwrap());
        let mut function = fixture();
        function.locals[0].ty = noncopy.clone();
        function.exprs[0].ty = noncopy.clone();
        for len in [0, 1] {
            function.exprs.push(TypedExpr {
                ty: Ty::Array {
                    element: Arc::new(noncopy.clone()),
                    len,
                },
                completion: Completion::Returns,
                kind: ExprKind::Repeat {
                    value: ExprId(0),
                    len,
                },
            });
        }
        function.validate().unwrap();
        function.exprs.push(TypedExpr {
            ty: Ty::Array {
                element: Arc::new(noncopy),
                len: 2,
            },
            completion: Completion::Returns,
            kind: ExprKind::Repeat {
                value: ExprId(0),
                len: 2,
            },
        });
        assert!(function.validate().unwrap_err().contains("Copy"));
    }

    fn unit_variant(name: &str) -> VariantDef {
        VariantDef {
            name: name.into(),
            shape: VariantShape::Unit,
            fields: Arc::from([]),
        }
    }

    #[test]
    fn payload_projection_requires_owner_bound_downcast_field_pair() {
        let id = NominalAppId {
            snapshot: SnapshotId::allocate().unwrap(),
            index: 0,
        };
        let ty = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id,
            name: "E".into(),
            module: String::new(),
            public: false,
            variants: vec![
                VariantDef {
                    name: "A".into(),
                    shape: VariantShape::Tuple,
                    fields: vec![FieldDef {
                        name: "0".into(),
                        ty: Ty::Tuple(vec![Ty::Int(IntTy::I32)].into()),
                        public: true,
                    }]
                    .into(),
                },
                unit_variant("B"),
            ]
            .into(),
        });
        assert!(ty.has_payload());
        let root = Ty::Enum(ty);
        let variant = VariantId {
            owner: id,
            index: 0,
        };
        assert_eq!(
            projected_ty(
                &root,
                &[
                    Projection::Downcast(variant),
                    Projection::Field(0),
                    Projection::Field(0)
                ]
            )
            .unwrap(),
            &Ty::Int(IntTy::I32)
        );
        for projection in [
            vec![Projection::Field(0)],
            vec![Projection::Downcast(variant)],
            vec![Projection::Downcast(variant), Projection::Downcast(variant)],
            vec![
                Projection::Downcast(VariantId {
                    index: 1,
                    ..variant
                }),
                Projection::Field(0),
            ],
            vec![
                Projection::Downcast(VariantId {
                    owner: NominalAppId { index: 1, ..id },
                    ..variant
                }),
                Projection::Field(0),
            ],
        ] {
            assert!(projected_ty(&root, &projection).is_err(), "{projection:?}");
        }
        let empty_payload = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id,
            name: "Empty".into(),
            module: String::new(),
            public: false,
            variants: vec![VariantDef {
                name: "A".into(),
                shape: VariantShape::Struct,
                fields: Arc::from([]),
            }]
            .into(),
        });
        assert!(
            empty_payload.has_payload(),
            "zero-field non-unit shapes retain indirect ABI"
        );
    }

    #[test]
    fn deferred_declaration_references_require_nonparameter_locals() {
        let mut function = fixture();
        function
            .body
            .statements
            .push(Stmt::Declare { local: LocalId(1) });
        function.validate().unwrap();
        function
            .body
            .statements
            .push(Stmt::Declare { local: LocalId(0) });
        assert!(function.validate().unwrap_err().contains("parameter"));
        function.body.statements.pop();
        function.body.statements.push(Stmt::Declare {
            local: LocalId(usize::MAX),
        });
        assert!(function.validate().unwrap_err().contains("invalid local"));
    }

    #[test]
    fn enum_identity_copy_debug_and_variant_bounds() {
        let id = NominalAppId {
            snapshot: SnapshotId::allocate().unwrap(),
            index: 0,
        };
        let handle = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id,
            name: "é".repeat(1000),
            module: String::new(),
            public: false,
            variants: vec![unit_variant("A")].into(),
        });
        let same_id = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id,
            name: "Other".into(),
            module: String::new(),
            public: true,
            variants: vec![unit_variant("B")].into(),
        });
        assert_eq!(handle, same_id);
        let mut set = std::collections::HashSet::new();
        assert!(set.insert(handle.clone()));
        assert!(!set.insert(same_id));
        let ty = Ty::Enum(handle.clone());
        assert!(!ty.is_copy());
        assert!(!Ty::Tuple(vec![ty.clone()].into()).is_copy());
        assert_eq!(ty.field_count(), None);
        assert_eq!(ty.field_ty(0), None);
        assert!(format!("{handle:?}").len() < 512);
        let mut function = fixture();
        function.exprs.push(TypedExpr {
            ty,
            completion: Completion::Returns,
            kind: ExprKind::EnumVariant {
                ty: handle,
                variant: 1,
                fields: vec![],
            },
        });
        assert!(function.validate().unwrap_err().contains("enum variant"));
        for count in [0, MAX_ENUM_VARIANTS + 1] {
            let invalid = EnumTy::new(EnumDef {
                source_decl: test_decl(),
                type_args: Arc::from([]),
                id,
                name: "Invalid".into(),
                module: String::new(),
                public: false,
                variants: vec![unit_variant(""); count].into(),
            });
            assert!(invalid.validate().is_err());
        }
    }

    #[test]
    fn match_validation_bounds_and_checks_dead_patterns() {
        let function = fixture();
        let parameters = function.params.iter().copied().collect();
        let arm = MatchArm {
            pattern: MatchPattern::Wildcard,
            body: ExprId(0),
        };
        assert!(
            function
                .validate_match(ExprId(0), &[], &parameters)
                .is_err()
        );
        assert!(
            function
                .validate_match(
                    ExprId(0),
                    &vec![arm.clone(); MAX_MATCH_ARMS + 1],
                    &parameters
                )
                .is_err()
        );
        let invalid = MatchArm {
            pattern: MatchPattern::Binding(LocalId(usize::MAX)),
            body: ExprId(0),
        };
        assert!(
            function
                .validate_match(ExprId(0), &[arm.clone(), invalid], &parameters)
                .is_err()
        );
        let invalid = MatchArm {
            pattern: MatchPattern::Wildcard,
            body: ExprId(usize::MAX),
        };
        assert!(
            function
                .validate_match(ExprId(0), &[arm, invalid], &parameters)
                .is_err()
        );
    }

    #[test]
    fn flattened_pattern_plan_validates_types_and_disjoint_fresh_bindings() {
        let mut function = fixture();
        let ty = Ty::Tuple(vec![Ty::Int(IntTy::I32), Ty::Bool].into());
        function.locals[0].ty = ty.clone();
        function.exprs[0].ty = ty.clone();
        function.locals.push(LocalDecl {
            name: "first".into(),
            ty: Ty::Int(IntTy::I32),
            mutable: false,
        });
        let bindings = vec![
            PatternBinding {
                local: LocalId(2),
                projection: vec![0].into_iter().map(Projection::Field).collect(),
            },
            PatternBinding {
                local: LocalId(1),
                projection: vec![1].into_iter().map(Projection::Field).collect(),
            },
        ];
        function.body.statements = vec![Stmt::LetPattern {
            value: ExprId(0),
            ty: ty.clone(),
            bindings: bindings.clone(),
        }];
        function.validate().unwrap();
        function.validate_pattern_bindings(&ty, &[]).unwrap();
        let mut invalid = bindings.clone();
        invalid[1].local = LocalId(2);
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("distinct")
        );
        invalid[1].local = LocalId(0);
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("fresh")
        );
        invalid[1].local = LocalId(1);
        invalid[1].projection = vec![Projection::Field(2)];
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("projection")
        );
        invalid[1].projection = vec![Projection::Field(0)];
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("type mismatch")
        );
        function.locals.push(LocalDecl {
            name: "overlap".into(),
            ty: Ty::Int(IntTy::I32),
            mutable: false,
        });
        invalid[1].local = LocalId(3);
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("overlapping")
        );
        function.locals[3].ty = ty.clone();
        invalid[1].projection.clear();
        assert!(
            function
                .validate_pattern_bindings(&ty, &invalid)
                .unwrap_err()
                .contains("overlapping")
        );
    }

    #[test]
    fn flattened_pattern_plan_enforces_binding_depth_and_work_bounds() {
        let mut function = fixture();
        let binding = PatternBinding {
            local: LocalId(1),
            projection: vec![],
        };
        let oversized = vec![binding.clone(); MAX_PATTERN_BINDINGS + 1];
        assert!(
            function
                .validate_pattern_bindings(&Ty::Bool, &oversized)
                .unwrap_err()
                .contains("binding limit")
        );
        let deep = PatternBinding {
            local: LocalId(1),
            projection: vec![Projection::Field(0); MAX_PATTERN_PROJECTION_DEPTH + 1],
        };
        assert!(
            function
                .validate_pattern_bindings(&Ty::Bool, &[deep])
                .unwrap_err()
                .contains("depth")
        );

        let mut root = Ty::Int(IntTy::I32);
        for _ in 0..MAX_PATTERN_PROJECTION_DEPTH {
            root = Ty::Tuple(vec![root].into());
        }
        let count = MAX_PATTERN_PROJECTION_WORK / (MAX_PATTERN_PROJECTION_DEPTH + 1) + 1;
        function.locals.truncate(1);
        let mut bindings = Vec::with_capacity(count);
        for index in 0..count {
            function.locals.push(LocalDecl {
                name: format!("b{index}"),
                ty: Ty::Int(IntTy::I32),
                mutable: false,
            });
            bindings.push(PatternBinding {
                local: LocalId(index + 1),
                projection: vec![Projection::Field(0); MAX_PATTERN_PROJECTION_DEPTH],
            });
        }
        assert!(
            function
                .validate_pattern_bindings(&root, &bindings)
                .unwrap_err()
                .contains("work limit")
        );
    }

    #[test]
    fn pattern_type_equality_fails_closed_on_independently_allocated_shared_dags() {
        let mut a = Ty::Int(IntTy::I32);
        let mut b = Ty::Int(IntTy::I32);
        for _ in 0..32 {
            a = Ty::Tuple(vec![a.clone(), a].into());
            b = Ty::Tuple(vec![b.clone(), b].into());
        }
        assert!(a.is_copy());
        assert!(b.is_copy());
        let mut function = fixture();
        function.locals[1].ty = a;
        let bindings = [PatternBinding {
            local: LocalId(1),
            projection: vec![],
        }];
        assert!(
            function
                .validate_pattern_bindings(&b, &bindings)
                .unwrap_err()
                .contains("work limit")
        );
    }

    fn nominal(id: NominalAppId, name: &str, fields: Vec<Ty>) -> StructTy {
        StructTy::new(StructDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id,
            name: name.into(),
            module: String::new(),
            public: false,
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, ty)| FieldDef {
                    name: format!("f{index}"),
                    ty,
                    public: false,
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    #[test]
    fn nominal_identity_is_not_name_shape_or_declaration_index_alone() {
        let snapshot = SnapshotId::allocate().unwrap();
        let first = nominal(
            NominalAppId { snapshot, index: 0 },
            "S",
            vec![Ty::Int(IntTy::I32)],
        );
        let same_shape = nominal(
            NominalAppId { snapshot, index: 1 },
            "S",
            vec![Ty::Int(IntTy::I32)],
        );
        let next_snapshot = nominal(
            NominalAppId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            "S",
            vec![Ty::Int(IntTy::I32)],
        );
        assert_eq!(first, first.clone());
        assert_ne!(first, same_shape);
        assert_ne!(first, next_snapshot);
        let mut identities = std::collections::HashSet::new();
        assert!(identities.insert(first.clone()));
        assert!(!identities.insert(first));
        assert!(identities.insert(same_shape));
        assert!(identities.insert(next_snapshot));
    }

    #[test]
    fn aggregate_helpers_preserve_nominal_non_copy_and_projected_copy() {
        let handle = nominal(
            NominalAppId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            "S",
            vec![
                Ty::Int(IntTy::I32),
                Ty::Tuple(vec![Ty::Bool, Ty::Unit].into()),
            ],
        );
        assert_eq!(handle.field_index("f1"), Some(1));
        assert_eq!(handle.field_index("missing"), None);
        let ty = Ty::Struct(handle);
        assert_eq!(ty.field_count(), Some(2));
        assert_eq!(ty.field_ty(0), Some(&Ty::Int(IntTy::I32)));
        assert_eq!(ty.field_ty(2), None);
        assert!(!ty.is_copy());
        assert!(ty.field_ty(1).unwrap().is_copy());
        assert!(Ty::Tuple(vec![Ty::Bool, Ty::Unit].into()).is_copy());
        assert!(!Ty::Tuple(vec![Ty::Bool, ty].into()).is_copy());
        assert_eq!(Ty::Int(IntTy::I32).field_count(), None);
        assert_eq!(Ty::Int(IntTy::I32).field_ty(0), None);
    }

    #[test]
    fn snapshot_allocation_fails_at_exhaustion_without_wrapping() {
        let next = AtomicU64::new(u64::MAX - 1);
        assert_eq!(SnapshotId::allocate_from(&next).unwrap().0, u64::MAX - 1);
        assert!(SnapshotId::allocate_from(&next).is_err());
        assert!(SnapshotId::allocate_from(&next).is_err());
        assert_eq!(next.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn snapshot_allocation_is_unique_across_threads() {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..32)
                        .map(|_| SnapshotId::allocate().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let ids: std::collections::HashSet<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(ids.len(), 256);
    }

    #[test]
    fn nominal_debug_bounds_unicode_names_and_does_not_expand_shared_fields() {
        let snapshot = SnapshotId::allocate().unwrap();
        let mut ty = Ty::Int(IntTy::I32);
        for index in 0..16 {
            ty = Ty::Struct(nominal(
                NominalAppId { snapshot, index },
                &"é".repeat(1000),
                vec![ty.clone(), ty],
            ));
        }
        let diagnostic = format!("{ty:?}");
        assert!(diagnostic.len() < 512, "unbounded nominal diagnostic");
        assert!(!diagnostic.contains("fields"));
        assert!(diagnostic.contains("NominalAppId"));
    }

    #[test]
    fn tuple_types_are_structural_and_cheaply_cloned() {
        let fields: Arc<[Ty]> = vec![
            Ty::Int(IntTy::I32),
            Ty::Tuple(vec![Ty::Bool, Ty::Unit].into()),
        ]
        .into();
        let ty = Ty::Tuple(fields.clone());
        let expression = TypedExpr {
            ty: ty.clone(),
            completion: Completion::Returns,
            kind: ExprKind::Tuple(vec![]),
        };
        let Some(Ty::Tuple(cloned_fields)) = expression.flow() else {
            panic!("tuple flow missing")
        };
        assert!(Arc::ptr_eq(&fields, &cloned_fields));
        assert_eq!(
            ty,
            Ty::Tuple(
                vec![
                    Ty::Int(IntTy::I32),
                    Ty::Tuple(vec![Ty::Bool, Ty::Unit].into())
                ]
                .into()
            )
        );
        assert_ne!(ty, Ty::Tuple(vec![Ty::Bool, Ty::Int(IntTy::I32)].into()));
        assert_ne!(Ty::Unit, Ty::Tuple(vec![Ty::Unit].into()));
    }

    #[test]
    fn tuple_field_and_assignment_references_are_checked() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Tuple(vec![Ty::Int(IntTy::I32)].into()),
            completion: Completion::Returns,
            kind: ExprKind::Tuple(vec![ExprId(0)]),
        });
        value.exprs.push(TypedExpr {
            ty: Ty::Int(IntTy::I32),
            completion: Completion::Returns,
            kind: ExprKind::Field {
                base: ExprId(1),
                index: 0,
            },
        });
        value.body.statements.push(Stmt::AssignPlace {
            place: Place {
                local: LocalId(0),
                projection: vec![0, 1].into_iter().map(Projection::Field).collect(),
            },
            value: ExprId(2),
        });
        // Structural validation intentionally does not redo projection typing.
        value.validate().unwrap();
        value.exprs[2].kind = ExprKind::Field {
            base: ExprId(99),
            index: 0,
        };
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[2].kind = ExprKind::Tuple(vec![ExprId(99)]);
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[2].kind = ExprKind::Int(IntValue::i32(0));
        value.body.statements.push(Stmt::AssignPlace {
            place: Place {
                local: LocalId(99),
                projection: vec![0].into_iter().map(Projection::Field).collect(),
            },
            value: ExprId(2),
        });
        assert!(value.validate().unwrap_err().contains("invalid local 99"));
    }

    fn fixture() -> CheckedFunction {
        CheckedFunction {
            ref_contract: Default::default(),
            name: "f".into(),
            params: vec![LocalId(0)],
            result: Ty::Unit,
            locals: vec![
                LocalDecl {
                    name: "x".into(),
                    ty: Ty::Int(IntTy::I32),
                    mutable: false,
                },
                LocalDecl {
                    name: "x".into(),
                    ty: Ty::Bool,
                    mutable: true,
                },
            ],
            exprs: vec![TypedExpr {
                ty: Ty::Int(IntTy::I32),
                completion: Completion::Returns,
                kind: ExprKind::Local(LocalId(0)),
            }],
            body: Block {
                statements: vec![Stmt::Expr(ExprId(0))],
                tail: None,
            },
        }
    }

    #[test]
    fn divergence_is_not_unit_flow() {
        let value = TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Block(Block {
                statements: vec![Stmt::Return(None)],
                tail: None,
            }),
        };
        assert_eq!(value.flow(), None);
        assert_eq!(value.ty, Ty::Never);
        let value = TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Returns,
            ..value
        };
        assert_eq!(value.flow(), Some(Ty::Unit));
    }

    #[test]
    fn unreachable_typed_tail_retains_its_semantic_type() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Int(IntTy::I32),
            completion: Completion::Never,
            kind: ExprKind::Block(Block {
                statements: vec![Stmt::Return(None)],
                tail: Some(ExprId(0)),
            }),
        });
        assert_eq!(value.exprs[1].ty, Ty::Int(IntTy::I32));
        assert_eq!(value.exprs[1].flow(), None);
        assert_ne!(value.exprs[1].ty, Ty::Never);
        value.validate().unwrap();

        // A loop's break operand can have a concrete type without completing.
        // Its semantic result type must not be overwritten by its completion.
        value.exprs.push(TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Break {
                target: LoopId(0),
                value: Some(ExprId(1)),
            },
        });
        value.exprs.push(TypedExpr {
            ty: Ty::Int(IntTy::I32),
            completion: Completion::Never,
            kind: ExprKind::Loop {
                id: LoopId(0),
                body: Block {
                    statements: vec![Stmt::Expr(ExprId(2))],
                    tail: None,
                },
            },
        });
        assert_eq!(value.exprs[3].ty, Ty::Int(IntTy::I32));
        assert_eq!(value.exprs[3].flow(), None);
        value.validate().unwrap();
    }

    #[test]
    fn display_names_do_not_define_local_identity() {
        let value = fixture();
        assert_eq!(value.locals[0].name, value.locals[1].name);
        assert_ne!(LocalId(0), LocalId(1));
        value.validate().unwrap();
    }

    #[test]
    fn structural_validation_rejects_invalid_references_and_parameters() {
        let mut value = fixture();
        value.body.tail = Some(ExprId(1));
        assert!(value.validate().unwrap_err().contains("invalid expression"));
        value.body.tail = None;
        value.params.push(LocalId(0));
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("duplicate parameter")
        );
        value.params = vec![LocalId(2)];
        assert!(value.validate().unwrap_err().contains("invalid local"));
    }

    #[test]
    fn structural_validation_rejects_call_arity() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Returns,
            kind: ExprKind::Call {
                target: test_target(),
                name: "g".into(),
                args: vec![],
                signature: Signature {
                    ref_contract: Default::default(),
                    params: vec![Ty::Int(IntTy::I32)],
                    result: Ty::Unit,
                },
            },
        });
        assert!(value.validate().unwrap_err().contains("invalid call arity"));
    }

    #[test]
    fn loop_body_and_break_operand_references_are_checked() {
        let mut value = fixture();
        value.exprs.push(TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Loop {
                id: LoopId(0),
                body: Block {
                    statements: vec![],
                    tail: Some(ExprId(99)),
                },
            },
        });
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[1].kind = ExprKind::Break {
            target: LoopId(0),
            value: Some(ExprId(99)),
        };
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        value.exprs[1].kind = ExprKind::Break {
            target: LoopId(0),
            value: Some(ExprId(0)),
        };
        assert_eq!(value.exprs[1].flow(), None);
        value.validate().unwrap();
        value.exprs[1].kind = ExprKind::Continue { target: LoopId(0) };
        assert_eq!(value.exprs[1].flow(), None);
        value.validate().unwrap();
    }

    #[test]
    fn while_body_references_are_checked() {
        let mut value = fixture();
        value.body.statements.push(Stmt::While {
            id: LoopId(0),
            condition: ExprId(0),
            body: Block {
                statements: vec![Stmt::Expr(ExprId(99))],
                tail: None,
            },
        });
        assert!(
            value
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
    }
}
