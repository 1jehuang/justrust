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
#[path = "ir/closure.rs"]
mod closure;
pub use closure::{ClosureKind, ClosureTy};
#[path = "ir/intrinsics.rs"]
mod intrinsics;
#[cfg(test)]
#[path = "ir/root_or_tests.rs"]
mod root_or_tests;
pub use intrinsics::{Intrinsic, IntrinsicOp, IntrinsicSignature, ScalarTy};

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
    Const,
    Struct,
    Enum,
    Impl,
    AssociatedFunction,
    Trait,
    TraitItem,
    AssociatedType,
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
pub struct AssociatedTypeProjection {
    pub trait_ref: TraitRef,
    pub item: DeclId,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Predicate {
    Implements(TraitRef),
    ProjectionEq {
        projection: AssociatedTypeProjection,
        ty: Ty,
    },
}
impl Predicate {
    /// The complete trait application, not evidence of membership by itself.
    pub fn trait_ref(&self) -> &TraitRef {
        match self {
            Self::Implements(trait_ref) => trait_ref,
            Self::ProjectionEq { projection, .. } => &projection.trait_ref,
        }
    }

    /// Every type supplied by this predicate, including an equality's RHS.
    /// Visitors must validate all of these before imposing capability limits.
    pub fn metadata_types(&self) -> impl Iterator<Item = &Ty> {
        let trait_ref = self.trait_ref();
        let rhs = match self {
            Self::Implements(_) => None,
            Self::ProjectionEq { ty, .. } => Some(ty),
        };
        std::iter::once(&trait_ref.self_ty)
            .chain(trait_ref.args.iter())
            .chain(rhs)
    }
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
    /// Dynamic dispatch through a trait object vtable. Argument 0 is the
    /// `&dyn`/`&mut dyn` receiver carrier; `slot` indexes the principal
    /// trait's method entries (declaration order, supertraits first).
    Virtual(VirtualCall),
}
/// `call.trait_ref.self_ty` is the exact `Ty::Dyn` receiver object type and
/// `call.trait_ref.declaration` the trait owning `call.item` (the principal or
/// one of its supertraits). The slot is derived from the principal's vtable
/// layout (`vtable_methods`) by the backend, never stored.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VirtualCall {
    pub call: TraitCall,
}
impl VirtualCall {
    pub fn principal(&self) -> Result<DeclId, String> {
        match &self.call.trait_ref.self_ty {
            Ty::Dyn(object) => Ok(object.principal),
            _ => Err("virtual call receiver is not a trait object".into()),
        }
    }
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
            Self::TraitItem(call) | Self::Virtual(VirtualCall { call }) => &call.method_args,
        }
    }
    pub fn metadata_types(&self) -> impl Iterator<Item = &Ty> {
        let (self_ty, trait_args): (Option<&Ty>, &[Ty]) = match self {
            Self::Direct(_) => (None, &[]),
            Self::TraitItem(call) | Self::Virtual(VirtualCall { call }) => {
                (Some(&call.trait_ref.self_ty), &call.trait_ref.args)
            }
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

/// Typed value coordinates. These are not memory-place projections or proof.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CarrierChannel {
    Fixed(ReferencePosition),
    ParamTail { parameter: ParamId, prefix: u8 },
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CarrierStep {
    Field(usize),
    ConstantIndex(usize),
    VariantField { variant: VariantId, field: usize },
    Referent,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CarrierPath(pub Arc<[CarrierStep]>);
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CarrierLocation {
    pub path: CarrierPath,
    pub channel: CarrierChannel,
}

/// One canonical typed component walk for world contracts and the SAME Plan.
/// Presence memoization avoids expanding reference-free shared type DAGs.
pub fn carrier_locations(ty: &Ty, budget: &PlanBudget) -> Result<Arc<[CarrierLocation]>, String> {
    reject_associated_types(ty, Some(budget))?;
    fn present(
        ty: &Ty,
        budget: &PlanBudget,
        depth: usize,
        memo: &mut std::collections::HashMap<usize, (bool, usize)>,
    ) -> Result<(bool, usize), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        let key = std::ptr::from_ref(ty) as usize;
        if let Some(&(has, height)) = memo.get(&key) {
            budget.check_depth(depth.checked_add(height).ok_or("carrier depth overflow")?)?;
            return Ok((has, height));
        }
        let mut has = matches!(ty, Ty::Ref { .. } | Ty::Param(_));
        let mut height = 0;
        let mut child = |ty: &Ty| -> Result<(), String> {
            let (found, h) = present(ty, budget, depth + 1, memo)?;
            has |= found;
            height = height.max(h + 1);
            Ok(())
        };
        match ty {
            Ty::Closure(closure) => {
                for ty in closure.captures() {
                    child(ty)?;
                }
            }
            Ty::AssociatedType(_) => {
                return Err("unsupported: residual associated type projection in carrier".into());
            }
            Ty::Ref { referent, .. }
            | Ty::Array {
                element: referent, ..
            } => child(referent)?,
            Ty::Str => {}
            Ty::Slice { element } => {
                // A slice descriptor carries its outer loan, not channels
                // hidden inside an element. Concrete element references are
                // rejected again after substitution, never given a fake tail.
                child(element)?;
                reject_references(element, budget)?;
                has = false;
            }
            Ty::Tuple(fields) => {
                for ty in fields.iter() {
                    child(ty)?;
                }
            }
            Ty::Struct(value) => {
                for field in value.def().fields.iter() {
                    child(&field.ty)?;
                }
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in variant.fields.iter() {
                        child(&field.ty)?;
                    }
                }
            }
            // Raw pointers carry no loan or provenance channel.
            Ty::RawPtr { .. } => {}
            Ty::Param(_)
            | Ty::Int(_)
            | Ty::Float(_)
            | Ty::Char
            | Ty::Bool
            | Ty::Unit
            | Ty::Dyn(_)
            | Ty::Never => {}
        }
        budget.retain_bytes(4 * std::mem::size_of::<(usize, (bool, usize))>())?;
        memo.try_reserve(1)
            .map_err(|_| "carrier presence allocation resource limit")?;
        memo.insert(key, (has, height));
        Ok((has, height))
    }
    fn emit(
        channel: CarrierChannel,
        path: &[CarrierStep],
        output: &mut Vec<CarrierLocation>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if output.len() >= MAX_LOAN_SITES {
            return Err("carrier component resource limit exceeded".into());
        }
        budget.charge_nodes(1)?;
        budget.charge_work(path.len().saturating_add(output.len()).saturating_add(1))?;
        let bytes = path
            .len()
            .checked_mul(2 * std::mem::size_of::<CarrierStep>())
            .and_then(|n| {
                n.checked_add(
                    3 * std::mem::size_of::<CarrierLocation>() + 4 * std::mem::size_of::<usize>(),
                )
            })
            .ok_or("carrier location size overflow")?;
        budget.retain_bytes(bytes)?;
        output
            .try_reserve(1)
            .map_err(|_| "carrier location allocation resource limit")?;
        let mut copied = Vec::new();
        copied
            .try_reserve_exact(path.len())
            .map_err(|_| "carrier path allocation resource limit")?;
        copied.extend_from_slice(path);
        output.push(CarrierLocation {
            path: CarrierPath(copied.into()),
            channel,
        });
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn walk(
        ty: &Ty,
        path: &mut Vec<CarrierStep>,
        prefix: u8,
        depth: usize,
        output: &mut Vec<CarrierLocation>,
        budget: &PlanBudget,
        memo: &std::collections::HashMap<usize, (bool, usize)>,
    ) -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        if !memo
            .get(&(std::ptr::from_ref(ty) as usize))
            .is_some_and(|(has, _)| *has)
        {
            return Ok(());
        }
        match ty {
            Ty::Ref { referent, .. } => {
                emit(
                    CarrierChannel::Fixed(ReferencePosition { depth: prefix }),
                    path,
                    output,
                    budget,
                )?;
                walk(
                    referent,
                    path,
                    prefix
                        .checked_add(1)
                        .ok_or("carrier reference depth overflow")?,
                    depth + 1,
                    output,
                    budget,
                    memo,
                )?;
            }
            Ty::Param(parameter) => emit(
                CarrierChannel::ParamTail {
                    parameter: *parameter,
                    prefix,
                },
                path,
                output,
                budget,
            )?,
            _ => {
                let saved = path.len();
                for _ in 0..prefix {
                    path.push(CarrierStep::Referent);
                }
                let mut child = |step, child: &Ty| -> Result<(), String> {
                    path.push(step);
                    let result = walk(child, path, 0, depth + 1, output, budget, memo);
                    path.pop();
                    result
                };
                match ty {
                    Ty::Closure(closure) => {
                        for (index, ty) in closure.captures().iter().enumerate() {
                            child(CarrierStep::Field(index), ty)?;
                        }
                    }
                    Ty::AssociatedType(_) => {
                        return Err(
                            "unsupported: residual associated type projection in carrier".into(),
                        );
                    }
                    Ty::Tuple(fields) => {
                        for (index, ty) in fields.iter().enumerate() {
                            child(CarrierStep::Field(index), ty)?;
                        }
                    }
                    Ty::Array { element, len } => {
                        if *len > MAX_ARRAY_LEN {
                            return Err("array carrier resource limit exceeded".into());
                        }
                        for index in 0..*len {
                            child(CarrierStep::ConstantIndex(index), element)?;
                        }
                    }
                    Ty::Slice { .. } => {
                        // Reached only when the element type carries references.
                        return Err("reference-carrying slice elements are unsupported".into());
                    }
                    Ty::Struct(value) => {
                        for (index, field) in value.def().fields.iter().enumerate() {
                            child(CarrierStep::Field(index), &field.ty)?;
                        }
                    }
                    Ty::Enum(value) => {
                        for (variant, definition) in value.def().variants.iter().enumerate() {
                            for (field, ty) in definition.fields.iter().enumerate() {
                                child(
                                    CarrierStep::VariantField {
                                        variant: VariantId {
                                            owner: value.def().id,
                                            index: variant,
                                        },
                                        field,
                                    },
                                    &ty.ty,
                                )?;
                            }
                        }
                    }
                    Ty::Int(_)
                    | Ty::Float(_)
                    | Ty::Char
                    | Ty::Str
                    | Ty::Bool
                    | Ty::Unit
                    | Ty::Dyn(_)
                    | Ty::Never
                    | Ty::RawPtr { .. } => {}
                    Ty::Ref { .. } | Ty::Param(_) => unreachable!(),
                }
                path.truncate(saved);
            }
        }
        Ok(())
    }
    let mut memo = std::collections::HashMap::new();
    present(ty, budget, 0, &mut memo)?;
    let _scratch = budget.reserve_bytes(129 * std::mem::size_of::<CarrierStep>())?;
    let mut path = Vec::new();
    path.try_reserve_exact(129)
        .map_err(|_| "carrier path allocation resource limit")?;
    let mut output = Vec::new();
    walk(ty, &mut path, 0, 0, &mut output, budget, &memo)?;
    Ok(output.into())
}

pub fn carrier_at<'a>(
    mut ty: &'a Ty,
    location: &CarrierLocation,
    budget: &PlanBudget,
) -> Result<&'a Ty, String> {
    reject_associated_types(ty, Some(budget))?;
    for step in location.path.0.iter() {
        budget.charge_work(1)?;
        ty = match (step, ty) {
            (CarrierStep::Referent, Ty::Ref { referent, .. }) => referent,
            (CarrierStep::Field(index), _) => {
                ty.field_ty(*index).ok_or("carrier field outside type")?
            }
            (CarrierStep::ConstantIndex(index), Ty::Array { element, len }) if index < len => {
                element
            }
            (CarrierStep::VariantField { variant, field }, Ty::Enum(value))
                if variant.owner == value.def().id =>
            {
                &value
                    .def()
                    .variants
                    .get(variant.index)
                    .and_then(|v| v.fields.get(*field))
                    .ok_or("carrier variant outside type")?
                    .ty
            }
            _ => return Err("carrier value path does not match type".into()),
        };
    }
    let count = match &location.channel {
        CarrierChannel::Fixed(position) => position.depth,
        CarrierChannel::ParamTail { prefix, .. } => *prefix,
    };
    for _ in 0..count {
        budget.charge_work(1)?;
        let Ty::Ref { referent, .. } = ty else {
            return Err("carrier prefix outside type".into());
        };
        ty = referent;
    }
    match (&location.channel, ty) {
        (CarrierChannel::Fixed(_), Ty::Ref { .. }) => Ok(ty),
        (CarrierChannel::ParamTail { parameter, .. }, Ty::Param(actual)) if parameter == actual => {
            Ok(ty)
        }
        _ => Err("carrier channel does not match typed component".into()),
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputCarrier {
    pub param: usize,
    pub location: CarrierLocation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarrierReturn {
    pub location: CarrierLocation,
    pub sources: Arc<[InputCarrier]>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarrierWriteEnvelope {
    pub destination: InputCarrier,
    pub sources: Arc<[InputCarrier]>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarrierContract {
    pub inputs: Arc<[InputCarrier]>,
    pub results: Arc<[CarrierReturn]>,
    pub writes: Arc<[CarrierWriteEnvelope]>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CarrierRegionTerm {
    Fixed(RegionParamId),
    Named(SourceRegionId),
    Family { parameter: ParamId },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarrierRegionConstraint {
    pub longer: CarrierRegionTerm,
    pub shorter: CarrierRegionTerm,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarrierRegionSchema {
    pub constraints: Arc<[CarrierRegionConstraint]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceRegionId {
    pub declaration: DeclId,
    pub index: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActualRegion {
    Named(SourceRegionId),
    Elided,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRegionEnv {
    pub declaration: DeclId,
    pub region_count: u16,
    pub outlives: Arc<[(SourceRegionId, SourceRegionId)]>,
    pub normalized: Arc<[Option<RegionParamId>]>,
    pub carriers: Arc<[SourceCarrierRegion]>,
}
/// A source lifetime bound to one exact carrier occurrence in its declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCarrierRegion {
    /// `None` identifies the result, otherwise this is the canonical input index.
    pub parameter: Option<usize>,
    pub location: CarrierLocation,
    pub region: CarrierRegionTerm,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallExpectation {
    NoFirmReference,
    FirmReference { kind: BorrowKind },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallArgumentTrace {
    pub argument: usize,
    pub value: ExprId,
    pub expectation: CallExpectation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeActualRegion {
    pub argument: usize,
    pub location: CarrierLocation,
    pub source: ActualRegion,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallEvidence {
    pub call: ExprId,
    pub arguments: Arc<[CallArgumentTrace]>,
    pub type_actual_regions: Arc<[TypeActualRegion]>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NominalCarrierMapping {
    pub original: NominalAppId,
    pub instantiated: NominalAppId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CarrierComponentMapping {
    pub original_parameter: ParamId,
    pub original: CarrierLocation,
    pub substituted: CarrierLocation,
}

/// Exact original-stage evidence. Only the validating source publication path
/// may construct this capsule. It is retained outside the source it owns.
#[derive(Clone, Debug)]
pub struct ValidatedCallEvidence {
    source: Arc<CheckedFunction>,
    call: ExprId,
    facts: Arc<super::traits::ValidatedFacts>,
    trace: CallEvidence,
    // Source-only replay ownership. Never retain the enclosing publication here.
    original: Option<Arc<super::source_replay::ValidatedOriginalCalls>>,
}
impl ValidatedCallEvidence {
    pub fn source(&self) -> &Arc<CheckedFunction> {
        &self.source
    }
    pub fn call(&self) -> ExprId {
        self.call
    }
    pub fn facts(&self) -> &Arc<super::traits::ValidatedFacts> {
        &self.facts
    }
    pub fn trace(&self) -> &CallEvidence {
        &self.trace
    }
    pub(super) fn original(&self) -> Option<&Arc<super::source_replay::ValidatedOriginalCalls>> {
        self.original.as_ref()
    }
}

/// Bind only a completed, exact canonical source publication. Raw typed IR uses
/// `bind_call_evidence` and cannot mint source inference permission.
pub(super) fn validated_call_capsules(
    publication: &Arc<super::source_replay::CanonicalSourcePublication>,
    budget: &PlanBudget,
) -> Result<Arc<[Arc<ValidatedCallEvidence>]>, super::traits::FactsError> {
    use super::traits::FactsError;
    let source = publication.source();
    let facts = publication.facts();
    publication.validate_source(source, facts, budget)?;
    validate_published_source(publication, budget)?;
    let count = source.call_evidence.len();
    let (nodes, bytes) = original_capsule_allocation(count, budget)?;
    budget
        .charge_nodes(nodes)
        .map_err(|error| FactsError::resource(budget, &error))?;
    budget
        .retain_bytes(bytes)
        .map_err(|error| FactsError::resource(budget, &error))?;
    let mut capsules = Vec::new();
    capsules
        .try_reserve_exact(count)
        .map_err(|_| FactsError::resource(budget, "original capsule allocation resource limit"))?;
    for trace in source.call_evidence.iter() {
        budget
            .charge_work(1)
            .map_err(|error| FactsError::resource(budget, &error))?;
        capsules.push(Arc::new(ValidatedCallEvidence {
            source: source.clone(),
            call: trace.call,
            facts: facts.clone(),
            trace: trace.clone(),
            original: Some(publication.replay().clone()),
        }));
    }
    Ok(capsules.into())
}

fn original_capsule_allocation(
    count: usize,
    budget: &PlanBudget,
) -> Result<(usize, usize), super::traits::FactsError> {
    use super::traits::FactsError;
    let nodes = count
        .checked_add(1)
        .ok_or_else(|| FactsError::resource(budget, "original capsule node overflow"))?;
    let header = 2usize
        .checked_mul(std::mem::size_of::<usize>())
        .ok_or_else(|| FactsError::resource(budget, "original capsule header overflow"))?;
    let row = std::mem::size_of::<ValidatedCallEvidence>()
        .checked_add(
            6usize
                .checked_mul(std::mem::size_of::<usize>())
                .ok_or_else(|| FactsError::resource(budget, "original capsule row overflow"))?,
        )
        .ok_or_else(|| FactsError::resource(budget, "original capsule row overflow"))?;
    let bytes = count
        .checked_mul(row)
        .and_then(|bytes| bytes.checked_add(header))
        .ok_or_else(|| FactsError::resource(budget, "original capsule storage overflow"))?;
    Ok((nodes, bytes))
}

/// Mandatory final validation before a canonical source publication is minted.
pub(super) fn validate_source_candidate(
    source: &Arc<CheckedFunction>,
    facts: &Arc<super::traits::ValidatedFacts>,
    replayed: &super::source_replay::ReplayedOriginalCalls,
    budget: &PlanBudget,
) -> Result<(), super::traits::FactsError> {
    use super::traits::FactsError;
    replayed.validate_binding(source, facts, budget)?;
    source
        .preflight_plan(FactsCtx::from_validated(facts), budget)
        .map_err(|error| FactsError::from_message(budget, &error))?;
    if source.body_substitution.is_some() {
        return Err(FactsError::invalid(
            budget,
            "source candidate is a generated body",
        ));
    }
    source
        .validate_with_evidence(
            FactsCtx::from_validated(facts),
            CallEvidencePolicy::Original(replayed.validated()),
        )
        .map_err(|error| FactsError::from_message(budget, &error))
}

/// Reuse the full factory validation only for its exact immutable publication.
/// This is not a validator for supplied or subsequently mutated checked payloads.
pub(super) fn validate_published_source(
    publication: &Arc<super::source_replay::CanonicalSourcePublication>,
    budget: &PlanBudget,
) -> Result<(), super::traits::FactsError> {
    use super::traits::FactsError;
    publication.validate_source(publication.source(), publication.facts(), budget)?;
    if publication.source().body_substitution.is_some() {
        return Err(FactsError::invalid(
            budget,
            "source publication is a generated body",
        ));
    }
    Ok(())
}

pub fn bind_call_evidence(
    source: Arc<CheckedFunction>,
    facts: Arc<super::traits::ValidatedFacts>,
    budget: &PlanBudget,
) -> Result<Arc<[Arc<ValidatedCallEvidence>]>, String> {
    if !facts.budget().same_authority(budget) {
        return Err("call evidence changed planning authority".into());
    }
    if let Some(proof) = &source.body_substitution {
        proof.validate(&source, &facts, budget)?;
        return Ok(proof.template.function().call_evidence.clone());
    }
    validate_call_evidence(&source, &facts, budget)?;
    budget.retain_bytes(
        source
            .call_evidence
            .len()
            .checked_mul(
                std::mem::size_of::<ValidatedCallEvidence>() + 6 * std::mem::size_of::<usize>(),
            )
            .ok_or("call evidence storage overflow")?,
    )?;
    let mut capsules = Vec::new();
    capsules
        .try_reserve_exact(source.call_evidence.len())
        .map_err(|_| "call evidence allocation resource limit")?;
    for trace in source.call_evidence.iter() {
        budget.charge_work(1)?;
        capsules.push(Arc::new(ValidatedCallEvidence {
            source: source.clone(),
            call: trace.call,
            facts: facts.clone(),
            trace: trace.clone(),
            original: None,
        }));
    }
    Ok(capsules.into())
}

#[derive(Clone, Copy)]
enum CallEvidencePolicy<'a> {
    Legacy,
    Original(&'a super::source_replay::ValidatedOriginalCalls),
}

pub(super) fn validate_call_evidence(
    source: &CheckedFunction,
    facts: &super::traits::ValidatedFacts,
    budget: &PlanBudget,
) -> Result<(), String> {
    validate_call_evidence_with_policy(source, facts, budget, CallEvidencePolicy::Legacy)
}

fn validate_call_evidence_with_policy(
    source: &CheckedFunction,
    facts: &super::traits::ValidatedFacts,
    budget: &PlanBudget,
    policy: CallEvidencePolicy<'_>,
) -> Result<(), String> {
    if let Some(proof) = &source.body_substitution {
        return proof.validate(source, facts, budget);
    }
    let _scratch = budget.reserve_bytes(
        source
            .exprs
            .len()
            .checked_mul(std::mem::size_of::<bool>())
            .ok_or("call evidence index overflow")?,
    )?;
    let mut seen = vec![false; source.exprs.len()];
    for evidence in source.call_evidence.iter() {
        budget.charge_work(1)?;
        let slot = seen
            .get_mut(evidence.call.0)
            .ok_or("call evidence outside arena")?;
        if *slot {
            return Err("duplicate call evidence".into());
        }
        *slot = true;
        let ExprKind::Call {
            signature,
            args,
            target,
            ..
        } = &source.exprs[evidence.call.0].kind
        else {
            return Err("call evidence does not name a call".into());
        };
        facts
            .service()
            .validate_signature(SignatureSubject::Call(target), signature, facts)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        if evidence.arguments.len() != args.len() || args.len() != signature.params.len() {
            return Err("call evidence argument arity mismatch".into());
        }
        for (index, trace) in evidence.arguments.iter().enumerate() {
            budget.charge_work(1)?;
            let legacy_expected = match &signature.params[index] {
                Ty::Ref { kind, .. } => CallExpectation::FirmReference { kind: *kind },
                _ => CallExpectation::NoFirmReference,
            };
            let expected = match policy {
                CallEvidencePolicy::Legacy => legacy_expected,
                CallEvidencePolicy::Original(replay) => replay
                    .expected(evidence.call, index, budget)
                    .map_err(|error| error.into_legacy_diagnostic(budget))?
                    .unwrap_or(legacy_expected),
            };
            if trace.argument != index
                || trace.value != args[index]
                || trace.expectation != expected
            {
                return Err(
                    "call evidence does not match original firm argument expectation".into(),
                );
            }
            source
                .exprs
                .get(trace.value.0)
                .ok_or("call evidence argument outside arena")?;
        }
        let actuals: &[Ty] = target.type_arguments();
        for (index, region) in evidence.type_actual_regions.iter().enumerate() {
            budget.charge_work(index.saturating_add(1))?;
            for old in evidence.type_actual_regions[..index].iter() {
                budget.charge_work(
                    old.location
                        .path
                        .0
                        .len()
                        .saturating_add(region.location.path.0.len())
                        .saturating_add(1),
                )?;
                if old.argument == region.argument && old.location == region.location {
                    return Err("duplicate type-actual region occurrence".into());
                }
            }
            let ty = actuals
                .get(region.argument)
                .ok_or("type-actual region argument out of bounds")?;
            if !matches!(carrier_at(ty, &region.location, budget)?, Ty::Ref { .. }) {
                return Err("type-actual region is not a reference component".into());
            }
            if let ActualRegion::Named(id) = region.source {
                let env = facts
                    .source_region_env(id.declaration)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?;
                if id.declaration != facts.declaration() || id.index >= env.region_count {
                    return Err("named call lifetime is outside the original caller binder".into());
                }
            }
        }
        for (argument, ty) in actuals.iter().enumerate() {
            for location in carrier_locations(ty, budget)?.iter() {
                if matches!(location.channel, CarrierChannel::Fixed(_)) {
                    budget.charge_work(evidence.type_actual_regions.len())?;
                    let mut found = false;
                    for row in evidence.type_actual_regions.iter() {
                        budget.charge_work(
                            row.location
                                .path
                                .0
                                .len()
                                .saturating_add(location.path.0.len())
                                .saturating_add(1),
                        )?;
                        if row.argument == argument && row.location == *location {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return Err("missing original type-actual region evidence".into());
                    }
                }
            }
        }
    }
    for (index, expression) in source.exprs.iter().enumerate() {
        budget.charge_work(1)?;
        if let ExprKind::Call { target, .. } = &expression.kind {
            // Trait Self has a distinct canonical schema role. Only ordinary
            // method actuals participate in this source-stage evidence gate.
            let actuals: &[Ty] = target.type_arguments();
            budget.charge_work(actuals.len())?;
            if actuals.iter().any(|ty| matches!(ty, Ty::Ref { .. })) && !seen[index] {
                return Err("new reference actual call is missing original-stage evidence".into());
            }
        }
    }
    Ok(())
}

/// Exact generated payload, without an output-source or proof backedge.
#[derive(Clone, Debug)]
struct GeneratedBodyShape {
    name: String,
    params: Arc<[LocalId]>,
    locals: Arc<[LocalDecl]>,
    exprs: Arc<[TypedExpr]>,
    body: Block,
    evidence: Arc<[CallEvidence]>,
    contract: Arc<RefContract>,
}

/// Guard derived metadata equality against expanding independently shared type
/// DAGs. Generated clones retain these canonical immutable type allocations.
pub(super) fn identical_type(a: &Ty, b: &Ty, budget: &PlanBudget) -> Result<bool, String> {
    budget.charge_work(1)?;
    Ok(match (a, b) {
        (
            Ty::Ref {
                kind: ak,
                referent: a,
            },
            Ty::Ref {
                kind: bk,
                referent: b,
            },
        ) => ak == bk && Arc::ptr_eq(a, b),
        (
            Ty::RawPtr {
                kind: ak,
                pointee: a,
            },
            Ty::RawPtr {
                kind: bk,
                pointee: b,
            },
        ) => ak == bk && Arc::ptr_eq(a, b),
        (
            Ty::Array {
                element: a,
                len: an,
            },
            Ty::Array {
                element: b,
                len: bn,
            },
        ) => an == bn && Arc::ptr_eq(a, b),
        (Ty::Closure(a), Ty::Closure(b)) => Arc::ptr_eq(a, b),
        (Ty::AssociatedType(a), Ty::AssociatedType(b)) => Arc::ptr_eq(a, b),
        (Ty::Tuple(a), Ty::Tuple(b)) => Arc::ptr_eq(a, b),
        (Ty::Struct(a), Ty::Struct(b)) => std::ptr::eq(a.def(), b.def()),
        (Ty::Enum(a), Ty::Enum(b)) => std::ptr::eq(a.def(), b.def()),
        (Ty::Dyn(a), Ty::Dyn(b)) => a.principal == b.principal,
        (Ty::Param(a), Ty::Param(b)) => a == b,
        (Ty::Int(a), Ty::Int(b)) => a == b,
        (Ty::Float(a), Ty::Float(b)) => a == b,
        (Ty::Char, Ty::Char)
        | (Ty::Str, Ty::Str)
        | (Ty::Bool, Ty::Bool)
        | (Ty::Unit, Ty::Unit)
        | (Ty::Never, Ty::Never) => true,
        _ => false,
    })
}
pub(super) fn identical_call_types(
    a: &Callee,
    sa: &Signature,
    b: &Callee,
    sb: &Signature,
    budget: &PlanBudget,
) -> Result<bool, String> {
    let count = |target: &Callee| {
        target.type_arguments().len().saturating_add(match target {
            Callee::Direct(_) => 0,
            Callee::TraitItem(call) | Callee::Virtual(VirtualCall { call }) => {
                call.trait_ref.args.len().saturating_add(1)
            }
        })
    };
    budget.charge_work(
        count(a)
            .saturating_add(count(b))
            .saturating_add(sa.params.len())
            .saturating_add(sb.params.len()),
    )?;
    if std::mem::discriminant(a) != std::mem::discriminant(b)
        || a.metadata_types().count() != b.metadata_types().count()
        || sa.params.len() != sb.params.len()
        || !Arc::ptr_eq(&sa.ref_contract, &sb.ref_contract)
    {
        return Ok(false);
    }
    for (a, b) in a
        .metadata_types()
        .zip(b.metadata_types())
        .chain(sa.params.iter().zip(&sb.params))
        .chain(std::iter::once((&sa.result, &sb.result)))
    {
        if !identical_type(a, b, budget)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn identical_block_types(
    a: &Block,
    b: &Block,
    budget: &PlanBudget,
    depth: usize,
) -> Result<bool, String> {
    budget.check_depth(depth)?;
    budget.charge_work(a.statements.len().saturating_add(b.statements.len()))?;
    if a.statements.len() != b.statements.len() {
        return Ok(false);
    }
    for (a, b) in a.statements.iter().zip(&b.statements) {
        match (a, b) {
            (Stmt::LetPattern { ty: a, .. }, Stmt::LetPattern { ty: b, .. })
                if !identical_type(a, b, budget)? =>
            {
                return Ok(false);
            }
            (
                Stmt::LetElse {
                    ty: a, pattern: pa, ..
                },
                Stmt::LetElse {
                    ty: b, pattern: pb, ..
                },
            ) => {
                if !identical_type(a, b, budget)? || !identical_pattern_types(pa, pb, budget)? {
                    return Ok(false);
                }
            }
            (Stmt::While { body: a, .. }, Stmt::While { body: b, .. })
                if !identical_block_types(a, b, budget, depth + 1)? =>
            {
                return Ok(false);
            }
            _ => {}
        }
    }
    Ok(true)
}
fn identical_pattern_types(
    a: &MatchPattern,
    b: &MatchPattern,
    budget: &PlanBudget,
) -> Result<bool, String> {
    budget.charge_work(1)?;
    match (a, b) {
        (
            MatchPattern::Alternatives { ty: a, rows: ar },
            MatchPattern::Alternatives { ty: b, rows: br },
        ) => {
            if !identical_type(a, b, budget)? || ar.len() != br.len() {
                return Ok(false);
            }
            for (a, b) in ar.iter().zip(br) {
                budget.charge_work(1)?;
                if a.len() != b.len() {
                    return Ok(false);
                }
                for (a, b) in a.iter().zip(b) {
                    budget.charge_work(
                        a.projection
                            .len()
                            .checked_add(b.projection.len())
                            .and_then(|n| n.checked_add(1))
                            .ok_or("or-pattern identity work overflow")?,
                    )?;
                    if a != b {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }
        (MatchPattern::EnumVariant { ty: a, .. }, MatchPattern::EnumVariant { ty: b, .. }) => {
            identical_type(&Ty::Enum(a.clone()), &Ty::Enum(b.clone()), budget)
        }
        (MatchPattern::Aggregate { ty: a, .. }, MatchPattern::Aggregate { ty: b, .. }) => {
            identical_type(a, b, budget)
        }
        (
            MatchPattern::Decision {
                ty: a, tests: at, ..
            },
            MatchPattern::Decision {
                ty: b, tests: bt, ..
            },
        ) => {
            budget.charge_work(
                at.len()
                    .checked_add(bt.len())
                    .ok_or("decision identity work overflow")?,
            )?;
            if !identical_type(a, b, budget)? || at.len() != bt.len() {
                return Ok(false);
            }
            for (a, b) in at.iter().zip(bt) {
                match (&a.kind, &b.kind) {
                    (
                        PatternTestKind::EnumVariant { ty: a, .. },
                        PatternTestKind::EnumVariant { ty: b, .. },
                    ) => {
                        if !identical_type(&Ty::Enum(a.clone()), &Ty::Enum(b.clone()), budget)? {
                            return Ok(false);
                        }
                    }
                    (a, b) if std::mem::discriminant(a) != std::mem::discriminant(b) => {
                        return Ok(false);
                    }
                    _ => {}
                }
            }
            Ok(true)
        }
        _ => Ok(std::mem::discriminant(a) == std::mem::discriminant(b)),
    }
}
fn identical_expr_types(a: &ExprKind, b: &ExprKind, budget: &PlanBudget) -> Result<bool, String> {
    budget.charge_work(1)?;
    if std::mem::discriminant(a) != std::mem::discriminant(b) {
        return Ok(false);
    }
    match (a, b) {
        (
            ExprKind::Call {
                target: a,
                signature: sa,
                ..
            },
            ExprKind::Call {
                target: b,
                signature: sb,
                ..
            },
        ) => identical_call_types(a, sa, b, sb, budget),
        (ExprKind::Struct { ty: a, .. }, ExprKind::Struct { ty: b, .. }) => {
            identical_type(&Ty::Struct(a.clone()), &Ty::Struct(b.clone()), budget)
        }
        (ExprKind::EnumVariant { ty: a, .. }, ExprKind::EnumVariant { ty: b, .. }) => {
            identical_type(&Ty::Enum(a.clone()), &Ty::Enum(b.clone()), budget)
        }
        (ExprKind::Match { arms: a, .. }, ExprKind::Match { arms: b, .. }) => {
            budget.charge_work(a.len().saturating_add(b.len()))?;
            if a.len() != b.len() {
                return Ok(false);
            }
            for (a, b) in a.iter().zip(b) {
                if !identical_pattern_types(&a.pattern, &b.pattern, budget)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (
            ExprKind::If {
                then_block: a,
                else_block: aa,
                ..
            },
            ExprKind::If {
                then_block: b,
                else_block: bb,
                ..
            },
        ) => Ok(
            identical_block_types(a, b, budget, 0)? && identical_block_types(aa, bb, budget, 0)?
        ),
        (ExprKind::Block(a), ExprKind::Block(b))
        | (ExprKind::LabeledBlock { body: a, .. }, ExprKind::LabeledBlock { body: b, .. })
        | (ExprKind::Loop { body: a, .. }, ExprKind::Loop { body: b, .. }) => {
            identical_block_types(a, b, budget, 0)
        }
        _ => Ok(true),
    }
}
impl GeneratedBodyShape {
    fn capture(
        source: &CheckedFunction,
        facts: &super::traits::ValidatedFacts,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        source.preflight_plan(FactsCtx::from_validated(facts), budget)?;
        source.preflight_plan(FactsCtx::from_validated(facts), budget)?;
        budget.retain_bytes(std::mem::size_of::<Self>() + 8 * std::mem::size_of::<usize>())?;
        Ok(Self {
            name: source.name.clone(),
            params: source.params.clone().into(),
            locals: source.locals.clone().into(),
            exprs: source.exprs.clone().into(),
            body: source.body.clone(),
            evidence: source.call_evidence.clone(),
            contract: source.ref_contract.clone(),
        })
    }
    fn validate(
        &self,
        source: &CheckedFunction,
        facts: &super::traits::ValidatedFacts,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        source.preflight_plan(FactsCtx::from_validated(facts), budget)?;
        budget.charge_work(
            self.name
                .len()
                .saturating_add(self.params.len())
                .saturating_add(self.locals.len())
                .saturating_add(self.exprs.len()),
        )?;
        if source.name != self.name
            || source.params.as_slice() != self.params.as_ref()
            || source.locals.len() != self.locals.len()
            || source.exprs.len() != self.exprs.len()
            || !Arc::ptr_eq(&source.call_evidence, &self.evidence)
            || !Arc::ptr_eq(&source.ref_contract, &self.contract)
        {
            return Err("generated body metadata subject changed".into());
        }
        for (a, b) in source.locals.iter().zip(self.locals.iter()) {
            if !identical_type(&a.ty, &b.ty, budget)? {
                return Err("generated local type allocation changed".into());
            }
        }
        for (a, b) in source.exprs.iter().zip(self.exprs.iter()) {
            if !identical_type(&a.ty, &b.ty, budget)?
                || !identical_expr_types(&a.kind, &b.kind, budget)?
            {
                return Err("generated expression metadata changed".into());
            }
        }
        if !identical_block_types(&source.body, &self.body, budget, 0)?
            || source.locals.as_slice() != self.locals.as_ref()
            || source.exprs.as_slice() != self.exprs.as_ref()
            || source.body != self.body
        {
            return Err("generated operation, call or control-flow metadata changed".into());
        }
        Ok(())
    }
}

/// Provenance of the one exhaustive instantiator, never an interner cache grant.
#[derive(Clone, Debug)]
pub struct BodySubstitutionProof {
    template: super::ownership::VerifiedTemplateCfg,
    target: CallTarget,
    nominals: Arc<[NominalCarrierMapping]>,
    local_types: Arc<[Ty]>,
    expression_types: Arc<[Ty]>,
    result: Ty,
    generated: GeneratedBodyShape,
}
impl BodySubstitutionProof {
    pub fn template(&self) -> &super::ownership::VerifiedTemplateCfg {
        &self.template
    }
    pub fn target(&self) -> &CallTarget {
        &self.target
    }
    pub fn nominals(&self) -> &[NominalCarrierMapping] {
        &self.nominals
    }
    pub fn validate(
        &self,
        source: &CheckedFunction,
        facts: &super::traits::ValidatedFacts,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if facts.phase() != super::traits::FactsPhase::Closed
            || !self.template.budget().same_authority(budget)
            || !facts.budget().same_authority(budget)
            || !Arc::ptr_eq(facts.world(), self.template.facts().world())
            || self.target.declaration != self.template.facts().declaration()
        {
            return Err("body substitution proof authority mismatch".into());
        }
        let target = facts
            .target()
            .ok_or("body substitution requires closed target")?;
        if target.declaration != self.target.declaration
            || !facts
                .world()
                .types_equal(&target.type_args, &self.target.type_args)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
            || source.exprs.len() != self.template.source().exprs.len()
            || source.locals.len() != self.template.source().locals.len()
            || source.locals.len() != self.local_types.len()
            || source.exprs.len() != self.expression_types.len()
        {
            return Err("body substitution proof subject mismatch".into());
        }
        for mapping in self.nominals.iter() {
            budget.charge_work(1)?;
            if mapping.original.snapshot != mapping.instantiated.snapshot {
                return Err("body nominal mapping changed registry snapshot".into());
            }
        }
        for (local, expected) in source.locals.iter().zip(self.local_types.iter()) {
            if !facts
                .world()
                .types_equal(
                    std::slice::from_ref(&local.ty),
                    std::slice::from_ref(expected),
                )
                .map_err(|e| e.into_legacy_diagnostic(budget))?
            {
                return Err("body substitution changed generated local type".into());
            }
        }
        if !facts
            .world()
            .types_equal(
                std::slice::from_ref(&source.result),
                std::slice::from_ref(&self.result),
            )
            .map_err(|e| e.into_legacy_diagnostic(budget))?
        {
            return Err("body substitution changed generated result type".into());
        }
        for (expression, expected) in source.exprs.iter().zip(self.expression_types.iter()) {
            if !facts
                .world()
                .types_equal(
                    std::slice::from_ref(&expression.ty),
                    std::slice::from_ref(expected),
                )
                .map_err(|e| e.into_legacy_diagnostic(budget))?
            {
                return Err("body substitution changed generated expression type".into());
            }
        }
        self.generated.validate(source, facts, budget)?;
        Ok(())
    }
    pub fn validate_cfg(
        &self,
        function: &super::cfg::Function,
        facts: &super::traits::ValidatedFacts,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if facts.phase() != super::traits::FactsPhase::Closed
            || !self.template.budget().same_authority(budget)
            || !facts.budget().same_authority(budget)
            || !Arc::ptr_eq(facts.world(), self.template.facts().world())
        {
            return Err("CFG substitution proof authority mismatch".into());
        }
        let target = facts
            .target()
            .ok_or("CFG substitution requires closed target")?;
        if target.declaration != self.target.declaration
            || !facts
                .world()
                .types_equal(&target.type_args, &self.target.type_args)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
        {
            return Err("CFG substitution target mismatch".into());
        }
        function
            .lowering_proof
            .as_ref()
            .ok_or("CFG body proof was not bound by canonical lowering")?
            .validate(function, facts, budget)?;
        let count = self
            .local_types
            .len()
            .checked_add(self.expression_types.len())
            .ok_or("CFG substitution local overflow")?;
        if function.locals.len() < count || function.params != self.template.source().params {
            return Err("CFG substitution locals or parameters changed".into());
        }
        for (local, expected) in function
            .locals
            .iter()
            .take(count)
            .zip(self.local_types.iter().chain(self.expression_types.iter()))
        {
            if !facts
                .world()
                .types_equal(
                    std::slice::from_ref(&local.ty),
                    std::slice::from_ref(expected),
                )
                .map_err(|e| e.into_legacy_diagnostic(budget))?
            {
                return Err("CFG substitution generated local type mismatch".into());
            }
        }
        Ok(())
    }
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
/// The rejected ledger operation, not a classification inferred from text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceBudgetKind {
    Work,
    Nodes,
    Applications,
    Obligations,
    DiagnosticBytes,
    Bytes,
    Depth,
    OwnershipWork,
    OwnershipStateBytes,
    OwnershipReservationStorage,
}
impl ResourceBudgetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Nodes => "nodes",
            Self::Applications => "applications",
            Self::Obligations => "obligations",
            Self::DiagnosticBytes => "diagnostic_bytes",
            Self::Bytes => "bytes",
            Self::Depth => "depth",
            Self::OwnershipWork => "ownership_work",
            Self::OwnershipStateBytes => "ownership_state_bytes",
            Self::OwnershipReservationStorage => "ownership_reservation_storage",
        }
    }
}

/// Allocation-free evidence captured at the rejection, before diagnostic text.
/// `requested` is the increment and `used` the pre-operation usage. Depth checks
/// instead use the observed depth as `requested` and zero as `used`.
/// Reservation-sizing overflow reports the two rejected addition operands.
/// This records instrumented ledger operations, not arbitrary semantic guards,
/// allocator failures, or process OOM. An absent event does not prove success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceBudgetFailure {
    pub kind: ResourceBudgetKind,
    pub requested: usize,
    pub used: usize,
    pub limit: usize,
    pub arithmetic_overflow: bool,
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
    pub graph_id: Option<u64>,
    pub task_id: Option<u64>,
}
impl ResourceBudgetFailure {
    #[track_caller]
    pub(super) fn new(
        kind: ResourceBudgetKind,
        requested: usize,
        used: usize,
        limit: usize,
    ) -> Self {
        let caller = std::panic::Location::caller();
        let identity = crate::engine::current_task_identity();
        Self {
            kind,
            requested,
            used,
            limit,
            arithmetic_overflow: used.checked_add(requested).is_none(),
            file: caller.file(),
            line: caller.line(),
            column: caller.column(),
            graph_id: identity.map(|(graph, _)| graph),
            task_id: identity.map(|(_, task)| task),
        }
    }

    /// Bounded JSON with no source text or backtrace. File names are capped at
    /// 256 Unicode scalars, escaped, and explicitly marked when truncated.
    /// The writer is caller-owned so recording a failure never allocates.
    pub fn write_json(&self, out: &mut impl std::fmt::Write) -> std::fmt::Result {
        out.write_str("{\"graph_id\":")?;
        match self.graph_id {
            Some(id) => write!(out, "{id}")?,
            None => out.write_str("null")?,
        }
        out.write_str(",\"task_id\":")?;
        match self.task_id {
            Some(id) => write!(out, "{id}")?,
            None => out.write_str("null")?,
        }
        out.write_char(',')?;
        write!(
            out,
            "\"kind\":\"{}\",\"requested\":{},\"used\":{},\"limit\":{},\"arithmetic_overflow\":{},\"call_site\":{{\"file\":\"",
            self.kind.as_str(),
            self.requested,
            self.used,
            self.limit,
            self.arithmetic_overflow
        )?;
        for character in self.file.chars().take(256) {
            match character {
                '"' => out.write_str("\\\"")?,
                '\\' => out.write_str("\\\\")?,
                '\u{0}'..='\u{1f}' => write!(out, "\\u{:04x}", character as u32)?,
                other => out.write_char(other)?,
            }
        }
        write!(
            out,
            "\",\"file_truncated\":{},\"line\":{},\"column\":{}}}}}",
            self.file.chars().nth(256).is_some(),
            self.line,
            self.column
        )
    }
    pub fn to_json(&self) -> String {
        let mut output = String::new();
        self.write_json(&mut output).expect("writing to a String");
        output
    }
}

pub use super::resource_accounting::ReservationAccounting;

/// Shared-plan counters sampled atomically under the existing budget lock.
/// `bytes` equals `reservations.current_bytes` for an unpoisoned ledger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanUsageSnapshot {
    pub applications: usize,
    pub nodes: usize,
    pub work: usize,
    pub bytes: usize,
    pub obligations: usize,
    pub diagnostic_bytes: usize,
    pub reservations: ReservationAccounting,
    pub first_failure: Option<ResourceBudgetFailure>,
    pub poisoned: bool,
}
impl PlanUsageSnapshot {
    pub fn write_json(&self, out: &mut impl std::fmt::Write) -> std::fmt::Result {
        write!(
            out,
            "{{\"applications\":{},\"nodes\":{},\"work\":{},\"bytes\":{},\"obligations\":{},\"diagnostic_bytes\":{},\"poisoned\":{},\"reservations\":",
            self.applications,
            self.nodes,
            self.work,
            self.bytes,
            self.obligations,
            self.diagnostic_bytes,
            self.poisoned
        )?;
        self.reservations.write_json(out)?;
        out.write_str(",\"first_failure\":")?;
        match self.first_failure {
            Some(failure) => failure.write_json(out)?,
            None => out.write_str("null")?,
        }
        out.write_char('}')
    }
    pub fn to_json(&self) -> String {
        let mut output = String::new();
        self.write_json(&mut output).expect("writing to a String");
        output
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
    resource_failed: bool,
    reservations: ReservationAccounting,
    first_failure: Option<ResourceBudgetFailure>,
}
/// Immutable identity and policy, with no resource ledger or spending API.
#[derive(Clone, Debug)]
pub struct PlanAuthority {
    identity: Arc<PlanAuthorityIdentity>,
}
#[derive(Debug)]
struct PlanAuthorityIdentity {
    limits: PlanLimits,
}
impl PlanAuthority {
    pub fn limits(&self) -> PlanLimits {
        self.identity.limits
    }
    pub fn same_authority(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}
/// Evidence of paid storage, without a ledger or meter back-reference.
/// Receipt drop/clone never refunds/charges bytes. The originating ledger or
/// task conservatively retains admission charges until its own lifetime ends.
#[derive(Clone, Debug)]
pub struct RetainedStorage {
    authority: PlanAuthority,
    bytes: usize,
}
impl RetainedStorage {
    pub fn authority(&self) -> &PlanAuthority {
        &self.authority
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}
#[derive(Clone, Debug)]
pub struct PlanBudget {
    limits: PlanLimits,
    authority: PlanAuthority,
    usage: Arc<std::sync::Mutex<PlanUsage>>,
}
#[derive(Debug)]
pub struct PlanReservation {
    authority: PlanAuthority,
    retained: bool,
    usage: Arc<std::sync::Mutex<PlanUsage>>,
    bytes: usize,
    /// Set when the bytes were charged to a task meter instead of the ledger.
    meter: Option<crate::engine::MeterHandle>,
}
impl PlanReservation {
    /// Release part of this reservation in place: decrease-only, never
    /// admits, never raises any charge or limit. `bytes` must not exceed the
    /// current reservation.
    pub fn shrink(&mut self, bytes: usize) -> Result<(), String> {
        if bytes > self.bytes {
            return Err("reservation shrink larger than the reservation".into());
        }
        if bytes == 0 {
            return Ok(());
        }
        if let Some(meter) = &self.meter {
            meter.meter().bytes.fetch_sub(
                u64::try_from(bytes).unwrap_or(u64::MAX),
                std::sync::atomic::Ordering::Relaxed,
            );
        } else {
            let mut usage = self
                .usage
                .lock()
                .map_err(|_| "generic planning budget poisoned".to_string())?;
            usage.bytes -= bytes;
            usage
                .reservations
                .resize_bytes(self.bytes, self.bytes - bytes);
        }
        self.bytes -= bytes;
        Ok(())
    }
    /// Consume a paid lease without releasing or admitting its bytes again.
    pub fn retain(mut self) -> Result<RetainedStorage, String> {
        if self.meter.is_none() {
            let mut usage = self
                .usage
                .lock()
                .map_err(|_| "generic planning budget poisoned".to_string())?;
            usage.reservations.record_retention();
            usage.reservations.current_reservations =
                usage.reservations.current_reservations.saturating_sub(1);
        }
        self.retained = true;
        Ok(RetainedStorage {
            authority: self.authority.clone(),
            bytes: self.bytes,
        })
    }
}
impl Drop for PlanReservation {
    fn drop(&mut self) {
        if self.retained {
            return;
        }
        if let Some(meter) = &self.meter {
            meter.meter().bytes.fetch_sub(
                u64::try_from(self.bytes).unwrap_or(u64::MAX),
                std::sync::atomic::Ordering::Relaxed,
            );
            return;
        }
        if let Ok(mut usage) = self.usage.lock() {
            usage.bytes -= self.bytes;
            usage.reservations.release(self.bytes);
        }
    }
}
#[cfg(test)]
impl PlanBudget {
    pub(crate) fn poison_ledger_for_tests(&self) {
        let usage = self.usage.clone();
        let _ = std::thread::spawn(move || {
            let _guard = usage.lock().unwrap();
            panic!("intentional ledger poison (test)");
        })
        .join();
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
            authority: PlanAuthority {
                identity: Arc::new(PlanAuthorityIdentity { limits }),
            },
            usage: Arc::new(std::sync::Mutex::new(PlanUsage::default())),
        }
    }
    pub fn limits(&self) -> PlanLimits {
        self.limits
    }
    pub fn authority(&self) -> &PlanAuthority {
        &self.authority
    }
    pub fn same_authority(&self, other: &Self) -> bool {
        self.authority.same_authority(&other.authority)
    }
    /// A fixed-size snapshot. A poisoned ledger is readable for reporting only,
    /// while all existing budget operations retain their poison error behavior.
    pub fn usage_snapshot(&self) -> PlanUsageSnapshot {
        let (usage, poisoned) = match self.usage.lock() {
            Ok(usage) => (usage, false),
            Err(error) => (error.into_inner(), true),
        };
        PlanUsageSnapshot {
            applications: usage.applications,
            nodes: usage.nodes,
            work: usage.work,
            bytes: usage.bytes,
            obligations: usage.obligations,
            diagnostic_bytes: usage.diagnostic_bytes,
            reservations: usage.reservations,
            first_failure: usage.first_failure,
            poisoned,
        }
    }
    pub fn first_resource_failure(&self) -> Option<ResourceBudgetFailure> {
        self.usage_snapshot().first_failure
    }
    /// Attach an exact rejection from a subordinate budget to the same ledger.
    /// It cannot replace a failure already observed by another worker.
    pub(super) fn record_resource_failure(&self, failure: ResourceBudgetFailure) {
        if let Ok(mut usage) = self.usage.lock() {
            usage.resource_failed = true;
            usage.first_failure.get_or_insert(failure);
        }
    }
    /// Resource classification evidence from this exact shared planning ledger.
    /// Reservations may release storage, but never clear a failed operation.
    pub(super) fn has_resource_failure(&self) -> bool {
        self.usage
            .lock()
            .map_or(true, |usage| usage.resource_failed)
    }
    #[track_caller]
    fn charge(
        &self,
        amount: usize,
        select: fn(&mut PlanUsage) -> &mut usize,
        limit: usize,
        kind: ResourceBudgetKind,
        reservation: Option<bool>,
    ) -> Result<(), String> {
        let mut usage = self
            .usage
            .lock()
            .map_err(|_| "generic planning budget poisoned")?;
        let used = *select(&mut usage);
        let next = match used.checked_add(amount) {
            Some(next) if next <= limit => next,
            _ => {
                usage.resource_failed = true;
                let failure = ResourceBudgetFailure::new(kind, amount, used, limit);
                usage.first_failure.get_or_insert(failure);
                return Err("generic planning resource limit exceeded".into());
            }
        };
        *select(&mut usage) = next;
        if let Some(retained) = reservation {
            usage.reservations.record(amount, retained);
        }
        Ok(())
    }
    #[track_caller]
    pub fn charge_work(&self, amount: usize) -> Result<(), String> {
        if let Some(result) = self.charge_task(amount, ResourceBudgetKind::Work) {
            return result;
        }
        self.charge(
            amount,
            |u| &mut u.work,
            self.limits.work,
            ResourceBudgetKind::Work,
            None,
        )
    }
    /// Per-task metering: inside a graph task, work and node charges go to
    /// the polled node's own meter with per-task limits, so total program size
    /// has no shared ceiling and workers never contend on one counter.
    /// Returns `None` outside graph polling (direct API and unit-test use),
    /// where the shared ledger still applies. Poison and recorded failures
    /// remain sticky through the ledger.
    #[track_caller]
    fn charge_task(&self, amount: usize, kind: ResourceBudgetKind) -> Option<Result<(), String>> {
        let caller = std::panic::Location::caller();
        let limit = match kind {
            ResourceBudgetKind::Work => self.limits.work,
            _ => self.limits.nodes,
        };
        crate::engine::with_current_meter(|meter| {
            let meter = meter?;
            let counter = match kind {
                ResourceBudgetKind::Work => &meter.work,
                _ => &meter.nodes,
            };
            let amount64 = u64::try_from(amount).unwrap_or(u64::MAX);
            let limit64 = u64::try_from(limit).unwrap_or(u64::MAX);
            let limit64 = match kind {
                ResourceBudgetKind::Work | ResourceBudgetKind::OwnershipWork => {
                    meter.work_limit(limit64)
                }
                _ => limit64,
            };
            let limit = usize::try_from(limit64).unwrap_or(usize::MAX);
            Some(
                match crate::engine::Meter::admit(counter, amount64, limit64) {
                    Ok(()) => Ok(()),
                    Err(used) => {
                        let used = usize::try_from(used).unwrap_or(usize::MAX);
                        match self.usage.lock() {
                            Ok(mut usage) => {
                                usage.resource_failed = true;
                                let mut failure =
                                    ResourceBudgetFailure::new(kind, amount, used, limit);
                                failure.file = caller.file();
                                failure.line = caller.line();
                                failure.column = caller.column();
                                usage.first_failure.get_or_insert(failure);
                                Err("generic planning resource limit exceeded".into())
                            }
                            Err(_) => Err("generic planning budget poisoned".into()),
                        }
                    }
                },
            )
        })
    }
    #[track_caller]
    pub fn charge_nodes(&self, amount: usize) -> Result<(), String> {
        if let Some(result) = self.charge_task(amount, ResourceBudgetKind::Nodes) {
            return result;
        }
        self.charge(
            amount,
            |u| &mut u.nodes,
            self.limits.nodes,
            ResourceBudgetKind::Nodes,
            None,
        )
    }
    #[track_caller]
    pub fn charge_application(&self) -> Result<(), String> {
        self.charge(
            1,
            |u| &mut u.applications,
            self.limits.applications,
            ResourceBudgetKind::Applications,
            None,
        )
    }
    #[track_caller]
    pub fn charge_obligation(&self) -> Result<(), String> {
        self.charge(
            1,
            |u| &mut u.obligations,
            4096,
            ResourceBudgetKind::Obligations,
            None,
        )
    }
    #[track_caller]
    pub fn charge_diagnostic_bytes(&self, bytes: usize) -> Result<(), String> {
        self.charge(
            bytes,
            |u| &mut u.diagnostic_bytes,
            64 * 1024,
            ResourceBudgetKind::DiagnosticBytes,
            None,
        )
    }
    #[track_caller]
    pub fn reserve_bytes(&self, bytes: usize) -> Result<PlanReservation, String> {
        if let Some(meter) = crate::engine::MeterHandle::current() {
            self.charge_meter_bytes(meter.meter(), bytes)?;
            return Ok(PlanReservation {
                authority: self.authority.clone(),
                retained: false,
                usage: self.usage.clone(),
                bytes,
                meter: Some(meter),
            });
        }
        self.charge(
            bytes,
            |u| &mut u.bytes,
            self.limits.bytes,
            ResourceBudgetKind::Bytes,
            Some(false),
        )?;
        Ok(PlanReservation {
            authority: self.authority.clone(),
            retained: false,
            usage: self.usage.clone(),
            bytes,
            meter: None,
        })
    }
    /// Charge live bytes to one task's meter under the per-task byte limit.
    #[track_caller]
    fn charge_meter_bytes(&self, meter: &crate::engine::Meter, bytes: usize) -> Result<(), String> {
        let limit64 = meter.bytes_limit(u64::try_from(self.limits.bytes).unwrap_or(u64::MAX));
        let limit = usize::try_from(limit64).unwrap_or(usize::MAX);
        match crate::engine::Meter::admit(
            &meter.bytes,
            u64::try_from(bytes).unwrap_or(u64::MAX),
            limit64,
        ) {
            Ok(()) => {
                if self.usage.is_poisoned() {
                    return Err("generic planning budget poisoned".into());
                }
                Ok(())
            }
            Err(used) => match self.usage.lock() {
                Ok(mut usage) => {
                    usage.resource_failed = true;
                    let failure = ResourceBudgetFailure::new(
                        ResourceBudgetKind::Bytes,
                        bytes,
                        usize::try_from(used).unwrap_or(usize::MAX),
                        limit,
                    );
                    usage.first_failure.get_or_insert(failure);
                    Err("generic planning resource limit exceeded".into())
                }
                Err(_) => Err("generic planning budget poisoned".into()),
            },
        }
    }
    /// Conservatively retain generated semantic storage until this plan ends.
    #[track_caller]
    pub fn retain_bytes(&self, bytes: usize) -> Result<(), String> {
        if let Some(result) = crate::engine::with_current_meter(|meter| {
            meter.map(|meter| self.charge_meter_bytes(meter, bytes))
        }) {
            return result;
        }
        self.charge(
            bytes,
            |u| &mut u.bytes,
            self.limits.bytes,
            ResourceBudgetKind::Bytes,
            Some(true),
        )
    }
    #[track_caller]
    pub fn check_depth(&self, depth: usize) -> Result<(), String> {
        if depth > self.limits.depth {
            if let Ok(mut usage) = self.usage.lock() {
                usage.resource_failed = true;
                let failure = ResourceBudgetFailure::new(
                    ResourceBudgetKind::Depth,
                    depth,
                    0,
                    self.limits.depth,
                );
                usage.first_failure.get_or_insert(failure);
            }
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
    /// A structural request minted by the common typed substitution walk.
    /// The returned type is unsealed until its body/subject proof is checked.
    fn intern_substituted_nominal(
        &mut self,
        request: &NominalSubstitutionRequest<'_>,
        caller: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Ty, String> {
        let info = self.nominal_application(request.original)?;
        self.intern_nominal(info.source_decl, request.arguments, caller, budget)
    }
}

pub struct NominalSubstitutionRequest<'a> {
    original: NominalAppId,
    arguments: &'a [Ty],
}
impl NominalSubstitutionRequest<'_> {
    pub fn original(&self) -> NominalAppId {
        self.original
    }
    pub fn arguments(&self) -> &[Ty] {
        self.arguments
    }
}

/// Shape admission only, never an executable family capability. Source and
/// Recipe actuals use the same root-chain rule. Nominal/aggregate reference
/// actuals are not admitted by this function.
pub fn validate_root_reference_actual(ty: &Ty, budget: &PlanBudget) -> Result<(), String> {
    let mut inner = ty;
    let mut depth = 0;
    while let Ty::Ref { referent, .. } = inner {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        depth += 1;
        inner = referent;
    }
    budget.check_depth(depth)?;
    reject_references(inner, budget)
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
        validate_root_reference_actual(argument, budget)?;
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
            root_reference_layers(argument)?;
            let mut inner = argument;
            while let Ty::Ref { referent, .. } = inner {
                budget.charge_work(1)?;
                inner = referent;
            }
            // Unsized `str` / `[T]` are valid trait Self types (`impl Tr for str`).
            match inner {
                Ty::Str => {}
                Ty::Slice { element } => reject_references(element, budget)?,
                _ => reject_references(inner, budget)?,
            }
        } else if declaration.kind == DeclKind::TraitItem
            && declaration.own_env.binder() == parameter.owner
            && declaration.own_env.contains(parameter)
        {
            validate_root_reference_actual(argument, budget)?;
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
    if budget.has_resource_failure() {
        return Err("planning resource budget already exhausted".into());
    }
    budget.check_depth(depth)?;
    budget.charge_work(1)?;
    budget.charge_nodes(1)?;
    Ok(match ty {
        Ty::Closure(closure) => {
            budget.retain_bytes(
                std::mem::size_of::<ClosureTy>()
                    + closure.types().count() * std::mem::size_of::<Ty>(),
            )?;
            Ty::Closure(Arc::new(closure.try_map(|ty| {
                substitute_ty_walk(ty, env, args, caller, cx, budget, depth + 1)
            })?))
        }
        Ty::AssociatedType(projection) => {
            if projection.trait_ref.args.len() > 32 {
                return Err("trait argument limit exceeded".into());
            }
            budget.charge_work(projection.trait_ref.args.len())?;
            budget.retain_bytes(
                std::mem::size_of::<AssociatedTypeProjection>() + 2 * std::mem::size_of::<usize>(),
            )?;
            let bytes = projection
                .trait_ref
                .args
                .len()
                .checked_mul(std::mem::size_of::<Ty>())
                .ok_or("substitution size overflow")?;
            budget.retain_bytes(bytes)?;
            let mut arguments = Vec::new();
            arguments
                .try_reserve_exact(projection.trait_ref.args.len())
                .map_err(|_| "projection substitution allocation resource failure")?;
            budget.retain_bytes(
                arguments
                    .capacity()
                    .checked_sub(projection.trait_ref.args.len())
                    .and_then(|n| n.checked_mul(std::mem::size_of::<Ty>()))
                    .ok_or("substitution capacity overflow")?,
            )?;
            let self_ty = substitute_ty_walk(
                &projection.trait_ref.self_ty,
                env,
                args,
                caller,
                cx,
                budget,
                depth + 1,
            )?;
            for argument in projection.trait_ref.args.iter() {
                arguments.push(substitute_ty_walk(
                    argument,
                    env,
                    args,
                    caller,
                    cx,
                    budget,
                    depth + 1,
                )?);
            }
            Ty::AssociatedType(Arc::new(AssociatedTypeProjection {
                trait_ref: TraitRef {
                    declaration: projection.trait_ref.declaration,
                    self_ty,
                    args: arguments.into(),
                },
                item: projection.item,
            }))
        }
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
        Ty::Slice { element } => {
            budget.retain_bytes(std::mem::size_of::<Ty>() + std::mem::size_of::<usize>())?;
            Ty::Slice {
                element: Arc::new(substitute_ty_walk(
                    element,
                    env,
                    args,
                    caller,
                    cx,
                    budget,
                    depth + 1,
                )?),
            }
        }
        Ty::RawPtr { kind, pointee } => {
            budget.retain_bytes(std::mem::size_of::<Ty>() + 2 * std::mem::size_of::<usize>())?;
            Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(substitute_ty_walk(
                    pointee,
                    env,
                    args,
                    caller,
                    cx,
                    budget,
                    depth + 1,
                )?),
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
            cx.intern_substituted_nominal(
                &NominalSubstitutionRequest {
                    original: value.def().id,
                    arguments: &arguments,
                },
                caller,
                budget,
            )?
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
            cx.intern_substituted_nominal(
                &NominalSubstitutionRequest {
                    original: value.def().id,
                    arguments: &arguments,
                },
                caller,
                budget,
            )?
        }
        Ty::Int(_)
        | Ty::Float(_)
        | Ty::Char
        | Ty::Str
        | Ty::Bool
        | Ty::Unit
        | Ty::Dyn(_)
        | Ty::Never => ty.clone(),
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
        validate_root_reference_actual(ty, budget)?;
        if matches!(ty, Ty::Ref { .. }) && template.carrier_effects().is_none() {
            return Err(
                "unsupported: reference actual requires exact universal carrier proof".into(),
            );
        }
    }
    let source = template.source();
    source.preflight_plan(FactsCtx::from_validated(template.facts()), budget)?;
    let mut output = source.as_ref().clone();
    let mut recording = RecordingSubstitutionCx {
        inner: cx,
        nominals: Vec::new(),
    };
    let mut visitor = Instantiate {
        facts: FactsCtx::from_validated(closed_facts),
        env,
        args,
        cx: &mut recording,
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
                    visitor.pattern(&mut arm.pattern)?;
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
                signature.ref_contract = closed_facts
                    .carrier_contract(SignatureSubject::Call(target))
                    .map_err(|error| error.into_legacy_diagnostic(budget))?
                    .ref_contract()
                    .clone();
            }
            ExprKind::If {
                then_block,
                else_block,
                ..
            } => {
                visitor.block(then_block, 0)?;
                visitor.block(else_block, 0)?;
            }
            ExprKind::Block(block)
            | ExprKind::Loop { body: block, .. }
            | ExprKind::LabeledBlock { body: block, .. } => visitor.block(block, 0)?,
            ExprKind::CharToInt { .. }
            | ExprKind::U8ToChar { .. }
            | ExprKind::PtrCast { .. }
            | ExprKind::Intrinsic { .. }
            | ExprKind::LeadingZeros { .. }
            | ExprKind::IntCast { .. }
            | ExprKind::FloatCast { .. }
            | ExprKind::Char(_)
            | ExprKind::Str(_)
            | ExprKind::Float(_)
            | ExprKind::Deref { .. }
            | ExprKind::DynamicIndex { .. }
            | ExprKind::ArrayToSlice { .. }
            | ExprKind::RefToDyn { .. }
            | ExprKind::SliceLen { .. }
            | ExprKind::SliceIndex { .. }
            | ExprKind::Subslice { .. }
            | ExprKind::Array(_)
            | ExprKind::Repeat { .. }
            | ExprKind::Index { .. }
            | ExprKind::Int(_)
            | ExprKind::Bool(_)
            | ExprKind::Local(_)
            | ExprKind::NativeSyscall(_)
            | ExprKind::Tuple(_)
            | ExprKind::Field { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. }
            | ExprKind::Break { .. }
            | ExprKind::Continue { .. }
            | ExprKind::Output(_)
            | ExprKind::Format { .. }
            | ExprKind::Println(_)
            | ExprKind::Panic(_) => {}
        }
    }
    visitor.block(&mut output.body, 0)?;
    output.ref_contract = closed_facts
        .carrier_contract(SignatureSubject::Body)
        .map_err(|e| e.into_legacy_diagnostic(budget))?
        .ref_contract()
        .clone();
    output = canonicalize_call_metadata(&output, closed_facts, recording.inner, budget)
        .map_err(|e| e.into_legacy_diagnostic(budget))?;
    let generated = GeneratedBodyShape::capture(&output, closed_facts, budget)?;
    budget.retain_bytes(
        output
            .locals
            .len()
            .checked_add(output.exprs.len())
            .and_then(|n| n.checked_mul(2 * std::mem::size_of::<Ty>()))
            .ok_or("body type provenance storage overflow")?,
    )?;
    budget.retain_bytes(
        std::mem::size_of::<BodySubstitutionProof>() + 4 * std::mem::size_of::<usize>(),
    )?;
    output.body_substitution = Some(Arc::new(BodySubstitutionProof {
        template: template.clone(),
        target: target.clone(),
        nominals: recording.nominals.into(),
        local_types: output
            .locals
            .iter()
            .map(|local| local.ty.clone())
            .collect::<Vec<_>>()
            .into(),
        expression_types: output
            .exprs
            .iter()
            .map(|expression| expression.ty.clone())
            .collect::<Vec<_>>()
            .into(),
        result: output.result.clone(),
        generated,
    }));
    output.validate_with_facts(FactsCtx::from_validated(closed_facts))?;
    Ok(output)
}

struct RecordingSubstitutionCx<'a, C> {
    inner: &'a mut C,
    nominals: Vec<NominalCarrierMapping>,
}
impl<C: SubstitutionCx> SubstitutionCx for RecordingSubstitutionCx<'_, C> {
    fn nominal_application(&self, id: NominalAppId) -> Result<NominalAppInfo, String> {
        self.inner.nominal_application(id)
    }
    fn intern_nominal(
        &mut self,
        declaration: DeclId,
        args: &[Ty],
        caller: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Ty, String> {
        self.inner.intern_nominal(declaration, args, caller, budget)
    }
    fn instantiate_signature(
        &mut self,
        target: &CallTarget,
        caller: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Signature, String> {
        self.inner.instantiate_signature(target, caller, budget)
    }
    fn intern_substituted_nominal(
        &mut self,
        request: &NominalSubstitutionRequest<'_>,
        caller: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<Ty, String> {
        let before = self.inner.nominal_application(request.original)?;
        let output = self
            .inner
            .intern_substituted_nominal(request, caller, budget)?;
        let instantiated = match &output {
            Ty::Struct(value) => value.def().id,
            Ty::Enum(value) => value.def().id,
            _ => return Err("nominal substitution changed kind".into()),
        };
        let after = self.inner.nominal_application(instantiated)?;
        if before.source_decl != after.source_decl {
            return Err("nominal substitution changed source declaration".into());
        }
        budget.charge_work(self.nominals.len().saturating_add(1))?;
        if let Some(old) = self
            .nominals
            .iter()
            .find(|mapping| mapping.original == request.original)
        {
            if old.instantiated != instantiated {
                return Err("conflicting nominal substitution".into());
            }
        } else {
            budget.retain_bytes(3 * std::mem::size_of::<NominalCarrierMapping>())?;
            self.nominals
                .try_reserve(1)
                .map_err(|_| "nominal mapping allocation resource limit")?;
            self.nominals.push(NominalCarrierMapping {
                original: request.original,
                instantiated,
            });
        }
        Ok(output)
    }
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
        // Trait object metadata is closed: `dyn P` contains no parameters.
        Callee::Virtual(virtual_call) => {
            if !virtual_call.call.method_args.is_empty()
                || !virtual_call.call.trait_ref.args.is_empty()
            {
                return Err("unsupported: generic virtual calls".into());
            }
            virtual_call.principal()?;
            Callee::Virtual(virtual_call.clone())
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
        Callee::Virtual(virtual_call) => {
            closed_facts
                .service()
                .instantiate_trait_signature(&virtual_call.call, closed_facts)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
            Ok(Callee::Virtual(virtual_call))
        }
    }
}

/// Private unsealed source transform. The consumed source capability and final
/// exact-Arc replay validator, not this intermediate value, establish authority.
pub(super) fn canonicalize_source_calls(
    source: &CheckedFunction,
    facts: &super::traits::ValidatedFacts,
    cx: &mut impl SubstitutionCx,
    _cap: &super::frontend::CanonicalSourceCapability,
    budget: &PlanBudget,
) -> Result<CheckedFunction, super::traits::FactsError> {
    if facts.phase() == super::traits::FactsPhase::Schema
        || !facts.budget().same_authority(budget)
        || source.body_substitution.is_some()
    {
        return Err(super::traits::FactsError::invalid(
            budget,
            "canonical source transform requires original executable facts",
        ));
    }
    canonicalize_call_metadata(source, facts, cx, budget)
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
    let output = canonicalize_call_metadata(source, facts, _cx, budget)?;
    output
        .validate_with_facts(FactsCtx::from_validated(facts))
        .map_err(|e| FactsError::from_message(budget, &e))?;
    Ok(output)
}

// Unsealed intermediate used only by the exhaustive verified-template walker.
// The caller must mint its exact payload proof and pass the public validator.
fn canonicalize_call_metadata(
    source: &CheckedFunction,
    facts: &super::traits::ValidatedFacts,
    _cx: &mut impl SubstitutionCx,
    budget: &PlanBudget,
) -> Result<CheckedFunction, super::traits::FactsError> {
    use super::traits::FactsError;
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
                // Dynamic dispatch stays virtual in every phase. Its canonical
                // signature is the trait item's at the object Self type.
                Callee::Virtual(virtual_call) => {
                    let signature = facts
                        .service()
                        .instantiate_trait_signature(&virtual_call.call, facts)?;
                    (Callee::Virtual(virtual_call.clone()), signature)
                }
            };
            if facts.phase() == super::traits::FactsPhase::Closed
                && !matches!(callee, Callee::Direct(_) | Callee::Virtual(_))
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
    reject_associated_types(root, Some(budget))?;
    fn walk(ty: &Ty, budget: &PlanBudget, depth: usize) -> Result<(), String> {
        budget.check_depth(depth)?;
        if ty.is_nominal_back_edge() {
            return Ok(());
        }
        budget.charge_work(1)?;
        match ty {
            Ty::Closure(closure) => {
                for ty in closure.types() {
                    walk(ty, budget, depth + 1)?;
                }
            }
            Ty::AssociatedType(_) => {
                return Err(
                    "unsupported: residual associated type projection in generic argument".into(),
                );
            }
            // References as generic/trait actuals and aggregate fields carry
            // loans checked by ownership analysis (G3). Unsized referents
            // behind a reference are fine; bare slices are not actuals.
            Ty::Ref { referent, .. } => match referent.as_ref() {
                Ty::Str | Ty::Slice { .. } | Ty::Dyn(_) => {}
                other => walk(other, budget, depth + 1)?,
            },
            Ty::Slice { .. } | Ty::Str => {
                return Err("slice generic arguments are unsupported".into());
            }
            Ty::Tuple(fields) => {
                for field in fields.iter() {
                    walk(field, budget, depth + 1)?;
                }
            }
            Ty::Array { element, .. }
            | Ty::RawPtr {
                pointee: element, ..
            } => walk(element, budget, depth + 1)?,
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
            Ty::Param(_)
            | Ty::Int(_)
            | Ty::Float(_)
            | Ty::Char
            | Ty::Bool
            | Ty::Unit
            | Ty::Dyn(_)
            | Ty::Never => {}
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
                let Ty::Enum(value) = self.cx.intern_substituted_nominal(
                    &NominalSubstitutionRequest {
                        original: variant.owner,
                        arguments: &args,
                    },
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
    fn pattern(&mut self, pattern: &mut MatchPattern) -> Result<(), String> {
        match pattern {
            MatchPattern::Alternatives { ty, rows } => {
                *ty = self.ty(ty)?;
                self.budget.charge_work(rows.len())?;
                for row in rows {
                    self.budget.charge_work(row.len())?;
                    for test in row {
                        self.path(&mut test.projection)?;
                    }
                }
            }
            MatchPattern::EnumVariant { ty, bindings, .. } => {
                self.enum_ty(ty)?;
                self.bindings(bindings)?;
            }
            MatchPattern::Aggregate { ty, bindings } => {
                *ty = self.ty(ty)?;
                self.bindings(bindings)?;
            }
            MatchPattern::Decision {
                ty,
                tests,
                bindings,
            } => {
                *ty = self.ty(ty)?;
                self.budget.charge_work(tests.len())?;
                for test in tests {
                    self.path(&mut test.projection)?;
                    if let PatternTestKind::EnumVariant { ty, .. } = &mut test.kind {
                        self.enum_ty(ty)?;
                    }
                }
                self.bindings(bindings)?;
            }
            _ => {}
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
                Stmt::LetElse { ty, pattern, .. } => {
                    *ty = self.ty(ty)?;
                    self.pattern(pattern)?;
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
pub const MAX_ARRAY_LEN: usize = 65535;

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
    I128,
    U128,
}

impl IntTy {
    pub const fn bits(self) -> u32 {
        match self {
            Self::I8 | Self::U8 => 8,
            Self::I16 | Self::U16 => 16,
            Self::I32 | Self::U32 => 32,
            Self::I128 | Self::U128 => 128,
            Self::I64 | Self::Isize | Self::U64 | Self::Usize => 64,
        }
    }

    pub const fn is_signed(self) -> bool {
        matches!(
            self,
            Self::I8 | Self::I16 | Self::I32 | Self::I64 | Self::Isize | Self::I128
        )
    }

    pub const fn mask(self) -> u128 {
        u128::MAX >> (128 - self.bits())
    }

    /// The same-width unsigned type.
    pub const fn unsigned(self) -> Self {
        match self {
            Self::I8 | Self::U8 => Self::U8,
            Self::I16 | Self::U16 => Self::U16,
            Self::I32 | Self::U32 => Self::U32,
            Self::I64 | Self::U64 => Self::U64,
            Self::Isize | Self::Usize => Self::Usize,
            Self::I128 | Self::U128 => Self::U128,
        }
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
            Self::I128 => "i128",
            Self::U128 => "u128",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntValue {
    pub ty: IntTy,
    pub bits: u128,
}

impl IntValue {
    pub fn validate(self) -> Result<(), String> {
        if self.bits & !self.ty.mask() != 0 {
            Err("integer constant contains noncanonical high bits".into())
        } else {
            Ok(())
        }
    }

    /// Order-preserving unsigned image: signed values flip their sign bit.
    pub const fn ordinal(self) -> u128 {
        if self.ty.is_signed() {
            self.bits ^ (1u128 << (self.ty.bits() - 1))
        } else {
            self.bits
        }
    }

    pub const fn i32(value: i32) -> Self {
        Self {
            ty: IntTy::I32,
            bits: value as u32 as u128,
        }
    }

    pub const fn usize(value: u64) -> Self {
        Self {
            ty: IntTy::Usize,
            bits: value as u128,
        }
    }
}

/// Binary IEEE-754 primitives. See FLOATS.md for the bounded contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatTy {
    F32,
    F64,
}

impl FloatTy {
    pub const fn bits(self) -> u32 {
        match self {
            Self::F32 => 32,
            Self::F64 => 64,
        }
    }

    pub const fn mask(self) -> u64 {
        u64::MAX >> (64 - self.bits())
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
}

/// Canonical raw IEEE bits. F32 occupies the low 32 bits, upper bits zero.
/// Equality is bit identity (distinguishes -0.0 and NaN payloads), which is
/// the correct notion for constants, never for runtime `==`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FloatValue {
    pub ty: FloatTy,
    pub bits: u64,
}

impl FloatValue {
    pub fn validate(self) -> Result<(), String> {
        if self.bits & !self.ty.mask() != 0 {
            Err("float constant contains noncanonical high bits".into())
        } else {
            Ok(())
        }
    }

    pub fn f64(value: f64) -> Self {
        Self {
            ty: FloatTy::F64,
            bits: value.to_bits(),
        }
    }

    pub fn f32(value: f32) -> Self {
        Self {
            ty: FloatTy::F32,
            bits: u64::from(value.to_bits()),
        }
    }
}

/// Endpoint of a numeric `as` conversion involving at least one float.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumTy {
    Int(IntTy),
    Float(FloatTy),
}

impl NumTy {
    pub fn of(ty: &Ty) -> Option<Self> {
        match ty {
            Ty::Int(t) => Some(Self::Int(*t)),
            Ty::Float(t) => Some(Self::Float(*t)),
            _ => None,
        }
    }

    pub fn ty(self) -> Ty {
        match self {
            Self::Int(t) => Ty::Int(t),
            Self::Float(t) => Ty::Float(t),
        }
    }
}

/// Shared handle to an immutable nominal definition. `Back` is the cycle-
/// closing edge of a recursive type (`enum List { Cons(Box<List>) }`): the
/// interner leaks one strong reference of every definition that has back
/// edges, so a completed back edge always points at a live definition.
pub enum NominalRef<D> {
    Strong(Arc<D>),
    Back(std::sync::Weak<D>, NominalAppId),
}
impl<D> Clone for NominalRef<D> {
    fn clone(&self) -> Self {
        match self {
            Self::Strong(a) => Self::Strong(a.clone()),
            Self::Back(w, id) => Self::Back(w.clone(), *id),
        }
    }
}
impl<D> NominalRef<D> {
    fn get(&self) -> &D {
        match self {
            Self::Strong(a) => a,
            Self::Back(w, _) => {
                assert!(
                    w.strong_count() > 0,
                    "recursive nominal definition read before completion"
                );
                // SAFETY: completed recursive definitions are leaked (strong
                // count never returns to zero), so the allocation stays live.
                unsafe { &*w.as_ptr() }
            }
        }
    }
    /// A cycle-closing edge. Type walkers treat it as a leaf: its definition
    /// is reached (and checked) through the strong outer application.
    pub fn is_back_edge(&self) -> bool {
        matches!(self, Self::Back(..))
    }
}

#[derive(Clone)]
pub struct EnumTy(NominalRef<EnumDef>);

impl EnumTy {
    pub(super) fn new(def: EnumDef) -> Self {
        Self(NominalRef::Strong(Arc::new(def)))
    }
    pub fn from_ref(r: NominalRef<EnumDef>) -> Self {
        Self(r)
    }
    /// Owned definition of a freshly built, unshared strong handle.
    pub fn into_def(self) -> Result<EnumDef, String> {
        match self.0 {
            NominalRef::Strong(a) => Arc::try_unwrap(a).map_err(|_| "shared nominal".into()),
            NominalRef::Back(..) => Err("back edge has no owned definition".into()),
        }
    }
    pub fn is_back_edge(&self) -> bool {
        self.0.is_back_edge()
    }
    /// Canonical application identity, readable even on a back edge whose
    /// definition is still under construction.
    pub fn id(&self) -> NominalAppId {
        match &self.0 {
            NominalRef::Strong(a) => a.id,
            NominalRef::Back(_, id) => *id,
        }
    }

    pub fn def(&self) -> &EnumDef {
        self.0.get()
    }

    pub fn has_payload(&self) -> bool {
        self.def()
            .variants
            .iter()
            .any(|variant| variant.shape != VariantShape::Unit)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.def().variants.is_empty() || self.def().variants.len() > MAX_ENUM_VARIANTS {
            return Err("enum variant count must be between 1 and 4096".into());
        }
        let mut fields = 0usize;
        for variant in self.def().variants.iter() {
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
        if self.is_back_edge() {
            return f.debug_struct("EnumTy").field("back", &self.id()).finish();
        }
        let name = &self.def().name;
        let end = name
            .char_indices()
            .nth(128)
            .map_or(name.len(), |(at, _)| at);
        f.debug_struct("EnumTy")
            .field("id", &self.id())
            .field("name", &&name[..end])
            .finish_non_exhaustive()
    }
}

impl PartialEq for EnumTy {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}
impl Eq for EnumTy {}
impl Hash for EnumTy {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

/// Source visibility normalized to a checked canonical module subtree.
/// A crate-root restriction does not grant access from another root.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Visibility {
    Public,
    Restricted(super::module_identity::ModuleId),
}

impl Visibility {
    pub fn is_public(&self) -> bool {
        matches!(self, Self::Public)
    }

    pub fn accessible_from(
        &self,
        modules: &super::module_identity::ModuleArena,
        module: super::module_identity::ModuleId,
    ) -> Result<bool, super::module_identity::ModuleError> {
        modules.check(module)?;
        match self {
            Self::Public => Ok(true),
            Self::Restricted(scope) => modules.is_descendant(module, *scope),
        }
    }

    /// Whether the alias's entire audience is permitted by this original.
    pub fn allows_reexport(
        &self,
        modules: &super::module_identity::ModuleArena,
        alias: &Self,
    ) -> Result<bool, super::module_identity::ModuleError> {
        modules
            .budget()
            .check()
            .map_err(super::module_identity::ModuleError::from)?;
        if let Self::Restricted(scope) = self {
            modules.check(*scope)?;
        }
        if let Self::Restricted(scope) = alias {
            modules.check(*scope)?;
        }
        match (self, alias) {
            (Self::Public, _) => Ok(true),
            (Self::Restricted(_), Self::Public) => Ok(false),
            (Self::Restricted(scope), Self::Restricted(alias)) => {
                modules.is_descendant(*alias, *scope)
            }
        }
    }

    /// Schema admission must check the definition even when it is public.
    /// A restricted audience is a lexical ancestor, never an alias namespace.
    pub fn validate_for(
        &self,
        modules: &super::module_identity::ModuleArena,
        definition: super::module_identity::ModuleId,
    ) -> Result<(), super::module_identity::ModuleError> {
        modules.check(definition)?;
        if let Self::Restricted(scope) = self
            && !modules.is_descendant(definition, *scope)?
        {
            return Err(super::module_identity::ModuleError::InvalidRestriction);
        }
        Ok(())
    }
}

impl EnumDef {
    /// Never published: stands in only when construction failed.
    pub(crate) fn placeholder(
        id: NominalAppId,
        source_decl: DeclId,
        module: super::module_identity::ModuleId,
    ) -> Self {
        Self {
            id,
            source_decl,
            type_args: Arc::from(Vec::new()),
            name: String::new(),
            module,
            visibility: Visibility::Public,
            variants: Arc::from(Vec::new()),
        }
    }
}
impl StructDef {
    /// Never published: stands in only when construction failed.
    pub(crate) fn placeholder(
        id: NominalAppId,
        source_decl: DeclId,
        module: super::module_identity::ModuleId,
    ) -> Self {
        Self {
            id,
            source_decl,
            type_args: Arc::from(Vec::new()),
            name: String::new(),
            module,
            visibility: Visibility::Public,
            fields: Arc::from(Vec::new()),
        }
    }
}

impl Ty {
    /// A cycle-closing reference of a recursive nominal type. Recursive type
    /// walkers treat it as a leaf: the outer strong application owns it.
    pub fn is_nominal_back_edge(&self) -> bool {
        match self {
            Ty::Struct(t) => t.is_back_edge(),
            Ty::Enum(t) => t.is_back_edge(),
            _ => false,
        }
    }
}

/// True when `ty` stores application `id` by value (not behind a pointer):
/// such a recursive type has infinite size.
pub fn by_value_contains(ty: &Ty, id: NominalAppId, depth: usize) -> Result<bool, String> {
    if depth > 256 {
        return Err("type nesting resource limit exceeded".into());
    }
    Ok(match ty {
        Ty::Struct(t) => {
            if depth > 0 && t.id() == id {
                return Ok(true);
            }
            if t.is_back_edge() {
                return Ok(false);
            }
            for f in t.def().fields.iter() {
                if by_value_contains(&f.ty, id, depth + 1)? {
                    return Ok(true);
                }
            }
            false
        }
        Ty::Enum(t) => {
            if depth > 0 && t.id() == id {
                return Ok(true);
            }
            if t.is_back_edge() {
                return Ok(false);
            }
            for v in t.def().variants.iter() {
                for f in v.fields.iter() {
                    if by_value_contains(&f.ty, id, depth + 1)? {
                        return Ok(true);
                    }
                }
            }
            false
        }
        Ty::Tuple(fields) => {
            for f in fields.iter() {
                if by_value_contains(f, id, depth + 1)? {
                    return Ok(true);
                }
            }
            false
        }
        Ty::Array { element, .. } => by_value_contains(element, id, depth + 1)?,
        _ => false,
    })
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub id: NominalAppId,
    pub source_decl: DeclId,
    pub type_args: Arc<[Ty]>,
    pub name: String,
    pub module: super::module_identity::ModuleId,
    pub visibility: Visibility,
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
    /// Runtime index read exactly once when a borrow address is materialized.
    /// Canonical overlap never uses the index value to prove disjointness.
    DynamicIndex(LocalId),
    Deref,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BorrowKind {
    Shared,
    Mut,
}

/// Index of the first `Deref` in `path` whose base has raw pointer type.
/// Storage reached through a raw pointer is outside every tracked canonical
/// root: analyses check the pointer prefix and treat the rest as untracked.
pub fn raw_deref_index(root: &Ty, path: &[Projection]) -> Option<usize> {
    let mut ty = root;
    let mut steps = path.iter().enumerate();
    while let Some((index, step)) = steps.next() {
        ty = match step {
            Projection::Deref => match ty {
                Ty::RawPtr { .. } => return Some(index),
                Ty::Ref { referent, .. } => referent,
                _ => return None,
            },
            Projection::Field(field) => ty.field_ty(*field)?,
            Projection::ConstantIndex(_) | Projection::DynamicIndex(_) => match ty {
                Ty::Array { element, .. } => element,
                _ => return None,
            },
            Projection::Downcast(variant) => {
                let Ty::Enum(owner) = ty else {
                    return None;
                };
                let Some((_, Projection::Field(field))) = steps.next() else {
                    return None;
                };
                &owner
                    .def()
                    .variants
                    .get(variant.index)?
                    .fields
                    .get(*field)?
                    .ty
            }
        };
    }
    None
}

/// Checks structural typing, not the runtime proof needed to read a payload.
pub fn projected_ty<'a>(root: &'a Ty, path: &[Projection]) -> Result<&'a Ty, String> {
    projected_ty_inner(root, path, false)
}

/// Array operand of a runtime index statement (`AssignIndex{Op}`,
/// `IndexAssign{Op}`, `IndexInspect`). That statement indexes the result
/// again, so a trailing runtime row is an interior step (NESTED-INDEX.md).
/// With `mutable`, also checks static mutation authority, including
/// unreachable metadata; binding lifetime, initialization and exclusive loan
/// access remain ownership-analysis duties.
pub(super) fn indexed_array_ty<'a>(
    root: &'a Ty,
    path: &[Projection],
    mutable: bool,
) -> Result<&'a Ty, String> {
    if let Some((Projection::DynamicIndex(_), prefix)) = path.split_last() {
        // The prefix is itself followed by this index step, so a trailing
        // runtime row in the prefix is interior too.
        let ty = indexed_array_ty(root, prefix, mutable)?;
        return match ty {
            Ty::Array { element, len }
                if *len <= MAX_ARRAY_LEN && matches!(element.as_ref(), Ty::Array { .. }) =>
            {
                Ok(element)
            }
            _ => Err("runtime place index requires array or slice type".into()),
        };
    }
    projected_ty_inner(root, path, mutable)
}

fn projected_ty_inner<'a>(
    root: &'a Ty,
    path: &[Projection],
    mutable: bool,
) -> Result<&'a Ty, String> {
    reject_associated_types(root, None)?;
    if path.len() > MAX_PATTERN_PROJECTION_DEPTH {
        return Err("projection depth exceeds limit 128".into());
    }
    let mut ty = root;
    let mut path = path.iter();
    while let Some(projection) = path.next() {
        ty = match *projection {
            Projection::Deref => {
                if let Ty::RawPtr { kind, pointee } = ty {
                    if mutable && *kind != BorrowKind::Mut {
                        return Err("mutation through a *const raw pointer".into());
                    }
                    ty = pointee;
                    continue;
                }
                let Ty::Ref { kind, referent } = ty else {
                    return Err("dereference requires reference type".into());
                };
                if mutable && *kind != BorrowKind::Mut {
                    return Err(
                        "compound indexed mutation requires mutable reference at every dereference"
                            .into(),
                    );
                }
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
            Projection::DynamicIndex(_) => {
                let element = match ty {
                    Ty::Array { element, len } if *len <= MAX_ARRAY_LEN => element,
                    Ty::Slice { element } => element,
                    _ => return Err("runtime place index requires array or slice type".into()),
                };
                // Any element type may be a runtime element loan or the base
                // of a further projection (`&v[i]`, `&v[i].x`, `m[i][j]`).
                // Analyses treat a runtime element's footprint as the whole
                // array and never prove disjointness by index value.
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
pub struct StructTy(NominalRef<StructDef>);

impl std::fmt::Debug for StructTy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_back_edge() {
            return f
                .debug_struct("StructTy")
                .field("back", &self.id())
                .finish();
        }
        // Types are a shared DAG, not a tree to expand into diagnostics. Names
        // themselves can be long, so bound their diagnostic rendering as well.
        let name = &self.def().name;
        let end = name
            .char_indices()
            .nth(128)
            .map_or(name.len(), |(at, _)| at);
        f.debug_struct("StructTy")
            .field("id", &self.id())
            .field("name", &&name[..end])
            .finish_non_exhaustive()
    }
}

impl StructTy {
    /// The declaration builder must validate the entire by-value dependency DAG
    /// before constructing definitions in dependency-first order.
    pub(super) fn new(def: StructDef) -> Self {
        Self(NominalRef::Strong(Arc::new(def)))
    }
    pub fn from_ref(r: NominalRef<StructDef>) -> Self {
        Self(r)
    }
    /// Owned definition of a freshly built, unshared strong handle.
    pub fn into_def(self) -> Result<StructDef, String> {
        match self.0 {
            NominalRef::Strong(a) => Arc::try_unwrap(a).map_err(|_| "shared nominal".into()),
            NominalRef::Back(..) => Err("back edge has no owned definition".into()),
        }
    }
    pub fn is_back_edge(&self) -> bool {
        self.0.is_back_edge()
    }
    /// Canonical application identity, readable even on a back edge whose
    /// definition is still under construction.
    pub fn id(&self) -> NominalAppId {
        match &self.0 {
            NominalRef::Strong(a) => a.id,
            NominalRef::Back(_, id) => *id,
        }
    }

    pub fn def(&self) -> &StructDef {
        self.0.get()
    }

    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.def()
            .fields
            .iter()
            .position(|field| field.name == name)
    }
}

impl PartialEq for StructTy {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for StructTy {}

impl Hash for StructTy {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
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
    pub module: super::module_identity::ModuleId,
    pub visibility: Visibility,
    /// Declaration order. No layout offsets or implicit Copy permission.
    pub fields: Arc<[FieldDef]>,
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub visibility: Visibility,
    /// The source field type names a declared (non-`'static`) lifetime, so
    /// its references carry loans rather than being `&'static` data.
    pub lifetime_bound: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalId(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId(pub usize);
/// A Unicode scalar, validated again at every checked IR boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CharValue {
    pub value: u32,
}
impl CharValue {
    pub fn validate(self) -> Result<(), String> {
        if char::from_u32(self.value).is_none() {
            return Err("character constant is not a Unicode scalar".into());
        }
        Ok(())
    }
}

/// Resolved control-flow target, allocated monotonically within one function.
/// Source labels resolve to these function-local targets before CFG lowering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoopId(pub usize);

/// Resolved semantic types. Inference variables never cross this boundary.
/// Tuple structure is immutable and shared cheaply across checked facts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Closure(Arc<ClosureTy>),
    AssociatedType(Arc<AssociatedTypeProjection>),
    Param(ParamId),
    Int(IntTy),
    Char,
    /// Unsized UTF-8 `str`. Only an admitted shared root reference `&str`
    /// has a native descriptor (data pointer, byte length).
    Str,
    Float(FloatTy),
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
    /// Unsized slice. Only an admitted shared root reference has a native
    /// descriptor; the exact element type is retained through substitution.
    Slice {
        element: Arc<Ty>,
    },
    /// Nominal and always non-Copy in this slice, even with only scalar fields.
    Struct(StructTy),
    Enum(EnumTy),
    /// Raw pointer `*const T` (Shared) or `*mut T` (Mut). Always Copy, carries
    /// no loan or provenance authority, and is only dereferenced in `unsafe`.
    RawPtr {
        kind: BorrowKind,
        pointee: Arc<Ty>,
    },
    /// Unsized trait object `dyn Trait<Args>`. Admitted only behind a shared or
    /// mutable reference, whose native carrier is two words (data, vtable).
    /// The principal trait application has a placeholder `Self` of `Unit`;
    /// the hidden concrete type is erased at the unsizing coercion.
    Dyn(Arc<DynTy>),
    /// The semantic never type, distinct from runtime completion information.
    Never,
}

/// Supported pointer-representation casts. All are one-word bit copies.
/// `&T`/`&mut T` to `*const T`, `&mut T` to `*mut T`, any raw pointer to any
/// raw pointer, raw pointer to `usize`, and `usize` to raw pointer.
pub fn ptr_cast_valid(from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (
            Ty::Ref { kind, referent },
            Ty::RawPtr {
                kind: target,
                pointee,
            },
        ) => {
            referent == pointee
                && !matches!(referent.as_ref(), Ty::Slice { .. })
                && (*target == BorrowKind::Shared || *kind == BorrowKind::Mut)
        }
        (Ty::RawPtr { pointee: a, .. }, Ty::RawPtr { pointee: b, .. }) => {
            !matches!(a.as_ref(), Ty::Slice { .. }) && !matches!(b.as_ref(), Ty::Slice { .. })
        }
        (Ty::RawPtr { pointee, .. }, Ty::Int(IntTy::Usize))
        | (Ty::Int(IntTy::Usize), Ty::RawPtr { pointee, .. }) => {
            !matches!(pointee.as_ref(), Ty::Slice { .. })
        }
        _ => false,
    }
}

/// Principal trait of a trait object. This milestone admits nongeneric
/// principal traits without associated types, so the object type is a rigid
/// metadata leaf identified by its trait declaration alone.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DynTy {
    pub principal: DeclId,
}

/// Words of a pointer to `pointee` in the private ABI: two for unsized slice
/// and trait object pointees (data + len / data + vtable), one otherwise.
/// Shared by references and (later) raw pointers and Box.
pub fn pointer_words(pointee: &Ty) -> usize {
    match pointee {
        Ty::Slice { .. } | Ty::Dyn(_) => 2,
        _ => 1,
    }
}

/// Structural residual guard only. This does not establish declaration ownership,
/// trait membership, normalization, Sized, Copy, or a layout. Every metadata child
/// is visited, including nominal actuals and zero-length array elements.
pub fn reject_associated_types(root: &Ty, budget: Option<&PlanBudget>) -> Result<(), String> {
    if type_metadata_walk(root, budget, None)? {
        return Err(
            "unsupported: residual associated type projection in executable metadata".into(),
        );
    }
    Ok(())
}

pub(super) fn contains_associated_types(root: &Ty, budget: &PlanBudget) -> Result<bool, String> {
    type_metadata_walk(root, Some(budget), None)
}

fn type_metadata_walk(
    root: &Ty,
    budget: Option<&PlanBudget>,
    facts: Option<FactsCtx<'_>>,
) -> Result<bool, String> {
    type Key = (u8, usize);
    type Entry = (Key, Option<(bool, usize)>);
    #[derive(Default)]
    struct Memo {
        entries: Vec<Entry>,
        _base: Option<PlanReservation>,
        extra: Option<PlanReservation>,
    }
    impl Memo {
        fn insert(
            &mut self,
            key: Key,
            budget: Option<&PlanBudget>,
            work: &mut usize,
        ) -> Result<usize, String> {
            if self.entries.len() == self.entries.capacity() {
                let target = self
                    .entries
                    .capacity()
                    .checked_mul(2)
                    .ok_or("projection memo capacity overflow")?
                    .max(4);
                let bytes = target
                    .checked_mul(std::mem::size_of::<Entry>())
                    .ok_or("projection memo byte overflow")?;
                let mut staged = Self {
                    _base: budget.map(|b| b.reserve_bytes(bytes)).transpose()?,
                    ..Self::default()
                };
                for _ in &self.entries {
                    charge(budget, work)?;
                }
                staged
                    .entries
                    .try_reserve_exact(target)
                    .map_err(|_| "projection memo allocation resource failure")?;
                let excess = staged
                    .entries
                    .capacity()
                    .checked_sub(target)
                    .and_then(|n| n.checked_mul(std::mem::size_of::<Entry>()))
                    .ok_or("projection memo capacity overflow")?;
                staged.extra = budget.map(|b| b.reserve_bytes(excess)).transpose()?;
                staged.entries.append(&mut self.entries);
                *self = staged;
            }
            let index = self.entries.len();
            self.entries.push((key, None));
            Ok(index)
        }
    }
    fn charge(budget: Option<&PlanBudget>, work: &mut usize) -> Result<(), String> {
        if budget.is_some_and(PlanBudget::has_resource_failure) {
            return Err("planning resource budget already exhausted".into());
        }
        *work = work
            .checked_add(1)
            .ok_or("associated type traversal work overflow")?;
        if *work > MAX_PATTERN_PROJECTION_WORK {
            return Err("associated type traversal resource limit exceeded".into());
        }
        if let Some(budget) = budget {
            budget.charge_work(1)?;
        }
        Ok(())
    }
    fn walk(
        ty: &Ty,
        budget: Option<&PlanBudget>,
        depth: usize,
        work: &mut usize,
        memo: &mut Memo,
        facts: Option<FactsCtx<'_>>,
    ) -> Result<(bool, usize), String> {
        charge(budget, work)?;
        if depth > MAX_PATTERN_PROJECTION_DEPTH {
            return Err("associated type traversal depth limit exceeded".into());
        }
        if ty.is_nominal_back_edge() {
            return Ok((false, 0));
        }
        if let Some(budget) = budget {
            budget.check_depth(depth)?;
        }
        let key = match ty {
            Ty::Closure(closure) => (8, Arc::as_ptr(closure) as usize),
            Ty::AssociatedType(value) => {
                if value.trait_ref.args.len() > 32 {
                    return Err("trait argument limit exceeded".into());
                }
                (0, Arc::as_ptr(value) as usize)
            }
            Ty::Ref { referent, .. } => (1, Arc::as_ptr(referent) as usize),
            Ty::Array { element, len } => {
                if *len > MAX_ARRAY_LEN {
                    return Err("array length limit exceeded".into());
                }
                (2, Arc::as_ptr(element) as usize)
            }
            Ty::Slice { element } => (3, Arc::as_ptr(element) as usize),
            Ty::RawPtr { pointee, .. } => (7, Arc::as_ptr(pointee) as usize),
            Ty::Tuple(fields) => (4, fields.as_ptr() as usize),
            Ty::Struct(value) => {
                if value.def().type_args.len() > 32 {
                    return Err("nominal argument limit exceeded".into());
                }
                (5, std::ptr::from_ref(value.def()) as usize)
            }
            Ty::Enum(value) => {
                if value.def().type_args.len() > 32
                    || value.def().variants.len() > MAX_ENUM_VARIANTS
                {
                    return Err("nominal metadata limit exceeded".into());
                }
                (6, std::ptr::from_ref(value.def()) as usize)
            }
            Ty::Param(parameter) => {
                if let Some(facts) = facts {
                    let FactsCtx::Universal(env) = facts else {
                        return Err("type parameter is outside its authoritative binder".into());
                    };
                    for _ in env.scopes() {
                        charge(budget, work)?;
                    }
                    if !env.contains(*parameter) {
                        return Err("type parameter is outside its authoritative binder".into());
                    }
                }
                return Ok((false, 0));
            }
            Ty::Int(_)
            | Ty::Float(_)
            | Ty::Char
            | Ty::Str
            | Ty::Bool
            | Ty::Unit
            | Ty::Dyn(_)
            | Ty::Never => {
                return Ok((false, 0));
            }
        };
        for (candidate, result) in &memo.entries {
            charge(budget, work)?;
            if *candidate == key {
                let (found, height) = result.ok_or("associated type metadata cycle")?;
                let deepest = depth
                    .checked_add(height)
                    .ok_or("associated type traversal depth overflow")?;
                if deepest > MAX_PATTERN_PROJECTION_DEPTH {
                    return Err("associated type traversal depth limit exceeded".into());
                }
                if let Some(budget) = budget {
                    budget.check_depth(deepest)?;
                }
                return Ok((found, height));
            }
        }
        let index = memo.insert(key, budget, work)?;
        if let Ty::Enum(value) = ty {
            for _ in value.def().variants.iter() {
                charge(budget, work)?;
            }
        }
        let mut found = false;
        let mut height = 0usize;
        let mut child = |ty: &Ty| -> Result<(), String> {
            let (has, child_height) = walk(ty, budget, depth + 1, work, memo, facts)?;
            found |= has;
            height = height.max(
                child_height
                    .checked_add(1)
                    .ok_or("associated type traversal depth overflow")?,
            );
            Ok(())
        };
        match ty {
            Ty::Closure(closure) => {
                for ty in closure.types() {
                    child(ty)?;
                }
            }
            Ty::AssociatedType(projection) => {
                if projection.trait_ref.args.len() > 32 {
                    return Err("trait argument limit exceeded".into());
                }
                child(&projection.trait_ref.self_ty)?;
                for argument in projection.trait_ref.args.iter() {
                    child(argument)?;
                }
                found = true;
            }
            Ty::Ref { referent, .. }
            | Ty::RawPtr {
                pointee: referent, ..
            }
            | Ty::Array {
                element: referent, ..
            }
            | Ty::Slice { element: referent } => child(referent)?,
            Ty::Tuple(fields) => {
                for field in fields.iter() {
                    child(field)?;
                }
            }
            Ty::Struct(value) => {
                if value.def().type_args.len() > 32 {
                    return Err("nominal argument limit exceeded".into());
                }
                for argument in value.def().type_args.iter() {
                    child(argument)?;
                }
                for field in value.def().fields.iter() {
                    child(&field.ty)?;
                }
            }
            Ty::Enum(value) => {
                if value.def().type_args.len() > 32
                    || value.def().variants.len() > MAX_ENUM_VARIANTS
                {
                    return Err("nominal metadata limit exceeded".into());
                }
                for argument in value.def().type_args.iter() {
                    child(argument)?;
                }
                for variant in value.def().variants.iter() {
                    for field in variant.fields.iter() {
                        child(&field.ty)?;
                    }
                }
            }
            Ty::Param(_)
            | Ty::Int(_)
            | Ty::Float(_)
            | Ty::Char
            | Ty::Str
            | Ty::Bool
            | Ty::Unit
            | Ty::Dyn(_)
            | Ty::Never => {}
        }
        memo.entries[index].1 = Some((found, height));
        Ok((found, height))
    }
    walk(root, budget, 0, &mut 0, &mut Memo::default(), facts).map(|(found, _)| found)
}

impl Ty {
    /// Kind and element of a root slice reference `&[T]` or `&mut [T]`
    /// (MUTABLE-SLICES.md). Grants no element admission or layout.
    pub fn slice_ref_element(&self) -> Option<(BorrowKind, &Ty)> {
        match self {
            Self::Ref { kind, referent } => match referent.as_ref() {
                Self::Slice { element } => Some((*kind, element.as_ref())),
                _ => None,
            },
            _ => None,
        }
    }

    /// Element of exactly a mutable root slice reference `&mut [T]`.
    pub fn mut_slice_element(&self) -> Option<&Ty> {
        match self.slice_ref_element() {
            Some((BorrowKind::Mut, element)) => Some(element),
            _ => None,
        }
    }

    /// Element of the private two-word (data, len) descriptor of `&[T]`,
    /// `&mut [T]` or `&str`. Physical transport only.
    pub fn descriptor_element(&self) -> Option<&Ty> {
        self.shared_descriptor_element()
            .or_else(|| self.mut_slice_element())
    }

    /// Kind, element and extent of a root array reference `&[T; N]` or
    /// `&mut [T; N]` (the source of an unsizing coercion).
    pub fn array_ref_element_len(&self) -> Option<(BorrowKind, &Ty, usize)> {
        match self {
            Self::Ref { kind, referent } => match referent.as_ref() {
                Self::Array { element, len } => Some((*kind, element.as_ref(), *len)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Element of exactly a shared root slice reference, without granting
    /// element admission, Copy permission, or a concrete native layout.
    pub fn shared_slice_element(&self) -> Option<&Ty> {
        match self {
            Self::Ref {
                kind: BorrowKind::Shared,
                referent,
            } => match referent.as_ref() {
                Self::Slice { element } => Some(element.as_ref()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Exactly shared `&str`.
    pub fn is_shared_str(&self) -> bool {
        matches!(self, Self::Ref { kind: BorrowKind::Shared, referent } if **referent == Self::Str)
    }

    /// The canonical `&str` type.
    pub fn shared_str_ty() -> Self {
        Self::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Self::Str),
        }
    }

    /// Element of the private two-word (data, len) descriptor carrying this
    /// value: `&[T]` carries T, `&str` carries UTF-8 bytes. This is physical
    /// transport only; it grants no slice indexing or element typing to str.
    pub fn shared_descriptor_element(&self) -> Option<&Ty> {
        const BYTE: &Ty = &Ty::Int(IntTy::U8);
        if self.is_shared_str() {
            return Some(BYTE);
        }
        self.shared_slice_element()
    }

    /// Exact element and extent of a shared root array reference.
    pub fn shared_array_element_len(&self) -> Option<(&Ty, usize)> {
        match self {
            Self::Ref {
                kind: BorrowKind::Shared,
                referent,
            } => match referent.as_ref() {
                Self::Array { element, len } => Some((element.as_ref(), *len)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Exactly shared `&[u8]`.
    pub fn shared_byte_slice(&self) -> bool {
        matches!(
            self,
            Self::Ref { kind: BorrowKind::Shared, referent }
                if matches!(&**referent, Self::Slice { element } if **element == Self::Int(IntTy::U8))
        )
    }

    /// `Some(N)` exactly for shared `&[u8; N]`.
    pub fn shared_byte_array_len(&self) -> Option<usize> {
        match self {
            Self::Ref {
                kind: BorrowKind::Shared,
                referent,
            } => match &**referent {
                Self::Array { element, len } if **element == Self::Int(IntTy::U8) => Some(*len),
                _ => None,
            },
            _ => None,
        }
    }

    /// The canonical shared byte slice type `&[u8]`.
    pub fn shared_byte_slice_ty() -> Self {
        Self::Ref {
            kind: BorrowKind::Shared,
            referent: Arc::new(Self::Slice {
                element: Arc::new(Self::Int(IntTy::U8)),
            }),
        }
    }

    /// Whether any slice type occurs anywhere inside this type. Shared type
    /// structure (nominal definitions, tuple/array/reference Arcs) is visited
    /// once, so exponentially large expanded layouts stay linear in the DAG.
    pub fn contains_slice(&self) -> bool {
        let mut pending = vec![self];
        let mut seen = std::collections::HashSet::new();
        while let Some(ty) = pending.pop() {
            match ty {
                Self::Closure(closure) => {
                    if seen.insert(Arc::as_ptr(closure) as usize) {
                        pending.extend(closure.captures());
                    }
                }
                Self::AssociatedType(projection) => {
                    if seen.insert(Arc::as_ptr(projection) as usize) {
                        pending.push(&projection.trait_ref.self_ty);
                        pending.extend(projection.trait_ref.args.iter());
                    }
                }
                Self::Slice { .. } => return true,
                Self::Ref {
                    referent: inner, ..
                }
                | Self::RawPtr { pointee: inner, .. }
                | Self::Array { element: inner, .. } => {
                    if seen.insert(Arc::as_ptr(inner) as usize) {
                        pending.push(inner);
                    }
                }
                Self::Tuple(fields) => {
                    if seen.insert(fields.as_ptr() as usize) {
                        pending.extend(fields.iter());
                    }
                }
                Self::Struct(value) => {
                    if seen.insert(std::ptr::from_ref(value.def()) as usize) {
                        pending.extend(value.def().type_args.iter());
                        pending.extend(value.def().fields.iter().map(|f| &f.ty));
                    }
                }
                Self::Enum(value) => {
                    if seen.insert(std::ptr::from_ref(value.def()) as usize) {
                        pending.extend(value.def().type_args.iter());
                        for variant in value.def().variants.iter() {
                            pending.extend(variant.fields.iter().map(|f| &f.ty));
                        }
                    }
                }
                Self::Param(_)
                | Self::Int(_)
                | Self::Float(_)
                | Self::Char
                | Self::Str
                | Self::Bool
                | Self::Unit
                | Self::Dyn(_)
                | Self::Never => {}
            }
        }
        false
    }

    pub fn field_count(&self) -> Option<usize> {
        match self {
            Self::Closure(closure) => Some(closure.captures().len()),
            Self::Tuple(fields) => Some(fields.len()),
            Self::Struct(ty) => Some(ty.def().fields.len()),
            _ => None,
        }
    }

    pub fn field_ty(&self, index: usize) -> Option<&Ty> {
        match self {
            Self::Closure(closure) => closure.captures().get(index),
            Self::Tuple(fields) => fields.get(index),
            Self::Struct(ty) => ty.def().fields.get(index).map(|field| &field.ty),
            _ => None,
        }
    }

    pub fn is_copy(&self) -> bool {
        match self {
            Self::Ref { kind, .. } => *kind == BorrowKind::Shared,
            Self::Int(_)
            | Self::Float(_)
            | Self::Char
            | Self::Bool
            | Self::Unit
            | Self::Never
            | Self::RawPtr { .. } => true,
            Self::AssociatedType(_)
            | Self::Struct(_)
            | Self::Enum(_)
            | Self::Param(_)
            | Self::Str
            | Self::Dyn(_)
            | Self::Slice { .. } => false,
            Self::Closure(_) | Self::Tuple(_) | Self::Array { .. } => {
                let mut pending = vec![self];
                let mut seen = std::collections::HashSet::new();
                let mut arrays = std::collections::HashSet::new();
                while let Some(ty) = pending.pop() {
                    match ty {
                        Self::Ref {
                            kind: BorrowKind::Mut,
                            ..
                        } => return false,
                        Self::AssociatedType(_)
                        | Self::Struct(_)
                        | Self::Enum(_)
                        | Self::Param(_)
                        | Self::Str
                        | Self::Slice { .. } => {
                            return false;
                        }
                        Self::Closure(closure)
                            if seen.insert(Arc::as_ptr(closure).cast::<Ty>()) =>
                        {
                            pending.extend(closure.captures());
                        }
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

/// Bounded postorder DAG walk. Metadata and binder validation happens first.
fn slice_boundary_walk(
    ty: &Ty,
    allow_references: bool,
    budget: Option<&PlanBudget>,
    depth: usize,
    work: &mut usize,
) -> Result<(), String> {
    struct Scratch<T> {
        values: Vec<T>,
        base: Option<PlanReservation>,
        extra: Option<PlanReservation>,
    }
    impl<T> Scratch<T> {
        fn new() -> Self {
            Self {
                values: Vec::new(),
                base: None,
                extra: None,
            }
        }
        fn push(&mut self, value: T, budget: Option<&PlanBudget>) -> Result<(), String> {
            if self.values.len() == self.values.capacity() {
                let target = self
                    .values
                    .capacity()
                    .checked_mul(2)
                    .ok_or("slice scratch capacity overflow")?
                    .max(4);
                let bytes = target
                    .checked_mul(std::mem::size_of::<T>())
                    .ok_or("slice scratch byte overflow")?;
                let mut staged = Self::new();
                staged.base = budget.map(|b| b.reserve_bytes(bytes)).transpose()?;
                if let Some(b) = budget {
                    b.charge_work(self.values.len())?;
                }
                staged
                    .values
                    .try_reserve_exact(target)
                    .map_err(|_| "slice scratch allocation resource failure")?;
                let excess = staged
                    .values
                    .capacity()
                    .checked_sub(target)
                    .and_then(|n| n.checked_mul(std::mem::size_of::<T>()))
                    .ok_or("slice scratch capacity overflow")?;
                staged.extra = budget.map(|b| b.reserve_bytes(excess)).transpose()?;
                staged.values.append(&mut self.values);
                *self = staged;
            }
            self.values.push(value);
            Ok(())
        }
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Key(u8, usize);
    enum Frame<'a> {
        Enter(&'a Ty, usize),
        Finish(Key, usize, usize),
    }
    fn charge(budget: Option<&PlanBudget>, work: &mut usize) -> Result<(), String> {
        if let Some(b) = budget {
            if b.has_resource_failure() {
                return Err("planning resource budget already exhausted".into());
            }
            b.charge_work(1)?;
        }
        *work = work.checked_add(1).ok_or("slice type work overflow")?;
        if *work > MAX_PATTERN_PROJECTION_WORK {
            return Err("slice type resource limit exceeded".into());
        }
        Ok(())
    }
    let mut pending = Scratch::new();
    // None is a currently visiting allocation, Some(height) a complete subtree.
    let mut memo: Scratch<(Key, Option<usize>)> = Scratch::new();
    let mut heights: Scratch<usize> = Scratch::new();
    charge(budget, work)?;
    pending.push(Frame::Enter(ty, depth), budget)?;
    while let Some(frame) = pending.values.pop() {
        charge(budget, work)?;
        match frame {
            Frame::Finish(key, at, count) => {
                let mut height = 0usize;
                for _ in 0..count {
                    charge(budget, work)?;
                    height = height.max(
                        heights
                            .values
                            .pop()
                            .ok_or("slice DAG result underflow")?
                            .checked_add(1)
                            .ok_or("slice type depth overflow")?,
                    );
                }
                if let Some(b) = budget {
                    b.check_depth(at.checked_add(height).ok_or("slice type depth overflow")?)?;
                }
                let mut found = false;
                for (candidate, result) in &mut memo.values {
                    charge(budget, work)?;
                    if *candidate == key {
                        *result = Some(height);
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err("slice DAG completion has no owner".into());
                }
                heights.push(height, budget)?;
            }
            Frame::Enter(node, at) => {
                if let Some(b) = budget {
                    b.check_depth(at)?;
                }
                if node.is_nominal_back_edge() {
                    heights.push(0usize, budget)?;
                    continue;
                }
                let key = match node {
                    Ty::AssociatedType(_) => return Err("unsupported: residual associated type projection at slice boundary".into()),
                    Ty::Ref { .. } if !allow_references => return Err("unsupported: slice elements containing references or slices, or nested slice carriers".into()),
                    Ty::Slice { .. } => return Err("unsupported: slice elements containing references or slices, or nested slice carriers".into()),
                    Ty::Ref { referent, .. } => Some(Key(0, Arc::as_ptr(referent) as usize)),
                    Ty::Array { element, .. } => Some(Key(1, Arc::as_ptr(element) as usize)),
                    Ty::Tuple(fields) => Some(Key(2, fields.as_ptr() as usize)),
                    Ty::Struct(value) => Some(Key(3, value.def() as *const StructDef as usize)),
                    Ty::Enum(value) => Some(Key(4, value.def() as *const EnumDef as usize)),
                    _ => None,
                };
                let Some(key) = key else {
                    heights.push(0usize, budget)?;
                    continue;
                };
                let mut known = None;
                for (candidate, result) in &memo.values {
                    charge(budget, work)?;
                    if *candidate == key {
                        known = Some(*result);
                        break;
                    }
                }
                if let Some(result) = known {
                    let height = result.ok_or("slice type by-value cycle")?;
                    if let Some(b) = budget {
                        b.check_depth(at.checked_add(height).ok_or("slice type depth overflow")?)?;
                    }
                    heights.push(height, budget)?;
                    continue;
                }
                memo.push((key, None), budget)?;
                let finish = pending.values.len();
                pending.push(Frame::Finish(key, at, 0), budget)?;
                let next = at.checked_add(1).ok_or("slice type depth overflow")?;
                let mut count = 0usize;
                let mut child = |ty| -> Result<(), String> {
                    charge(budget, work)?;
                    count = count.checked_add(1).ok_or("slice DAG edge overflow")?;
                    pending.push(Frame::Enter(ty, next), budget)
                };
                match node {
                    Ty::Ref { referent, .. }
                    | Ty::Array {
                        element: referent, ..
                    } => child(referent)?,
                    Ty::AssociatedType(_) => {
                        return Err(
                            "unsupported: residual associated type projection in carrier".into(),
                        );
                    }
                    Ty::Tuple(fields) => {
                        for field in fields.iter() {
                            child(field)?;
                        }
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
                            for field in variant.fields.iter() {
                                child(&field.ty)?;
                            }
                        }
                    }
                    _ => return Err("slice DAG node lost compound shape".into()),
                }
                pending.values[finish] = Frame::Finish(key, at, count);
            }
        }
    }
    if heights.values.len() != 1 {
        return Err("slice DAG result count mismatch".into());
    }
    Ok(())
}

fn validate_shared_slice_element_inner(
    element: &Ty,
    facts: FactsCtx<'_>,
    budget: Option<&PlanBudget>,
) -> Result<(), String> {
    if let Some(budget) = budget {
        if budget.has_resource_failure() {
            return Err("planning resource budget already exhausted".into());
        }
        facts.validate_value_ty_with_budget(element, budget)?;
    } else {
        facts.validate_value_ty(element)?;
    }
    slice_boundary_walk(element, false, budget, 0, &mut 0)
}

/// Validate exact element metadata, Sized obligations and the current
/// reference-free slice boundary. Re-run after every concrete substitution.
pub fn validate_shared_slice_element(
    element: &Ty,
    facts: FactsCtx<'_>,
    budget: &PlanBudget,
) -> Result<(), String> {
    validate_shared_slice_element_inner(element, facts, Some(budget))
}

/// Legacy structural callers have no planning ledger. Validated callers reuse
/// their original ledger, never an equal-limit replacement authority.
pub fn validate_shared_slice_element_with_facts(
    element: &Ty,
    facts: FactsCtx<'_>,
) -> Result<(), String> {
    validate_shared_slice_element_inner(element, facts, facts.validated().map(|f| f.budget()))
}

fn validate_slice_carrier_with_facts(ty: &Ty, facts: FactsCtx<'_>) -> Result<(), String> {
    if let Some((_, element)) = ty.slice_ref_element() {
        validate_shared_slice_element_with_facts(element, facts)
    } else {
        slice_boundary_walk(ty, true, facts.validated().map(|f| f.budget()), 0, &mut 0)
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
    /// Raw structural metadata validation only. No normalization or executable
    /// Sized, carrier, reference, or Copy permission follows from success.
    pub fn validate_ty_metadata(self, root: &Ty) -> Result<(), String> {
        if let Self::Validated(facts) = self {
            return facts
                .validate_ty_metadata(root)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()));
        }
        self.validate_ty_metadata_inner(root, None).map(|_| ())
    }
    pub fn validate_ty_metadata_with_budget(
        self,
        root: &Ty,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if let Self::Validated(facts) = self {
            return facts
                .validate_ty_metadata_with_budget(root, budget)
                .map_err(|e| e.into_legacy_diagnostic(budget));
        }
        self.validate_ty_metadata_inner(root, Some(budget))
            .map(|_| ())
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
    /// Owned snapshots need Sized independently of legal place/referent types.
    /// Aggregate formation already checks nested parameter positions, but a
    /// root trait Self parameter may be a legal unsized place, never a value.
    fn validate_value_ty(self, root: &Ty) -> Result<(), String> {
        self.validate_ty(root)?;
        if let (Self::Validated(facts), Ty::Param(_)) = (self, root) {
            facts
                .require_sized(root)
                .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
        }
        Ok(())
    }
    fn validate_value_ty_with_budget(self, root: &Ty, budget: &PlanBudget) -> Result<(), String> {
        self.validate_ty_with_budget(root, budget)?;
        if let (Self::Validated(facts), Ty::Param(_)) = (self, root) {
            facts
                .require_sized(root)
                .map_err(|e| e.into_legacy_diagnostic(budget))?;
        }
        Ok(())
    }
    fn validate_ty_inner(self, root: &Ty, budget: Option<&PlanBudget>) -> Result<(), String> {
        if self.validate_ty_metadata_inner(root, budget)? {
            return Err("unsupported: associated type projection requires authoritative world normalization".into());
        }
        Ok(())
    }
    fn validate_ty_metadata_inner(
        self,
        root: &Ty,
        budget: Option<&PlanBudget>,
    ) -> Result<bool, String> {
        type_metadata_walk(root, budget, Some(self))
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub call_evidence: Arc<[CallEvidence]>,
    /// Minted only by instantiate_checked from the exact verified source.
    pub body_substitution: Option<Arc<BodySubstitutionProof>>,
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
        let layers = root_reference_layers(ty)?;
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
        return Err(format!(
            "signature-origin parameter limit {MAX_LOAN_SITES} exceeded"
        ));
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
/// The legacy structural helper below retains its prior restricted domain.
pub fn root_reference_layers(ty: &Ty) -> Result<usize, String> {
    reject_associated_types(ty, None)?;
    let mut current = ty;
    let mut count = 0usize;
    while let Ty::Ref { referent, .. } = current {
        count = count.checked_add(1).ok_or("reference depth overflow")?;
        if count > 128 {
            return Err("reference depth resource limit exceeded".into());
        }
        current = referent;
    }
    Ok(count)
}

/// Legacy unsealed structural fixture policy, not generic carrier admission.
pub fn reference_layers(ty: &Ty) -> Result<usize, String> {
    reject_associated_types(ty, None)?;
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
    let layers = root_reference_layers(ty)?;
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
        || contract.input_regions.len() > MAX_LOAN_SITES
        || contract.results.len() > MAX_PATTERN_PROJECTION_DEPTH
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
        budget.check_depth(usize::from(row.input.position.depth) + 1)?;
        if usize::from(row.input.position.depth) >= MAX_PATTERN_PROJECTION_DEPTH {
            return Err("reference depth resource limit exceeded".into());
        }
        if previous.is_some_and(|old| old >= row.input) {
            return Err("noncanonical input reference position order".into());
        }
        previous = Some(row.input);
    }
    let mut previous = None;
    for row in contract.results.iter() {
        budget.check_depth(usize::from(row.position.depth) + 1)?;
        if usize::from(row.position.depth) >= MAX_PATTERN_PROJECTION_DEPTH {
            return Err("reference depth resource limit exceeded".into());
        }
        if previous.is_some_and(|old| old >= row.position) {
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
                    budget.check_depth(usize::from(path.input.position.depth) + 1)?;
                    if usize::from(path.input.position.depth) >= MAX_PATTERN_PROJECTION_DEPTH {
                        return Err("reference depth resource limit exceeded".into());
                    }
                    if path.projection.iter().any(|step| {
                        !matches!(step, Projection::Field(_) | Projection::ConstantIndex(_))
                    }) {
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
                for path in paths.iter() {
                    budget.check_depth(usize::from(path.input.position.depth) + 1)?;
                    if usize::from(path.input.position.depth) >= MAX_PATTERN_PROJECTION_DEPTH {
                        return Err("reference depth resource limit exceeded".into());
                    }
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
        if ty.is_nominal_back_edge() {
            return Ok((false, 0));
        }
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
        // Trait objects erase their concrete type and slices have a runtime
        // element domain. Neither can be enumerated as fixed input paths.
        // Raw pointees (Box, Vec storage) are likewise reachable only through
        // the pointer, never as a field path of the input.
        // Their original lifetime/kind-qualified input envelope remains the
        // authority for returned references, checked against actual body loans.
        let mut opaque = matches!(
            ty,
            Ty::Param(_) | Ty::Dyn(_) | Ty::Slice { .. } | Ty::RawPtr { .. }
        );
        let mut height = 0;
        let mut child = |value: &Ty| -> Result<(), String> {
            let (is_opaque, child_height) = walk(value, false, budget, depth + 1, visits, memo)?;
            opaque |= is_opaque;
            height = height.max(child_height + 1);
            Ok(())
        };
        match ty {
            Ty::Closure(closure) => {
                for ty in closure.captures() {
                    child(ty)?;
                }
            }
            Ty::AssociatedType(_) => {
                return Err(
                    "unsupported: residual associated type projection in reference contract".into(),
                );
            }
            Ty::Ref { referent, .. } => {
                root_reference_layers(ty)?;
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
            Ty::Slice { element }
            | Ty::RawPtr {
                pointee: element, ..
            } => child(element)?,
            Ty::Str => {}
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
            Ty::Param(_)
            | Ty::Int(_)
            | Ty::Float(_)
            | Ty::Char
            | Ty::Bool
            | Ty::Unit
            | Ty::Dyn(_)
            | Ty::Never => {}
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
            (Ty::Closure(a), Ty::Closure(b)) => a.try_eq(b, &mut child)?,
            (Ty::AssociatedType(a), Ty::AssociatedType(b)) => {
                if a.trait_ref.args.len() > 32 || b.trait_ref.args.len() > 32 {
                    return Err("trait argument limit exceeded".into());
                }
                let mut equal = a.item == b.item
                    && a.trait_ref.declaration == b.trait_ref.declaration
                    && a.trait_ref.args.len() == b.trait_ref.args.len();
                equal &= child(&a.trait_ref.self_ty, &b.trait_ref.self_ty)?;
                for (a, b) in a.trait_ref.args.iter().zip(b.trait_ref.args.iter()) {
                    equal &= child(a, b)?;
                }
                equal
            }
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
            (Ty::Slice { element: a }, Ty::Slice { element: b }) => child(a, b)?,
            (Ty::Dyn(a), Ty::Dyn(b)) => a.principal == b.principal,
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
            (
                Ty::RawPtr {
                    kind: a,
                    pointee: x,
                },
                Ty::RawPtr {
                    kind: b,
                    pointee: y,
                },
            ) => a == b && child(x, y)?,
            (Ty::Struct(a), Ty::Struct(b)) => a.def().id == b.def().id,
            (Ty::Enum(a), Ty::Enum(b)) => a.def().id == b.def().id,
            (Ty::Param(a), Ty::Param(b)) => a == b,
            (Ty::Int(a), Ty::Int(b)) => a == b,
            (Ty::Float(a), Ty::Float(b)) => a == b,
            (Ty::Char, Ty::Char)
            | (Ty::Str, Ty::Str)
            | (Ty::Bool, Ty::Bool)
            | (Ty::Unit, Ty::Unit)
            | (Ty::Never, Ty::Never) => true,
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
    if erased && root_reference_layers(result)? == 0 {
        return Ok(());
    }
    let mut cursor = 0;
    for (param, ty) in params.iter().enumerate() {
        for depth in 0..root_reference_layers(ty)? {
            budget.charge_work(1)?;
            let input = InputReference {
                param,
                position: ReferencePosition { depth: depth as u8 },
            };
            let row = contract.input_regions.get(cursor).ok_or(
                "invalid noescape reference contract: missing reference parameter region position",
            )?;
            if row.input != input {
                return Err(format!(
                    "invalid noescape reference contract: missing reference parameter region at expected position {input:?}, found {:?}",
                    row.input
                ));
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
    let layers = root_reference_layers(result)?;
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

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternBinding {
    pub local: LocalId,
    pub projection: Vec<Projection>,
    /// `None` binds the selected place by value (Copy or move). `Some(kind)`
    /// is a `ref`/`ref mut` or default-binding-mode borrow of that place; the
    /// local then has type `&kind T` and lowering issues a genuine loan.
    pub borrow: Option<BorrowKind>,
}
impl PatternBinding {
    /// The binding local's type for a selected place of type `selected`.
    pub fn local_ty(&self, selected: &Ty) -> Ty {
        match self.borrow {
            None => selected.clone(),
            Some(kind) => Ty::Ref {
                kind,
                referent: Arc::new(selected.clone()),
            },
        }
    }
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
            (Ty::Closure(a), Ty::Closure(b)) => {
                if !a.try_eq(b, |a, b| {
                    pending.push((a, b, depth + 1));
                    Ok::<bool, String>(true)
                })? {
                    return Ok(false);
                }
            }
            (Ty::AssociatedType(_), _) | (_, Ty::AssociatedType(_)) => {
                return Err(
                    "unsupported: residual associated type projection in executable pattern".into(),
                );
            }
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
            (
                Ty::RawPtr {
                    kind: ak,
                    pointee: a,
                },
                Ty::RawPtr {
                    kind: bk,
                    pointee: b,
                },
            ) if ak == bk => {
                if !Arc::ptr_eq(a, b) {
                    pending.push((a, b, depth + 1));
                }
            }
            (Ty::Int(a), Ty::Int(b)) if a == b => {}
            (Ty::Float(a), Ty::Float(b)) if a == b => {}
            (Ty::Param(a), Ty::Param(b)) if a == b => {}
            (Ty::Slice { element: a }, Ty::Slice { element: b }) => {
                if !Arc::ptr_eq(a, b) {
                    pending.push((a, b, depth + 1));
                }
            }
            (Ty::Dyn(a), Ty::Dyn(b)) if a.principal == b.principal => {}
            (Ty::Char, Ty::Char)
            | (Ty::Str, Ty::Str)
            | (Ty::Bool, Ty::Bool)
            | (Ty::Unit, Ty::Unit)
            | (Ty::Never, Ty::Never) => {}
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    LetElse {
        value: ExprId,
        ty: Ty,
        pattern: MatchPattern,
        else_branch: ExprId,
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
            matches!(target, Ty::Int(_) | Ty::Float(_)) && target == value_ty
        }
        BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
            matches!(target, Ty::Int(_) | Ty::Bool) && target == value_ty
        }
        BinaryOp::Shl | BinaryOp::Shr => {
            if let (Ty::Int(integer), ExprKind::Int(count), Completion::Returns) =
                (target, &value.kind, value.completion)
                && count.bits >= u128::from(integer.bits())
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

/// A pure observation in an ordered conjunctive pattern decision. Every
/// downcast in this path requires an earlier matching tag observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternTest {
    pub projection: Vec<Projection>,
    pub kind: PatternTestKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternTestKind {
    Bool(bool),
    Int(IntValue),
    Char(CharValue),
    /// Inclusive `lo..=hi` over one integer type, `lo <= hi` in its signedness.
    IntRange {
        lo: IntValue,
        hi: IntValue,
    },
    /// Inclusive `lo..=hi` over Unicode scalars, `lo <= hi`.
    CharRange {
        lo: CharValue,
        hi: CharValue,
    },
    EnumVariant {
        ty: EnumTy,
        variant: usize,
    },
}

impl PatternTestKind {
    /// Exact inclusive scalar interval in order-preserving ordinal space, or
    /// None for non-scalar tests. Validates every scalar endpoint.
    pub fn scalar_interval(&self) -> Result<Option<(u128, u128)>, String> {
        let interval = match self {
            Self::Int(value) => {
                value.validate()?;
                (value.ordinal(), value.ordinal())
            }
            Self::Char(value) => {
                value.validate()?;
                (u128::from(value.value), u128::from(value.value))
            }
            Self::IntRange { lo, hi } => {
                lo.validate()?;
                hi.validate()?;
                if lo.ty != hi.ty || lo.ordinal() > hi.ordinal() {
                    return Err("integer range pattern bounds are mistyped or reversed".into());
                }
                (lo.ordinal(), hi.ordinal())
            }
            Self::CharRange { lo, hi } => {
                lo.validate()?;
                hi.validate()?;
                if lo.value > hi.value {
                    return Err("character range pattern bounds are reversed".into());
                }
                (u128::from(lo.value), u128::from(hi.value))
            }
            Self::Bool(_) | Self::EnumVariant { .. } => return Ok(None),
        };
        Ok(Some(interval))
    }
    /// Scalar domain type tested by this kind, when scalar.
    pub fn scalar_ty(&self) -> Option<Ty> {
        match self {
            Self::Int(value) | Self::IntRange { lo: value, .. } => Some(Ty::Int(value.ty)),
            Self::Char(_) | Self::CharRange { .. } => Some(Ty::Char),
            Self::Bool(_) | Self::EnumVariant { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchPattern {
    /// Ordered source root alternatives. Each row is a binder-free conjunction.
    Alternatives {
        ty: Ty,
        rows: Vec<Vec<PatternTest>>,
    },
    Wildcard,
    Binding(LocalId),
    Bool(bool),
    Int(IntValue),
    Unit,
    /// Irrefutable aggregate selection with no tag test or whole-value move.
    /// The explicit domain also types projections when the scrutinee is Never.
    Aggregate {
        ty: Ty,
        bindings: Vec<PatternBinding>,
    },
    EnumVariant {
        ty: EnumTy,
        variant: usize,
        bindings: Vec<PatternBinding>,
    },
    /// Tests inspect the source in order. Bindings may be initialized only
    /// after every test succeeds, never on a partially successful row.
    Decision {
        ty: Ty,
        tests: Vec<PatternTest>,
        bindings: Vec<PatternBinding>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    /// Bool guard evaluated after bindings; false falls through to later arms.
    pub guard: Option<ExprId>,
    pub body: ExprId,
}

/// The deliberately closed Copy domain of original root-arm alternatives.
/// Builtin-Copy shapes recognizable without trait facts (scalars, shared
/// references, and tuples/arrays of them). Conservative: false means unknown.
fn structurally_copy(ty: &Ty, depth: usize) -> bool {
    if depth > MAX_PATTERN_PROJECTION_DEPTH {
        return false;
    }
    match ty {
        Ty::Int(_) | Ty::Bool | Ty::Char | Ty::Float(_) | Ty::Unit => true,
        Ty::Ref {
            kind: BorrowKind::Shared,
            ..
        } => true,
        Ty::Tuple(fields) => fields.iter().all(|f| structurally_copy(f, depth + 1)),
        Ty::Array { element, .. } => structurally_copy(element, depth + 1),
        _ => false,
    }
}

pub fn validate_alternative_domain(ty: &Ty, budget: Option<&PlanBudget>) -> Result<(), String> {
    if budget.is_some_and(PlanBudget::has_resource_failure) {
        return Err("planning resource budget already exhausted".into());
    }
    fn walk(
        ty: &Ty,
        budget: Option<&PlanBudget>,
        depth: usize,
        work: &mut usize,
    ) -> Result<(), String> {
        if depth > MAX_PATTERN_PROJECTION_DEPTH {
            return Err("or-pattern domain depth resource limit exceeded".into());
        }
        *work = work
            .checked_add(1)
            .ok_or("or-pattern domain work overflow")?;
        if *work > MAX_PATTERN_PROJECTION_WORK {
            return Err("or-pattern domain work resource limit exceeded".into());
        }
        if let Some(budget) = budget {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
        }
        match ty {
            Ty::Bool | Ty::Int(_) | Ty::Char | Ty::Unit => Ok(()),
            Ty::Tuple(fields) => {
                for field in fields.iter() {
                    walk(field, budget, depth + 1, work)?;
                }
                Ok(())
            }
            _ => Err("unsupported: or-patterns require a scalar or scalar tuple domain".into()),
        }
    }
    walk(ty, budget, 0, &mut 0)
}

fn validate_alternative_rows(
    ty: &Ty,
    rows: &[Vec<PatternTest>],
    budget: Option<&PlanBudget>,
) -> Result<(), String> {
    if budget.is_some_and(PlanBudget::has_resource_failure) {
        return Err("planning resource budget already exhausted".into());
    }
    fn charge(work: &mut usize, amount: usize, budget: Option<&PlanBudget>) -> Result<(), String> {
        CheckedFunction::decision_work(work, amount)?;
        if let Some(budget) = budget {
            budget.charge_work(amount)?;
        }
        Ok(())
    }
    validate_alternative_domain(ty, budget)?;
    if rows.len() < 2 {
        return Err("invalid or-pattern alternative count".into());
    }
    let mut work = 0;
    charge(&mut work, rows.len(), budget)?;
    for row in rows {
        if row.len() > MAX_PATTERN_BINDINGS {
            return Err("or-pattern row test limit exceeded".into());
        }
        charge(&mut work, row.len(), budget)?;
        for (index, test) in row.iter().enumerate() {
            charge(
                &mut work,
                test.projection
                    .len()
                    .checked_add(1)
                    .ok_or("or-pattern projection work overflow")?,
                budget,
            )?;
            if !matches!(
                test.kind,
                PatternTestKind::Bool(_) | PatternTestKind::Int(_) | PatternTestKind::Char(_)
            ) || test
                .projection
                .iter()
                .any(|p| !matches!(p, Projection::Field(_)))
            {
                return Err("invalid scalar tuple or-pattern row".into());
            }
            for previous in &row[..index] {
                let amount = previous
                    .projection
                    .len()
                    .checked_add(test.projection.len())
                    .and_then(|n| n.checked_add(1))
                    .ok_or("or-pattern row work overflow")?;
                charge(&mut work, amount, budget)?;
                if previous.projection == test.projection {
                    return Err("or-pattern repeats a tested source path".into());
                }
            }
            let actual = projected_ty(ty, &test.projection)?;
            let valid = match &test.kind {
                PatternTestKind::Bool(_) => *actual == Ty::Bool,
                PatternTestKind::Int(_) | PatternTestKind::Char(_) => {
                    test.kind.scalar_interval()?;
                    test.kind.scalar_ty().as_ref() == Some(actual)
                }
                _ => false,
            };
            if !valid {
                return Err("or-pattern test type mismatch".into());
            }
        }
    }
    Ok(())
}

/// Cold diagnostic payload for an abort-only, nonreturning panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanicMessage {
    pub message: String,
    pub location: String,
}

/// A fixed standard output stream, never an arbitrary file descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
    /// Internal panic diagnostic output. Write failures must not return or
    /// replace the panic-abort terminator with an unrelated output trap.
    PanicStderr,
}

/// Native formatting operations, not name-based std::fmt trait dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatMode {
    Display,
    Debug,
}

/// One immutable admissibility contract shared by source, typed IR and CFG.
/// Debug aggregates are structural and Copy: no nominal/user implementation
/// or ownership permission is invented by selecting a format mode.
pub fn validate_format_ty(
    mode: FormatMode,
    ty: &Ty,
    budget: Option<&PlanBudget>,
) -> Result<(), String> {
    fn walk(
        mode: FormatMode,
        ty: &Ty,
        budget: Option<&PlanBudget>,
        depth: usize,
    ) -> Result<(), String> {
        if let Some(budget) = budget {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
        }
        if depth > 128 {
            return Err("format type nesting limit exceeded".into());
        }
        match (mode, ty) {
            (_, Ty::Int(_) | Ty::Bool | Ty::Never) => Ok(()),
            (FormatMode::Display, Ty::Char) => Ok(()),
            (FormatMode::Display, ty) if ty.is_shared_str() => Ok(()),
            (FormatMode::Debug, Ty::Unit) => Ok(()),
            (FormatMode::Debug, Ty::Tuple(fields)) => {
                for field in fields.iter() { walk(mode, field, budget, depth + 1)?; }
                Ok(())
            }
            (FormatMode::Debug, Ty::Array { element, len }) if *len <= MAX_ARRAY_LEN => walk(mode, element, budget, depth + 1),
            (FormatMode::Display, _) => Err("format argument requires integer, bool, char or &str Display".into()),
            (FormatMode::Debug, _) => Err("unsupported: native Debug requires bool, integer, unit, or recursive tuple/fixed array of those types".into()),
        }
    }
    walk(mode, ty, budget, 0)
}

/// Decoded literal bytes and their exact output operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiteralOutput {
    pub text: String,
    pub stream: OutputStream,
    pub newline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    /// Linux syscall number followed by its six usize ABI arguments.
    NativeSyscall(Vec<ExprId>),
    Char(CharValue),
    /// Inherent primitive method. `args[0]` is the by-value receiver; all
    /// operand and result types are exactly `intrinsic.signature()`.
    Intrinsic {
        intrinsic: Intrinsic,
        args: Vec<ExprId>,
    },
    /// A string literal: `&'static str` over immutable program data.
    Str(String),
    /// Count zero bits before the highest set bit within the integer's declared width.
    LeadingZeros {
        value: ExprId,
    },
    CharToInt {
        value: ExprId,
        to: IntTy,
    },
    U8ToChar {
        value: ExprId,
    },
    /// Pointer-representation `as` cast: reference to raw pointer, raw pointer
    /// to raw pointer, raw pointer to usize, or usize to raw pointer. The
    /// result type is the expression type; `ptr_cast_valid` is authoritative.
    PtrCast {
        value: ExprId,
    },
    IntCast {
        value: ExprId,
        to: IntTy,
    },
    /// `as` between numeric primitives where at least one endpoint is a float.
    FloatCast {
        value: ExprId,
        to: NumTy,
    },
    Float(FloatValue),
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
    /// Unsizes shared `&[T; N]` to shared `&[T]`. N comes from the
    /// validated operand type. The operand is evaluated once and its loan is the
    /// only storage authority; this node appends length metadata only.
    ArrayToSlice {
        reference: ExprId,
    },
    /// Unsizes `&T`/`&mut T` (sized concrete or rigid generic `T: Trait`) to a
    /// trait object reference of the same kind. The vtable is chosen from the
    /// operand's exact (substituted) referent type, never stored here.
    RefToDyn {
        reference: ExprId,
    },
    /// Builtin length of an admitted shared slice, result `usize`.
    SliceLen {
        slice: ExprId,
    },
    /// Runtime bounds-checked Copy read of the exact shared slice element at a
    /// `usize` index. There is no slice place or mutable slice index.
    SliceIndex {
        slice: ExprId,
        index: ExprId,
    },
    /// Shared subslice `&s[start..end]` (`..=` when `inclusive`) of a shared
    /// `&[T]` (SUBSLICE.md). Absent bounds mean 0 / len. Evaluation order is
    /// slice, start, end; then stock checks: end overflow (inclusive), end > len,
    /// start > end, each a runtime trap. Result shares the source loan.
    Subslice {
        slice: ExprId,
        start: Option<ExprId>,
        end: Option<ExprId>,
        inclusive: bool,
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
    /// A single-evaluation block with a value-producing labeled break target.
    LabeledBlock {
        id: LoopId,
        body: Block,
    },
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
    Output(LiteralOutput),
    Println(String),
    Format {
        pieces: Vec<String>,
        arguments: Vec<ExprId>,
        modes: Vec<FormatMode>,
        stream: OutputStream,
        newline: bool,
    },
    Panic(Box<PanicMessage>),
}

impl CheckedFunction {
    /// Source while-let statements are erased to Unit-typed loops. Their
    /// source completion and final-semicolon obligation cannot be recovered.
    /// Keep that uncertainty explicit, without expanding expression graphs.
    fn conditional_block_flow<'a>(
        &'a self,
        block: &Block,
        budget: &PlanBudget,
    ) -> Result<([Option<&'a Ty>; 2], bool, bool), String> {
        if budget.has_resource_failure() {
            return Err("planning resource budget already exhausted".into());
        }
        let mut definitely_completes = true;
        let mut possibly_completes = true;
        let mut erased_obligation = false;
        let expression = |id: ExprId| -> Result<&TypedExpr, String> {
            budget.charge_work(1)?;
            self.exprs
                .get(id.0)
                .ok_or_else(|| format!("invalid conditional expression {}", id.0))
        };
        for statement in &block.statements {
            budget.charge_work(1)?;
            let mut ambiguous = false;
            let returns = match statement {
                Stmt::Declare { .. } => true,
                Stmt::Return(value) => {
                    if let Some(value) = value {
                        expression(*value)?;
                    }
                    false
                }
                Stmt::AssignIndex { index, value, .. }
                | Stmt::AssignIndexOp { index, value, .. } => {
                    let value = expression(*value)?.completion == Completion::Returns;
                    let index = expression(*index)?.completion == Completion::Returns;
                    value && index
                }
                Stmt::Expr(id) => {
                    let value = expression(*id)?;
                    // This is a possible erased source obligation, not proof
                    // that the expression originated as a while-let.
                    ambiguous = value.ty == Ty::Unit && matches!(value.kind, ExprKind::Loop { .. });
                    erased_obligation |= ambiguous;
                    value.completion == Completion::Returns
                }
                Stmt::Let { value, .. }
                | Stmt::LetPattern { value, .. }
                // The else arm is separately required to diverge by
                // validate_let_else. Only initializer completion propagates.
                | Stmt::LetElse { value, .. }
                | Stmt::Assign { value, .. }
                | Stmt::AssignPlace { value, .. }
                | Stmt::AssignOp { value, .. } => {
                    expression(*value)?.completion == Completion::Returns
                }
                // A while body need not execute. Its condition must complete.
                Stmt::While { condition, .. } => {
                    expression(*condition)?.completion == Completion::Returns
                }
            };
            definitely_completes &= returns;
            possibly_completes &= returns || ambiguous;
        }
        let types = if let Some(tail) = block.tail {
            let tail = expression(tail)?;
            definitely_completes &= tail.completion == Completion::Returns;
            possibly_completes &= tail.completion == Completion::Returns;
            [Some(&tail.ty), None]
        } else if definitely_completes {
            [Some(&Ty::Unit), None]
        } else if erased_obligation {
            [Some(&Ty::Unit), Some(&Ty::Never)]
        } else {
            [Some(&Ty::Never), None]
        };
        Ok((types, definitely_completes, possibly_completes))
    }

    /// Compare the frontend's Never/array/tuple join with retained result
    /// metadata, without constructing or recursively expanding a joined type.
    fn conditional_join_matches(
        left: &Ty,
        right: &Ty,
        result: &Ty,
        budget: &PlanBudget,
    ) -> Result<bool, String> {
        struct Scratch<T> {
            values: Vec<T>,
            _base: Option<PlanReservation>,
            _extra: Option<PlanReservation>,
        }
        impl<T> Scratch<T> {
            fn new() -> Self {
                Self {
                    values: Vec::new(),
                    _base: None,
                    _extra: None,
                }
            }
            fn push(&mut self, value: T, budget: &PlanBudget) -> Result<(), String> {
                if self.values.len() == self.values.capacity() {
                    let target = self
                        .values
                        .capacity()
                        .checked_mul(2)
                        .ok_or("conditional scratch capacity overflow")?
                        .max(4);
                    let bytes = target
                        .checked_mul(std::mem::size_of::<T>())
                        .ok_or("conditional scratch byte overflow")?;
                    let mut staged = Self::new();
                    staged._base = Some(budget.reserve_bytes(bytes)?);
                    budget.charge_work(self.values.len())?;
                    staged
                        .values
                        .try_reserve_exact(target)
                        .map_err(|_| "conditional scratch allocation failed")?;
                    let excess = staged
                        .values
                        .capacity()
                        .checked_sub(target)
                        .and_then(|n| n.checked_mul(std::mem::size_of::<T>()))
                        .ok_or("conditional scratch capacity overflow")?;
                    staged._extra = Some(budget.reserve_bytes(excess)?);
                    staged.values.append(&mut self.values);
                    *self = staged;
                }
                self.values.push(value);
                Ok(())
            }
        }
        if budget.has_resource_failure() {
            return Err("planning resource budget already exhausted".into());
        }
        let mut pending = Scratch::new();
        let mut seen = Scratch::new();
        pending.push((left, right, result, 0usize), budget)?;
        while let Some((left, right, result, depth)) = pending.values.pop() {
            budget.charge_work(1)?;
            budget.check_depth(depth)?;
            let key = (
                std::ptr::from_ref(left) as usize,
                std::ptr::from_ref(right) as usize,
                std::ptr::from_ref(result) as usize,
            );
            let mut found = false;
            for candidate in &seen.values {
                budget.charge_work(1)?;
                if candidate == &key {
                    found = true;
                    break;
                }
            }
            if found {
                continue;
            }
            seen.push(key, budget)?;
            if matches!(left, Ty::Never) {
                if !ref_types_equal(right, result, budget, depth)? {
                    return Ok(false);
                }
                continue;
            }
            if matches!(right, Ty::Never) {
                if !ref_types_equal(left, result, budget, depth)? {
                    return Ok(false);
                }
                continue;
            }
            match (left, right, result) {
                (
                    Ty::Array { element: a, len: x },
                    Ty::Array { element: b, len: y },
                    Ty::Array { element: c, len: z },
                ) if x == y && y == z => {
                    pending.push((a.as_ref(), b.as_ref(), c.as_ref(), depth + 1), budget)?;
                }
                (Ty::Tuple(a), Ty::Tuple(b), Ty::Tuple(c))
                    if a.len() == b.len() && b.len() == c.len() =>
                {
                    budget.charge_work(a.len())?;
                    for ((a, b), c) in a.iter().zip(b.iter()).zip(c.iter()) {
                        pending.push((a, b, c, depth + 1), budget)?;
                    }
                }
                _ => {
                    if !ref_types_equal(left, result, budget, depth)?
                        || !ref_types_equal(right, result, budget, depth)?
                    {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// Validate all retained type metadata before any executable admission check.
    /// Arena rows and statement annotations are authoritative even when dead.
    fn validate_all_type_metadata(
        &self,
        facts: FactsCtx<'_>,
        budget: Option<&PlanBudget>,
    ) -> Result<(), String> {
        fn rows(count: usize, budget: Option<&PlanBudget>, work: &mut usize) -> Result<(), String> {
            *work = work.checked_add(count).ok_or("metadata work overflow")?;
            if *work > MAX_PATTERN_PROJECTION_WORK {
                return Err("metadata work resource limit exceeded".into());
            }
            if let Some(budget) = budget {
                budget.charge_work(count)?;
            }
            Ok(())
        }
        fn pattern(
            pattern: &MatchPattern,
            visit: &mut impl FnMut(&Ty) -> Result<(), String>,
            budget: Option<&PlanBudget>,
            work: &mut usize,
        ) -> Result<(), String> {
            rows(1, budget, work)?;
            match pattern {
                MatchPattern::Alternatives {
                    ty,
                    rows: alternatives,
                } => {
                    visit(ty)?;
                    validate_alternative_rows(ty, alternatives, budget)?;
                    rows(alternatives.len(), budget, work)?;
                }
                MatchPattern::Aggregate { ty, .. } => visit(ty)?,
                MatchPattern::EnumVariant { ty, .. } => visit(&Ty::Enum(ty.clone()))?,
                MatchPattern::Decision { ty, tests, .. } => {
                    rows(tests.len(), budget, work)?;
                    visit(ty)?;
                    for test in tests {
                        if let PatternTestKind::EnumVariant { ty, .. } = &test.kind {
                            visit(&Ty::Enum(ty.clone()))?;
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
        fn block(
            value: &Block,
            visit: &mut impl FnMut(&Ty) -> Result<(), String>,
            budget: Option<&PlanBudget>,
            depth: usize,
            work: &mut usize,
        ) -> Result<(), String> {
            if depth > MAX_PATTERN_PROJECTION_DEPTH {
                return Err("metadata block depth resource limit exceeded".into());
            }
            if let Some(budget) = budget {
                budget.check_depth(depth)?;
            }
            for statement in &value.statements {
                *work = work.checked_add(1).ok_or("metadata work overflow")?;
                if *work > MAX_PATTERN_PROJECTION_WORK {
                    return Err("metadata work resource limit exceeded".into());
                }
                if let Some(budget) = budget {
                    budget.charge_work(1)?;
                }
                match statement {
                    Stmt::LetPattern { ty, .. } => visit(ty)?,
                    Stmt::LetElse {
                        ty, pattern: row, ..
                    } => {
                        visit(ty)?;
                        pattern(row, visit, budget, work)?;
                    }
                    Stmt::While { body, .. } => block(body, visit, budget, depth + 1, work)?,
                    _ => {}
                }
            }
            Ok(())
        }
        let budget = budget.or_else(|| facts.validated().map(|f| f.budget()));
        if budget.is_some_and(PlanBudget::has_resource_failure) {
            return Err("planning resource budget already exhausted".into());
        }
        let mut residual = false;
        let mut visits = 0usize;
        let mut visit = |ty: &Ty| -> Result<(), String> {
            visits = visits.checked_add(1).ok_or("metadata work overflow")?;
            if visits > MAX_PATTERN_PROJECTION_WORK {
                return Err("metadata work resource limit exceeded".into());
            }
            if let Some(authority) = facts.validated() {
                let ledger = budget.unwrap_or(authority.budget());
                authority
                    .validate_ty_metadata_with_budget(ty, ledger)
                    .map_err(|e| e.into_legacy_diagnostic(ledger))?;
                residual |= type_metadata_walk(ty, Some(ledger), None)?;
            } else {
                residual |= facts.validate_ty_metadata_inner(ty, budget)?;
            }
            Ok(())
        };
        visit(&self.result)?;
        for local in &self.locals {
            visit(&local.ty)?;
        }
        let mut work = 0;
        for expression in &self.exprs {
            visit(&expression.ty)?;
            match &expression.kind {
                ExprKind::Struct { ty, .. } => visit(&Ty::Struct(ty.clone()))?,
                ExprKind::EnumVariant { ty, .. } => visit(&Ty::Enum(ty.clone()))?,
                ExprKind::Call {
                    target, signature, ..
                } => {
                    if let Some(authority) = facts.validated() {
                        authority
                            .validate_callee_metadata(target)
                            .map_err(|error| {
                                error.into_legacy_diagnostic(budget.unwrap_or(authority.budget()))
                            })?;
                    }
                    for ty in target
                        .metadata_types()
                        .chain(signature.params.iter())
                        .chain(std::iter::once(&signature.result))
                    {
                        visit(ty)?;
                    }
                }
                ExprKind::Match { arms, .. } => {
                    for arm in arms {
                        pattern(&arm.pattern, &mut visit, budget, &mut work)?;
                    }
                }
                ExprKind::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    block(then_block, &mut visit, budget, 0, &mut work)?;
                    block(else_block, &mut visit, budget, 0, &mut work)?;
                }
                ExprKind::Block(value)
                | ExprKind::Loop { body: value, .. }
                | ExprKind::LabeledBlock { body: value, .. } => {
                    block(value, &mut visit, budget, 0, &mut work)?
                }
                _ => {}
            }
        }
        block(&self.body, &mut visit, budget, 0, &mut work)?;
        if residual {
            return Err(
                "unsupported: residual associated type projection in executable metadata".into(),
            );
        }
        Ok(())
    }
    pub fn preflight_plan(&self, facts: FactsCtx<'_>, budget: &PlanBudget) -> Result<(), String> {
        self.validate_all_type_metadata(facts, Some(budget))?;
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
        fn pattern_storage(
            pattern: &MatchPattern,
            facts: FactsCtx<'_>,
            budget: &PlanBudget,
        ) -> Result<(), String> {
            match pattern {
                MatchPattern::Alternatives { ty, rows } => {
                    validate_alternative_domain(ty, Some(budget))?;
                    facts.validate_value_ty_with_budget(ty, budget)?;
                    storage(
                        budget,
                        rows.capacity(),
                        std::mem::size_of::<Vec<PatternTest>>(),
                    )?;
                    for row in rows {
                        storage(budget, row.capacity(), std::mem::size_of::<PatternTest>())?;
                        for test in row {
                            budget.check_depth(test.projection.len())?;
                            storage(
                                budget,
                                test.projection.capacity(),
                                std::mem::size_of::<Projection>(),
                            )?;
                        }
                    }
                }
                MatchPattern::EnumVariant {
                    ty,
                    bindings: values,
                    ..
                } => {
                    facts.validate_ty_with_budget(&Ty::Enum(ty.clone()), budget)?;
                    bindings(values, budget)?;
                }
                MatchPattern::Aggregate {
                    ty,
                    bindings: values,
                } => {
                    facts.validate_value_ty_with_budget(ty, budget)?;
                    bindings(values, budget)?;
                }
                MatchPattern::Decision {
                    ty,
                    tests,
                    bindings: values,
                } => {
                    storage(budget, tests.len(), std::mem::size_of::<PatternTest>())?;
                    facts.validate_value_ty_with_budget(ty, budget)?;
                    for test in tests {
                        path(&test.projection, budget)?;
                        if let PatternTestKind::EnumVariant { ty, .. } = &test.kind {
                            facts.validate_value_ty_with_budget(&Ty::Enum(ty.clone()), budget)?;
                        }
                    }
                    bindings(values, budget)?;
                }
                _ => {}
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
                    Stmt::LetElse { ty, pattern, .. } => {
                        facts.validate_value_ty_with_budget(ty, budget)?;
                        pattern_storage(pattern, facts, budget)?;
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
        facts.validate_value_ty_with_budget(&self.result, budget)?;
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
        storage(
            budget,
            self.call_evidence.len(),
            std::mem::size_of::<CallEvidence>(),
        )?;
        for evidence in self.call_evidence.iter() {
            storage(
                budget,
                evidence.arguments.len(),
                std::mem::size_of::<CallArgumentTrace>(),
            )?;
            storage(
                budget,
                evidence.type_actual_regions.len(),
                std::mem::size_of::<TypeActualRegion>(),
            )?;
            for region in evidence.type_actual_regions.iter() {
                budget.check_depth(region.location.path.0.len())?;
                storage(
                    budget,
                    region.location.path.0.len(),
                    std::mem::size_of::<CarrierStep>(),
                )?;
            }
        }
        for local in &self.locals {
            facts.validate_value_ty_with_budget(&local.ty, budget)?;
            storage(budget, local.name.len(), 1)?;
        }
        for expression in &self.exprs {
            facts.validate_value_ty_with_budget(&expression.ty, budget)?;
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
                        pattern_storage(&arm.pattern, facts, budget)?;
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
                ExprKind::Block(value)
                | ExprKind::Loop { body: value, .. }
                | ExprKind::LabeledBlock { body: value, .. } => block(value, facts, budget, 0)?,
                ExprKind::NativeSyscall(values)
                | ExprKind::Tuple(values)
                | ExprKind::Array(values)
                | ExprKind::Intrinsic { args: values, .. } => {
                    storage(budget, values.len(), std::mem::size_of::<ExprId>())?
                }
                ExprKind::Output(output) => storage(budget, output.text.len(), 1)?,
                ExprKind::Format {
                    pieces,
                    arguments,
                    modes,
                    ..
                } => {
                    storage(budget, pieces.len(), std::mem::size_of::<String>())?;
                    for piece in pieces {
                        storage(budget, piece.len(), 1)?;
                    }
                    storage(budget, arguments.len(), std::mem::size_of::<ExprId>())?;
                    storage(budget, modes.len(), std::mem::size_of::<FormatMode>())?;
                }
                ExprKind::Str(text) => storage(budget, text.len(), 1)?,
                ExprKind::Println(text) => storage(budget, text.len(), 1)?,
                ExprKind::Panic(payload) => {
                    storage(budget, 1, std::mem::size_of::<PanicMessage>())?;
                    storage(budget, payload.message.len(), 1)?;
                    storage(budget, payload.location.len(), 1)?;
                }
                ExprKind::CharToInt { .. }
                | ExprKind::U8ToChar { .. }
                | ExprKind::PtrCast { .. }
                | ExprKind::LeadingZeros { .. }
                | ExprKind::IntCast { .. }
                | ExprKind::FloatCast { .. }
                | ExprKind::Char(_)
                | ExprKind::Float(_)
                | ExprKind::Deref { .. }
                | ExprKind::DynamicIndex { .. }
                | ExprKind::ArrayToSlice { .. }
                | ExprKind::RefToDyn { .. }
                | ExprKind::SliceLen { .. }
                | ExprKind::SliceIndex { .. }
                | ExprKind::Subslice { .. }
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
            if let Some(guard) = arm.guard {
                let guard = self.exprs.get(guard.0).ok_or("invalid match guard")?;
                if !matches!(guard.ty, Ty::Bool | Ty::Never) {
                    return Err("match guard must be bool".into());
                }
            }
            self.validate_pattern(ty, &arm.pattern, parameters, &mut bindings)?;
        }
        Ok(())
    }

    fn validate_pattern(
        &self,
        ty: &Ty,
        pattern: &MatchPattern,
        parameters: &std::collections::HashSet<LocalId>,
        bindings: &mut std::collections::HashSet<LocalId>,
    ) -> Result<(), String> {
        let valid = match pattern {
            MatchPattern::Alternatives { ty: domain, rows } => {
                validate_alternative_rows(domain, rows, None)?;
                let mut work = 0;
                *ty == Ty::Never || pattern_types_equal(ty, domain, &mut work)?
            }
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
            MatchPattern::Aggregate {
                ty: domain,
                bindings: aggregate_bindings,
            } => {
                let aggregate = match domain {
                    Ty::Struct(_) => true,
                    Ty::Tuple(fields) => !fields.is_empty(),
                    Ty::Array { len, .. } => *len > 0 && *len <= MAX_ARRAY_LEN,
                    // A matched reference is selected through: some binding
                    // must dereference it (a binding-free match is a wildcard).
                    Ty::Ref { .. } => aggregate_bindings
                        .iter()
                        .any(|binding| binding.projection.first() == Some(&Projection::Deref)),
                    // A whole-value `ref` binding of a scalar place.
                    _ => {
                        !aggregate_bindings.is_empty()
                            && aggregate_bindings.iter().all(|binding| {
                                binding.borrow.is_some() && binding.projection.is_empty()
                            })
                    }
                };
                if !aggregate {
                    return Err(
                            "aggregate match requires a nominal struct, nonempty tuple or fixed array domain"
                                .into(),
                        );
                }
                self.validate_pattern_bindings_with_params(domain, aggregate_bindings, parameters)?;
                for binding in aggregate_bindings {
                    if (binding.projection.is_empty()
                        && binding.borrow.is_none()
                        && !structurally_copy(domain, 0))
                        || binding.projection.iter().any(|step| {
                            !matches!(
                                step,
                                Projection::Field(_)
                                    | Projection::ConstantIndex(_)
                                    | Projection::Deref
                            )
                        })
                    {
                        return Err(
                                "aggregate match bindings require nonempty typed field or constant-index projections"
                                    .into(),
                            );
                    }
                }
                for binding in aggregate_bindings {
                    if !bindings.insert(binding.local) {
                        return Err("match bindings must be distinct fresh locals".into());
                    }
                }
                let mut work = 0;
                *ty == Ty::Never || pattern_types_equal(ty, domain, &mut work)?
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
                    // `whole @ E::V(..)` binds the enum itself (empty projection).
                    let whole = binding.projection.is_empty();
                    if (!whole && binding.projection.first() != Some(&expected))
                        || !bindings.insert(binding.local)
                    {
                        return Err("invalid or repeated enum payload binding".into());
                    }
                }
                *variant < pattern_ty.def().variants.len()
                    && (*ty == Ty::Never || *ty == Ty::Enum(pattern_ty.clone()))
            }
            MatchPattern::Decision {
                ty: domain,
                tests,
                bindings: decision_bindings,
            } => {
                self.validate_decision(domain, tests, decision_bindings, parameters)?;
                for binding in decision_bindings {
                    if !bindings.insert(binding.local) {
                        return Err("match bindings must be distinct fresh locals".into());
                    }
                }
                let mut work = 0;
                *ty == Ty::Never || pattern_types_equal(ty, domain, &mut work)?
            }
        };
        if !valid {
            return Err("match pattern type or variant mismatch".into());
        }
        Ok(())
    }

    fn decision_work(work: &mut usize, count: usize) -> Result<(), String> {
        *work = work
            .checked_add(count)
            .ok_or("pattern decision work limit exceeded")?;
        if *work > MAX_PATTERN_PROJECTION_WORK {
            return Err("pattern decision work limit exceeded".into());
        }
        Ok(())
    }

    /// Only earlier successful tag tests authorize a downcast. A path's own
    /// nominal metadata cannot grant the runtime refinement it needs to read.
    fn validate_decision_path(
        domain: &Ty,
        path: &[Projection],
        earlier: &[PatternTest],
        work: &mut usize,
    ) -> Result<(), String> {
        if path.len() > MAX_PATTERN_PROJECTION_DEPTH {
            return Err("pattern decision projection depth exceeds limit 128".into());
        }
        Self::decision_work(work, path.len())?;
        for (index, step) in path.iter().enumerate() {
            match step {
                Projection::DynamicIndex(_) => {
                    return Err("runtime index is not a source pattern projection".into());
                }
                Projection::Field(_) | Projection::ConstantIndex(_) | Projection::Deref => {}
                Projection::Downcast(variant) => {
                    let mut authorized = false;
                    for test in earlier {
                        Self::decision_work(work, 1 + test.projection.len() + index)?;
                        if test.projection == path[..index]
                            && let PatternTestKind::EnumVariant {
                                ty,
                                variant: selected,
                            } = &test.kind
                            && ty.def().id == variant.owner
                            && *selected == variant.index
                        {
                            authorized = true;
                            break;
                        }
                    }
                    if !authorized {
                        return Err("pattern decision downcast lacks earlier exact tag test".into());
                    }
                }
            }
        }
        // This independently checks owner, shape and physical bounds at every
        // step, including paths that will never execute at runtime.
        projected_ty(domain, path)?;
        Ok(())
    }

    fn validate_decision(
        &self,
        domain: &Ty,
        tests: &[PatternTest],
        bindings: &[PatternBinding],
        parameters: &std::collections::HashSet<LocalId>,
    ) -> Result<(), String> {
        let admitted = match domain {
            Ty::Bool
            | Ty::Int(_)
            | Ty::Char
            | Ty::Unit
            | Ty::Struct(_)
            | Ty::Enum(_)
            | Ty::Ref { .. } => true,
            Ty::Tuple(fields) => !fields.is_empty(),
            Ty::Array { len, .. } => *len > 0 && *len <= MAX_ARRAY_LEN,
            _ => false,
        };
        if !admitted || tests.is_empty() || tests.len() > MAX_PATTERN_BINDINGS {
            return Err(
                "pattern decision requires a known domain and bounded nonempty tests".into(),
            );
        }
        let mut work = tests.len();
        Self::decision_work(&mut work, bindings.len())?;
        for (index, test) in tests.iter().enumerate() {
            Self::validate_decision_path(domain, &test.projection, &tests[..index], &mut work)?;
            for previous in &tests[..index] {
                Self::decision_work(
                    &mut work,
                    1 + previous.projection.len() + test.projection.len(),
                )?;
                if previous.projection == test.projection {
                    return Err("pattern decision repeats a tested source path".into());
                }
            }
            let actual = projected_ty(domain, &test.projection)?;
            let valid = match &test.kind {
                PatternTestKind::Bool(_) => *actual == Ty::Bool,
                PatternTestKind::Int(_)
                | PatternTestKind::Char(_)
                | PatternTestKind::IntRange { .. }
                | PatternTestKind::CharRange { .. } => {
                    test.kind.scalar_interval()?;
                    test.kind.scalar_ty().as_ref() == Some(actual)
                }
                PatternTestKind::EnumVariant { ty, variant } => {
                    ty.validate()?;
                    *variant < ty.def().variants.len()
                        && pattern_types_equal(actual, &Ty::Enum(ty.clone()), &mut work)?
                }
            };
            if !valid {
                return Err("pattern decision test type or variant mismatch".into());
            }
        }
        self.validate_pattern_bindings_with_work(domain, bindings, parameters, &mut work)?;
        for binding in bindings {
            Self::validate_decision_path(domain, &binding.projection, tests, &mut work)?;
        }
        Ok(())
    }

    pub(super) fn validate_let_else(
        &self,
        value: ExprId,
        ty: &Ty,
        pattern: &MatchPattern,
        else_branch: ExprId,
        parameters: &std::collections::HashSet<LocalId>,
    ) -> Result<(), String> {
        if matches!(pattern, MatchPattern::Alternatives { .. }) {
            return Err("unsupported: or-pattern outside match arm root".into());
        }
        let value = self
            .exprs
            .get(value.0)
            .ok_or("invalid let-else initializer")?;
        let otherwise = self
            .exprs
            .get(else_branch.0)
            .ok_or("invalid let-else branch")?;
        let mut work = 0;
        if value.ty != Ty::Never && !pattern_types_equal(&value.ty, ty, &mut work)? {
            return Err("let-else initializer domain mismatch".into());
        }
        if otherwise.ty != Ty::Never || otherwise.completion != Completion::Never {
            return Err("let-else branch must have never type and completion".into());
        }
        self.validate_pattern(
            ty,
            pattern,
            parameters,
            &mut std::collections::HashSet::new(),
        )
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
        self.validate_pattern_bindings_with_work(ty, bindings, parameters, &mut 0)
    }

    fn validate_pattern_bindings_with_work(
        &self,
        ty: &Ty,
        bindings: &[PatternBinding],
        parameters: &std::collections::HashSet<LocalId>,
        work: &mut usize,
    ) -> Result<(), String> {
        if bindings.len() > MAX_PATTERN_BINDINGS {
            return Err("pattern binding limit exceeded".into());
        }
        Self::decision_work(work, bindings.len())?;
        let mut locals = std::collections::HashSet::new();
        let mut paths = Vec::with_capacity(bindings.len());
        for binding in bindings {
            if binding.projection.len() > MAX_PATTERN_PROJECTION_DEPTH {
                return Err("pattern projection depth exceeds limit 128".into());
            }
            *work = work
                .checked_add(binding.projection.len())
                .ok_or("pattern projection work limit exceeded")?;
            if *work > MAX_PATTERN_PROJECTION_WORK {
                return Err("pattern projection work limit exceeded".into());
            }
            let local = self
                .locals
                .get(binding.local.0)
                .ok_or("invalid pattern binding local")?;
            if !locals.insert(binding.local) || parameters.contains(&binding.local) {
                return Err("pattern binding locals must be distinct fresh locals".into());
            }
            let projected = binding.local_ty(projected_ty(ty, &binding.projection)?);
            if !pattern_types_equal(&local.ty, &projected, work)? {
                return Err("pattern binding local type mismatch".into());
            }
            // `x @ Some(y)`: overlapping shared borrows and Copy reads are
            // sound; only overlapping moves or mutable borrows conflict.
            let exclusive = match binding.borrow {
                Some(BorrowKind::Shared) => false,
                Some(_) => true,
                None => !structurally_copy(projected_ty(ty, &binding.projection)?, 0),
            };
            paths.push((binding.projection.as_slice(), exclusive));
        }
        paths.sort_unstable_by(|a, b| a.0.cmp(b.0));
        for (i, (outer, outer_exclusive)) in paths.iter().enumerate() {
            for (inner, inner_exclusive) in &paths[i + 1..] {
                if !inner.starts_with(outer) {
                    break;
                }
                if *outer_exclusive || *inner_exclusive {
                    return Err("overlapping pattern binding projections".into());
                }
            }
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
        self.validate_with_evidence(facts, CallEvidencePolicy::Legacy)
    }
    fn validate_with_evidence(
        &self,
        facts: FactsCtx<'_>,
        policy: CallEvidencePolicy<'_>,
    ) -> Result<(), String> {
        self.validate_all_type_metadata(facts, facts.validated().map(|f| f.budget()))?;
        if let Some(authority) = facts.validated() {
            if let Some(proof) = &self.body_substitution {
                proof.validate(self, authority, authority.budget())?;
            } else {
                validate_call_evidence_with_policy(self, authority, authority.budget(), policy)?;
            }
        }
        facts.validate_value_ty(&self.result)?;
        validate_slice_carrier_with_facts(&self.result, facts)?;
        for local in &self.locals {
            facts.validate_value_ty(&local.ty)?;
            validate_slice_carrier_with_facts(&local.ty, facts)?;
        }
        for expression in &self.exprs {
            facts.validate_value_ty(&expression.ty)?;
            validate_slice_carrier_with_facts(&expression.ty, facts)?;
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
        let fallback_budget = PlanBudget::default();
        let conditional_budget = facts
            .validated()
            .map_or(&fallback_budget, |authority| authority.budget());
        for (borrow_index, value) in self.exprs.iter().enumerate() {
            match &value.kind {
                ExprKind::CharToInt { value: source, to } => {
                    expr(*source)?;
                    let source = &self.exprs[source.0];
                    if source.ty != Ty::Char {
                        return Err("character cast requires char source".into());
                    }
                    if value.ty != Ty::Int(*to) {
                        return Err("character cast result type mismatch".into());
                    }
                    if value.completion != source.completion {
                        return Err("character cast completion mismatch".into());
                    }
                }
                ExprKind::U8ToChar { value: source } => {
                    expr(*source)?;
                    let source = &self.exprs[source.0];
                    if source.ty != Ty::Int(IntTy::U8) {
                        return Err("character cast requires u8 source".into());
                    }
                    if value.ty != Ty::Char {
                        return Err("character cast result type mismatch".into());
                    }
                    if value.completion != source.completion {
                        return Err("character cast completion mismatch".into());
                    }
                }
                ExprKind::PtrCast { value: source } => {
                    expr(*source)?;
                    let source = &self.exprs[source.0];
                    if !ptr_cast_valid(&source.ty, &value.ty) {
                        return Err("pointer cast requires a supported pointer pair".into());
                    }
                    if value.completion != source.completion {
                        return Err("pointer cast completion mismatch".into());
                    }
                }
                ExprKind::Intrinsic { intrinsic, args } => {
                    let signature = intrinsic
                        .signature()
                        .ok_or("primitive method has no signature for its receiver")?;
                    if args.len() != signature.params.len() {
                        return Err("primitive method arity mismatch".into());
                    }
                    let mut completion = Completion::Returns;
                    for (arg, param) in args.iter().zip(&signature.params) {
                        expr(*arg)?;
                        let operand = &self.exprs[arg.0];
                        if operand.ty != *param && operand.ty != Ty::Never {
                            return Err("primitive method operand type mismatch".into());
                        }
                        if operand.completion == Completion::Never {
                            completion = Completion::Never;
                        }
                    }
                    if value.ty != signature.result {
                        return Err("primitive method result type mismatch".into());
                    }
                    if value.completion != completion {
                        return Err("primitive method completion mismatch".into());
                    }
                }
                ExprKind::LeadingZeros { value: source } => {
                    expr(*source)?;
                    let source = &self.exprs[source.0];
                    if !matches!(source.ty, Ty::Int(_)) {
                        return Err("leading zeros requires integer source".into());
                    }
                    if value.ty != Ty::Int(IntTy::U32) {
                        return Err("leading zeros result type mismatch".into());
                    }
                    if value.completion != source.completion {
                        return Err("leading zeros completion mismatch".into());
                    }
                }
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
                ExprKind::FloatCast { value: source, to } => {
                    expr(*source)?;
                    let source = &self.exprs[source.0];
                    let Some(from) = NumTy::of(&source.ty) else {
                        return Err("float cast requires numeric source".into());
                    };
                    if !matches!((from, to), (NumTy::Float(_), _) | (_, NumTy::Float(_))) {
                        return Err("float cast requires a float endpoint".into());
                    }
                    if value.ty != to.ty() {
                        return Err("float cast result type mismatch".into());
                    }
                    if value.completion != source.completion {
                        return Err("float cast completion mismatch".into());
                    }
                }
                ExprKind::Float(float) => {
                    float.validate()?;
                    if value.ty != Ty::Float(float.ty) || value.completion != Completion::Returns {
                        return Err("float expression type or completion mismatch".into());
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
                    for projection in &place.projection {
                        if let Projection::DynamicIndex(index) = projection
                            && self.locals.get(index.0).map(|local| &local.ty)
                                != Some(&Ty::Int(IntTy::Usize))
                        {
                            return Err("runtime borrow index requires a valid usize local".into());
                        }
                    }
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
                    let (Ty::Ref { referent, .. }
                    | Ty::RawPtr {
                        pointee: referent, ..
                    }) = &self.exprs[source.0].ty
                    else {
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
                ExprKind::ArrayToSlice { reference } => {
                    expr(*reference)?;
                    let source = &self.exprs[reference.0];
                    let Some((from, source_element, _)) = source.ty.array_ref_element_len() else {
                        return Err("array-to-slice requires array reference".into());
                    };
                    let Some((to, result_element)) = value.ty.slice_ref_element() else {
                        return Err("array-to-slice result must be a slice reference".into());
                    };
                    if from != to {
                        return Err("array-to-slice changed reference mutability".into());
                    }
                    validate_shared_slice_element_with_facts(source_element, facts)?;
                    validate_shared_slice_element_with_facts(result_element, facts)?;
                    if !pattern_types_equal(source_element, result_element, &mut 0)? {
                        return Err("array-to-slice element type mismatch".into());
                    }
                    if value.completion != source.completion {
                        return Err("array-to-slice completion mismatch".into());
                    }
                }
                ExprKind::RefToDyn { reference } => {
                    expr(*reference)?;
                    let source = &self.exprs[reference.0];
                    let (
                        Ty::Ref {
                            kind: from,
                            referent: concrete,
                        },
                        Ty::Ref {
                            kind: to,
                            referent: object,
                        },
                    ) = (&source.ty, &value.ty)
                    else {
                        return Err("trait object unsizing requires references".into());
                    };
                    if from != to || !matches!(object.as_ref(), Ty::Dyn(_)) {
                        return Err("trait object unsizing changed reference shape".into());
                    }
                    if matches!(
                        concrete.as_ref(),
                        Ty::Dyn(_) | Ty::Slice { .. } | Ty::Never | Ty::AssociatedType(_)
                    ) {
                        return Err("unsupported: trait object unsizing source".into());
                    }
                    if value.completion != source.completion {
                        return Err("trait object unsizing completion mismatch".into());
                    }
                }
                ExprKind::SliceLen { slice } => {
                    expr(*slice)?;
                    let source = &self.exprs[slice.0];
                    if !source.ty.is_shared_str() {
                        let Some(element) = source.ty.shared_slice_element() else {
                            return Err("slice length requires shared slice or str".into());
                        };
                        validate_shared_slice_element_with_facts(element, facts)?;
                    }
                    if value.ty != Ty::Int(IntTy::Usize) {
                        return Err("slice length result must be usize".into());
                    }
                    if value.completion != source.completion {
                        return Err("slice length completion mismatch".into());
                    }
                }
                ExprKind::Subslice {
                    slice, start, end, ..
                } => {
                    expr(*slice)?;
                    for bound in [start, end].into_iter().flatten() {
                        expr(*bound)?;
                        if self.exprs[bound.0].ty != Ty::Int(IntTy::Usize)
                            && self.exprs[bound.0].ty != Ty::Never
                        {
                            return Err("subslice bound requires usize".into());
                        }
                    }
                    let Some(element) = self.exprs[slice.0].ty.shared_slice_element() else {
                        return Err("subslice requires shared slice".into());
                    };
                    validate_shared_slice_element_with_facts(element, facts)?;
                    if !pattern_types_equal(&value.ty, &self.exprs[slice.0].ty, &mut 0)? {
                        return Err("subslice result must equal its shared slice type".into());
                    }
                }
                ExprKind::SliceIndex { slice, index } => {
                    expr(*slice)?;
                    expr(*index)?;
                    let Some(element) = self.exprs[slice.0].ty.shared_slice_element() else {
                        return Err("slice index requires shared slice".into());
                    };
                    validate_shared_slice_element_with_facts(element, facts)?;
                    if self.exprs[index.0].ty != Ty::Int(IntTy::Usize) {
                        return Err("slice index requires usize".into());
                    }
                    if !pattern_types_equal(&value.ty, element, &mut 0)? {
                        return Err("slice index result element type mismatch".into());
                    }
                    if !facts.must_copy(element)? {
                        return Err(
                            "slice index cannot move a non-Copy element from borrowed storage"
                                .into(),
                        );
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
                    for arm in arms {
                        match &arm.pattern {
                            MatchPattern::Aggregate { ty, .. }
                            | MatchPattern::Alternatives { ty, .. } => {
                                facts.validate_value_ty(ty)?
                            }
                            MatchPattern::Decision { ty, tests, .. } => {
                                facts.validate_value_ty(ty)?;
                                for test in tests {
                                    if let PatternTestKind::EnumVariant { ty, .. } = &test.kind {
                                        facts.validate_value_ty(&Ty::Enum(ty.clone()))?;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    self.validate_match(*scrutinee, arms, &parameters)?;
                    for arm in arms {
                        if let Some(guard) = arm.guard {
                            expr(guard)?;
                        }
                        expr(arm.body)?;
                    }
                }
                ExprKind::Local(id) => local(*id)?,
                ExprKind::NativeSyscall(arguments) => {
                    if arguments.len() != 7 || value.ty != Ty::Int(IntTy::Usize) {
                        return Err("invalid native syscall shape".into());
                    }
                    let mut completes = true;
                    for &id in arguments {
                        expr(id)?;
                        let argument = &self.exprs[id.0];
                        if argument.ty != Ty::Int(IntTy::Usize) {
                            return Err("native syscall arguments must be usize".into());
                        }
                        completes &= argument.completion == Completion::Returns;
                    }
                    if (value.completion == Completion::Returns) != completes {
                        return Err("native syscall completion mismatch".into());
                    }
                }
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
                ExprKind::Unary { op, value: operand } => {
                    expr(*operand)?;
                    let operand = &self.exprs[operand.0];
                    if operand.ty == Ty::Char {
                        return Err("unary operators do not accept char".into());
                    }
                    if matches!(operand.ty, Ty::Float(_))
                        && (*op != UnaryOp::Neg
                            || value.ty != operand.ty
                            || value.completion != operand.completion)
                    {
                        return Err("invalid float unary type, operator or completion".into());
                    }
                }
                ExprKind::Binary { op, left, right } => {
                    expr(*left)?;
                    expr(*right)?;
                    let lhs = &self.exprs[left.0];
                    let rhs = &self.exprs[right.0];
                    if lhs.ty.is_shared_str() || rhs.ty.is_shared_str() {
                        let comparison = matches!(
                            op,
                            BinaryOp::Eq
                                | BinaryOp::Ne
                                | BinaryOp::Lt
                                | BinaryOp::Le
                                | BinaryOp::Gt
                                | BinaryOp::Ge
                        );
                        let completion = if lhs.completion == Completion::Returns
                            && rhs.completion == Completion::Returns
                        {
                            Completion::Returns
                        } else {
                            Completion::Never
                        };
                        if !comparison
                            || !lhs.ty.is_shared_str()
                            || !rhs.ty.is_shared_str()
                            || value.ty != Ty::Bool
                            || value.completion != completion
                        {
                            return Err(
                                "&str permits only &str comparisons with bool result".into()
                            );
                        }
                    } else if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
                        // Metadata for the entire arena was validated above. Equality
                        // remains a closed scalar operation, not structural/trait equality.
                        if !matches!(
                            lhs.ty,
                            Ty::Unit
                                | Ty::Bool
                                | Ty::Char
                                | Ty::Int(_)
                                | Ty::Float(_)
                                | Ty::RawPtr { .. }
                        ) || !ref_types_equal(&lhs.ty, &rhs.ty, conditional_budget, 0)?
                            || !ref_types_equal(&value.ty, &Ty::Bool, conditional_budget, 0)?
                        {
                            return Err(
                                "equality requires identical admitted scalar types and bool result"
                                    .into(),
                            );
                        }
                        let completion = if lhs.completion == Completion::Returns
                            && rhs.completion == Completion::Returns
                        {
                            Completion::Returns
                        } else {
                            Completion::Never
                        };
                        if value.completion != completion {
                            return Err("equality completion mismatch".into());
                        }
                    }
                    if matches!(op, BinaryOp::And | BinaryOp::Or) {
                        // Only logical operators admit semantic Never coercion.
                        // A nonreturning Bool is also legal. Short-circuit flow
                        // depends on the LHS, not on whether the RHS returns.
                        for operand in [lhs, rhs] {
                            if !ref_types_equal(&operand.ty, &Ty::Bool, conditional_budget, 0)?
                                && !(ref_types_equal(
                                    &operand.ty,
                                    &Ty::Never,
                                    conditional_budget,
                                    0,
                                )? && operand.completion == Completion::Never)
                            {
                                return Err(
                                    "logical operator requires bool or diverging never operands"
                                        .into(),
                                );
                            }
                        }
                        if !ref_types_equal(&value.ty, &Ty::Bool, conditional_budget, 0)?
                            || value.completion != lhs.completion
                        {
                            return Err(
                                "logical operator result type or completion mismatch".into()
                            );
                        }
                    }
                    if lhs.ty == Ty::Char || rhs.ty == Ty::Char {
                        let comparison = matches!(
                            op,
                            BinaryOp::Eq
                                | BinaryOp::Ne
                                | BinaryOp::Lt
                                | BinaryOp::Le
                                | BinaryOp::Gt
                                | BinaryOp::Ge
                        );
                        let completion = if lhs.completion == Completion::Returns
                            && rhs.completion == Completion::Returns
                        {
                            Completion::Returns
                        } else {
                            Completion::Never
                        };
                        if !comparison
                            || lhs.ty != rhs.ty
                            || value.ty != Ty::Bool
                            || value.completion != completion
                        {
                            return Err(
                                "char permits only identical-type comparisons with bool result"
                                    .into(),
                            );
                        }
                    }
                    if matches!(lhs.ty, Ty::Float(_)) || matches!(rhs.ty, Ty::Float(_)) {
                        let comparison = matches!(
                            op,
                            BinaryOp::Eq
                                | BinaryOp::Ne
                                | BinaryOp::Lt
                                | BinaryOp::Le
                                | BinaryOp::Gt
                                | BinaryOp::Ge
                        );
                        let arithmetic = matches!(
                            op,
                            BinaryOp::Add
                                | BinaryOp::Sub
                                | BinaryOp::Mul
                                | BinaryOp::Div
                                | BinaryOp::Rem
                        );
                        let expected = if comparison { Ty::Bool } else { lhs.ty.clone() };
                        let completion = if lhs.completion == Completion::Returns
                            && rhs.completion == Completion::Returns
                        {
                            Completion::Returns
                        } else {
                            Completion::Never
                        };
                        if !(comparison || arithmetic)
                            || lhs.ty != rhs.ty
                            || value.ty != expected
                            || value.completion != completion
                        {
                            return Err("invalid float binary types, operator or completion".into());
                        }
                    }
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
                    let condition = &self.exprs[condition.0];
                    if condition.ty != Ty::Bool
                        && !(condition.ty == Ty::Never && condition.completion == Completion::Never)
                    {
                        return Err("if condition requires bool or diverging never".into());
                    }
                    conditional_budget.charge_work(1)?;
                    let (then_types, then_definite, then_possible) =
                        self.conditional_block_flow(then_block, conditional_budget)?;
                    let (else_types, else_definite, else_possible) =
                        self.conditional_block_flow(else_block, conditional_budget)?;
                    let mut valid_join = false;
                    for then_ty in then_types.into_iter().flatten() {
                        for else_ty in else_types.into_iter().flatten() {
                            valid_join |= Self::conditional_join_matches(
                                then_ty,
                                else_ty,
                                &value.ty,
                                conditional_budget,
                            )?;
                        }
                    }
                    if !valid_join {
                        return Err("if branch result type mismatch".into());
                    }
                    let condition_returns = condition.completion == Completion::Returns;
                    let definitely_returns = condition_returns && (then_definite || else_definite);
                    let possibly_returns = condition_returns && (then_possible || else_possible);
                    if (value.completion == Completion::Returns && !possibly_returns)
                        || (value.completion == Completion::Never && definitely_returns)
                    {
                        return Err("if completion mismatch".into());
                    }
                    blocks.push(then_block);
                    blocks.push(else_block);
                }
                ExprKind::Block(block)
                | ExprKind::Loop { body: block, .. }
                | ExprKind::LabeledBlock { body: block, .. } => blocks.push(block),
                ExprKind::Break {
                    value: Some(value), ..
                } => expr(*value)?,
                ExprKind::Break { value: None, .. } | ExprKind::Continue { .. } => {}
                ExprKind::Str(_) => {
                    if !value.ty.is_shared_str() || value.completion != Completion::Returns {
                        return Err("string literal expression type mismatch".into());
                    }
                }
                ExprKind::Char(character) => {
                    character.validate()?;
                    if value.ty != Ty::Char || value.completion != Completion::Returns {
                        return Err("character expression type mismatch".into());
                    }
                }
                ExprKind::Int(integer) => {
                    integer.validate()?;
                    if value.ty != Ty::Int(integer.ty) || value.completion != Completion::Returns {
                        return Err("integer expression type mismatch".into());
                    }
                }
                ExprKind::Panic(_) => {
                    if value.ty != Ty::Never || value.completion != Completion::Never {
                        return Err(
                            "panic requires exact Never type and nonreturning completion".into(),
                        );
                    }
                }
                ExprKind::Output(_) => {
                    if value.ty != Ty::Unit || value.completion != Completion::Returns {
                        return Err(
                            "literal output requires exact Unit type and returning completion"
                                .into(),
                        );
                    }
                }
                ExprKind::Bool(_) | ExprKind::Println(_) => {}
                ExprKind::Format {
                    pieces,
                    arguments,
                    modes,
                    ..
                } => {
                    if pieces.len() != arguments.len() + 1 || modes.len() != arguments.len() {
                        return Err("format argument count mismatch".into());
                    }
                    let mut completion = Completion::Returns;
                    for (&id, &mode) in arguments.iter().zip(modes) {
                        expr(id)?;
                        let argument = &self.exprs[id.0];
                        let ty = if matches!(
                            argument.kind,
                            ExprKind::Borrow {
                                kind: BorrowKind::Shared,
                                ..
                            }
                        ) {
                            let Ty::Ref { referent, .. } = &argument.ty else {
                                return Err("format borrow requires reference type".into());
                            };
                            referent.as_ref()
                        } else {
                            &argument.ty
                        };
                        validate_format_ty(
                            mode,
                            ty,
                            facts.validated().map(|facts| facts.budget()),
                        )?;
                        if argument.completion == Completion::Never {
                            completion = Completion::Never;
                        }
                    }
                    if value.ty != Ty::Unit || value.completion != completion {
                        return Err("format output type or completion mismatch".into());
                    }
                }
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
                        if !array.projection.contains(&Projection::Deref)
                            && !self.locals[array.local.0].mutable
                        {
                            return Err("compound indexed mutation requires mutable binding".into());
                        }
                        let array_ty = indexed_array_ty(
                            &self.locals[array.local.0].ty,
                            &array.projection,
                            true,
                        )?;
                        let (Ty::Array { element, .. } | Ty::Slice { element }) = array_ty else {
                            return Err("compound indexed assignment requires array type".into());
                        };
                        if matches!(array_ty, Ty::Slice { .. }) && !facts.must_copy(element)? {
                            return Err(
                                "unsupported: mutable slice assignment of non-Copy elements".into(),
                            );
                        }
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
                        let array_ty = indexed_array_ty(
                            &self.locals[array.local.0].ty,
                            &array.projection,
                            false,
                        )?;
                        if let Ty::Slice { element } = array_ty {
                            if !facts.must_copy(element)? {
                                return Err(
                                    "unsupported: mutable slice assignment of non-Copy elements"
                                        .into(),
                                );
                            }
                            // `*s` of a slice carrier: writable only through
                            // `&mut` at every dereference (MUTABLE-SLICES.md).
                            indexed_array_ty(
                                &self.locals[array.local.0].ty,
                                &array.projection,
                                true,
                            )?;
                        } else if !matches!(array_ty, Ty::Array { .. }) {
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
                    Stmt::LetElse {
                        value,
                        ty,
                        pattern,
                        else_branch,
                    } => {
                        expr(*value)?;
                        expr(*else_branch)?;
                        facts.validate_value_ty(ty)?;
                        match pattern {
                            MatchPattern::EnumVariant { ty, .. } => {
                                facts.validate_value_ty(&Ty::Enum(ty.clone()))?
                            }
                            MatchPattern::Aggregate { ty, .. }
                            | MatchPattern::Alternatives { ty, .. } => {
                                facts.validate_value_ty(ty)?
                            }
                            MatchPattern::Decision { ty, tests, .. } => {
                                facts.validate_value_ty(ty)?;
                                for test in tests {
                                    if let PatternTestKind::EnumVariant { ty, .. } = &test.kind {
                                        facts.validate_value_ty(&Ty::Enum(ty.clone()))?;
                                    }
                                }
                            }
                            _ => {}
                        }
                        self.validate_let_else(*value, ty, pattern, *else_branch, &parameters)?;
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
#[path = "ir/typed_slice_tests.rs"]
mod typed_slice_tests;

#[cfg(test)]
mod conditional_validation_tests {
    use super::*;

    fn block(statements: Vec<Stmt>, tail: Option<ExprId>) -> Block {
        Block { statements, tail }
    }

    fn fixture() -> CheckedFunction {
        CheckedFunction {
            call_evidence: Arc::from([]),
            body_substitution: None,
            ref_contract: Default::default(),
            name: "conditional".into(),
            params: vec![],
            result: Ty::Unit,
            locals: vec![],
            exprs: vec![
                TypedExpr {
                    ty: Ty::Bool,
                    completion: Completion::Returns,
                    kind: ExprKind::Bool(true),
                },
                TypedExpr {
                    ty: Ty::Never,
                    completion: Completion::Never,
                    kind: ExprKind::Panic(Box::new(PanicMessage {
                        message: "assertion failed".into(),
                        location: "fixture:1:1".into(),
                    })),
                },
                TypedExpr {
                    ty: Ty::Unit,
                    completion: Completion::Returns,
                    kind: ExprKind::If {
                        condition: ExprId(0),
                        then_block: block(vec![], None),
                        else_block: block(vec![Stmt::Expr(ExprId(1))], None),
                    },
                },
            ],
            // Deliberately unused: validation must check all retained rows.
            body: block(vec![], None),
        }
    }

    #[test]
    fn char_scalar_domain_is_checked_even_in_unused_ir() {
        assert!(Ty::Char.is_copy());
        assert!(NumTy::of(&Ty::Char).is_none());
        for value in [0, 0xd7ff, 0xe000, 0x10ffff] {
            let mut function = fixture();
            function.exprs.push(TypedExpr {
                ty: Ty::Char,
                completion: Completion::Returns,
                kind: ExprKind::Char(CharValue { value }),
            });
            function.validate().unwrap();
            let last = function.exprs.len() - 1;
            function.exprs[last].completion = Completion::Never;
            assert!(function.validate().is_err());
            function.exprs[last].completion = Completion::Returns;
            function.exprs[last].ty = Ty::Int(IntTy::U32);
            assert!(function.validate().is_err());
        }
        for value in [0xd800, 0xdfff, 0x110000, u32::MAX] {
            let mut function = fixture();
            function.exprs.push(TypedExpr {
                ty: Ty::Char,
                completion: Completion::Returns,
                kind: ExprKind::Char(CharValue { value }),
            });
            assert!(function.validate().unwrap_err().contains("Unicode scalar"));
        }
        assert!(identical_type(&Ty::Char, &Ty::Char, &PlanBudget::default()).unwrap());
        for other in [Ty::Int(IntTy::U32), Ty::Bool, Ty::Float(FloatTy::F32)] {
            assert!(!identical_type(&Ty::Char, &other, &PlanBudget::default()).unwrap());
        }
    }

    #[test]
    fn unused_char_operations_require_exact_comparison_domain() {
        for op in [
            BinaryOp::Eq,
            BinaryOp::Ne,
            BinaryOp::Lt,
            BinaryOp::Le,
            BinaryOp::Gt,
            BinaryOp::Ge,
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
            let mut function = fixture();
            function.exprs = vec![
                TypedExpr {
                    ty: Ty::Char,
                    completion: Completion::Returns,
                    kind: ExprKind::Char(CharValue { value: 0xd7ff }),
                },
                TypedExpr {
                    ty: Ty::Char,
                    completion: Completion::Returns,
                    kind: ExprKind::Char(CharValue { value: 0xe000 }),
                },
                TypedExpr {
                    ty: Ty::Bool,
                    completion: Completion::Returns,
                    kind: ExprKind::Binary {
                        op,
                        left: ExprId(0),
                        right: ExprId(1),
                    },
                },
            ];
            let comparison = matches!(
                op,
                BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge
            );
            assert_eq!(function.validate().is_ok(), comparison, "{op:?}");
            if comparison {
                function.exprs[2].ty = Ty::Char;
                assert!(function.validate().is_err());
                function.exprs[2].ty = Ty::Bool;
                function.exprs[1].ty = Ty::Int(IntTy::U32);
                function.exprs[1].kind = ExprKind::Int(IntValue {
                    ty: IntTy::U32,
                    bits: 0xe000,
                });
                assert!(function.validate().is_err());
            }
        }
    }

    #[test]
    fn unused_assertion_conditional_rejects_forged_result_and_condition() {
        let function = fixture();
        function.validate().unwrap();
        let mut forged = function.clone();
        forged.exprs[2].ty = Ty::Int(IntTy::I32);
        assert!(forged.validate().unwrap_err().contains("branch result"));
        let mut forged = function.clone();
        forged.exprs[0].ty = Ty::Int(IntTy::I32);
        assert!(forged.validate().unwrap_err().contains("if condition"));
        let mut forged = function;
        forged.exprs[2].completion = Completion::Never;
        assert!(forged.validate().unwrap_err().contains("if completion"));
    }

    #[test]
    fn never_condition_must_diverge_and_controls_conditional_completion() {
        let mut function = fixture();
        let ExprKind::If { condition, .. } = &mut function.exprs[2].kind else {
            unreachable!()
        };
        *condition = ExprId(1);
        function.exprs[2].completion = Completion::Never;
        function.validate().unwrap();
        function.exprs[2].completion = Completion::Returns;
        assert!(function.validate().unwrap_err().contains("if completion"));
        // A forged Never row that returns is not a Bool coercion.
        function.exprs[0].ty = Ty::Never;
        let ExprKind::If { condition, .. } = &mut function.exprs[2].kind else {
            unreachable!()
        };
        *condition = ExprId(0);
        assert!(function.validate().unwrap_err().contains("if condition"));
    }

    #[test]
    fn dead_explicit_tail_still_constrains_join_but_not_completion() {
        let mut function = fixture();
        function.exprs[0].ty = Ty::Bool;
        let ExprKind::If { then_block, .. } = &mut function.exprs[2].kind else {
            unreachable!()
        };
        *then_block = block(vec![Stmt::Return(None)], Some(ExprId(0)));
        function.exprs[2].ty = Ty::Bool;
        function.exprs[2].completion = Completion::Never;
        function.validate().unwrap();
        function.exprs[2].ty = Ty::Unit;
        assert!(function.validate().unwrap_err().contains("branch result"));
        function.exprs[2].ty = Ty::Bool;
        function.exprs[2].completion = Completion::Returns;
        assert!(function.validate().unwrap_err().contains("if completion"));
    }

    #[test]
    fn erased_while_let_obligations_preserve_both_source_possibilities() {
        let mut function = fixture();
        function.exprs.push(TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Never,
            kind: ExprKind::Loop {
                id: LoopId(0),
                body: block(vec![], None),
            },
        });
        let ExprKind::If { then_block, .. } = &mut function.exprs[2].kind else {
            unreachable!()
        };
        *then_block = block(vec![Stmt::Expr(ExprId(3))], None);
        // An erased irrefutable while-let can retain source Returns.
        function.validate().unwrap();
        function.exprs[2].completion = Completion::Never;
        function.validate().unwrap();
        function.exprs[2].ty = Ty::Never;
        function.validate().unwrap();
        function.exprs[2].ty = Ty::Int(IntTy::I32);
        assert!(function.validate().unwrap_err().contains("branch result"));
    }

    #[test]
    fn aggregate_join_is_exact_memoized_and_uses_original_budget() {
        let left = Ty::Tuple(vec![Ty::Never, Ty::Bool].into());
        let right = Ty::Tuple(vec![Ty::Int(IntTy::I32), Ty::Bool].into());
        let budget = PlanBudget::default();
        assert!(CheckedFunction::conditional_join_matches(&left, &right, &right, &budget).unwrap());
        assert!(!CheckedFunction::conditional_join_matches(&left, &right, &left, &budget).unwrap());
        let exhausted = PlanBudget::new(PlanLimits {
            work: 0,
            ..PlanLimits::default()
        });
        assert!(
            CheckedFunction::conditional_join_matches(&left, &right, &right, &exhausted).is_err()
        );
        assert!(exhausted.has_resource_failure());

        let mut dag = Ty::Bool;
        for _ in 0..24 {
            dag = Ty::Tuple(vec![dag.clone(), dag].into());
        }
        let bounded = PlanBudget::new(PlanLimits {
            work: 10_000,
            ..PlanLimits::default()
        });
        assert!(CheckedFunction::conditional_join_matches(&dag, &dag, &dag, &bounded).unwrap());
    }
}

#[cfg(test)]
mod original_replay_transport_tests {
    use super::*;
    use crate::compiler::{frontend, traits::FactsErrorKind};

    #[test]
    fn original_replay_raw_forgery_exact_owner_facts_and_resource_fences() {
        let latch = PlanBudget::new(PlanLimits {
            bytes: 8,
            ..PlanLimits::default()
        });
        let shared = latch.clone();
        assert!(!latch.has_resource_failure());
        let reservation = latch.reserve_bytes(4).unwrap();
        assert!(!shared.has_resource_failure());
        assert!(shared.retain_bytes(5).is_err());
        assert!(latch.has_resource_failure());
        drop(reservation);
        assert_eq!(latch.usage.lock().unwrap().bytes, 0);
        shared.retain_bytes(1).unwrap();
        assert!(latch.has_resource_failure());
        assert!(shared.has_resource_failure());
        assert!(!PlanBudget::default().has_resource_failure());
        let depth = PlanBudget::default();
        depth.check_depth(depth.limits().depth).unwrap();
        assert!(!depth.has_resource_failure());
        assert!(depth.clone().check_depth(depth.limits().depth + 1).is_err());
        assert!(depth.has_resource_failure());
        let overflow = PlanBudget::default();
        overflow.usage.lock().unwrap().work = usize::MAX;
        assert!(overflow.clone().charge_work(1).is_err());
        assert!(overflow.has_resource_failure());
        let poisoned = PlanBudget::default();
        let poison_clone = poisoned.clone();
        let panic = std::panic::catch_unwind(|| {
            let _guard = poisoned.usage.lock().unwrap();
            panic!("intentional ledger poison for conservative resource evidence");
        });
        assert!(panic.is_err());
        assert!(poisoned.has_resource_failure());
        assert!(poison_clone.has_resource_failure());
        assert!(poison_clone.charge_work(1).is_err());
        let budget = PlanBudget::default();
        assert_eq!(
            original_capsule_allocation(0, &budget).unwrap(),
            (1, 2 * std::mem::size_of::<usize>())
        );
        assert_eq!(
            original_capsule_allocation(usize::MAX, &budget)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Resource
        );
        assert_eq!(
            original_capsule_allocation(usize::MAX / 2, &budget)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Resource
        );
        for exhausted in 0..3 {
            let limited = PlanBudget::default();
            let empty = frontend::parse("fn empty(){} fn main(){}").unwrap();
            let empty_plan = frontend::PlanCtx::from_program(&empty, &limited).unwrap();
            let frontend::PublishedCallableBody::Source(empty_publication) = empty_plan
                .publish_callable_body(empty_plan.function_decl(0).unwrap(), &limited)
                .unwrap()
            else {
                panic!("canonical empty publication required")
            };
            assert!(
                validated_call_capsules(&empty_publication, &limited)
                    .unwrap()
                    .is_empty()
            );
            let (used_nodes, used_bytes, used_work) = {
                let used = limited.usage.lock().unwrap();
                (used.nodes, used.bytes, used.work)
            };
            match exhausted {
                0 => limited
                    .charge_nodes(limited.limits().nodes - used_nodes)
                    .unwrap(),
                1 => limited
                    .retain_bytes(limited.limits().bytes - used_bytes)
                    .unwrap(),
                _ => limited
                    .charge_work(limited.limits().work - used_work)
                    .unwrap(),
            }
            assert!(!limited.has_resource_failure());
            assert_eq!(
                validated_call_capsules(&empty_publication, &limited)
                    .unwrap_err()
                    .kind(),
                FactsErrorKind::Resource
            );
            assert!(limited.has_resource_failure());
        }
        let program = frontend::parse(
            "fn id<T:Copy>(x:T)->T{x} fn caller(r:&i32){let held=id(r);} fn main(){}",
        )
        .unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let frontend::PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("canonical source publication required")
        };
        validate_published_source(&publication, &budget).unwrap();
        let source = publication.source();
        let facts = publication.facts();
        let capsules = validated_call_capsules(&publication, &budget).unwrap();
        assert_eq!(capsules.len(), 1);
        assert!(Arc::ptr_eq(capsules[0].source(), source));
        assert!(Arc::ptr_eq(
            capsules[0].original().unwrap(),
            publication.replay()
        ));
        assert_eq!(
            capsules[0].trace().arguments[0].expectation,
            CallExpectation::NoFirmReference
        );
        // The new permission belongs only to the original source publication.
        assert!(bind_call_evidence(source.clone(), facts.clone(), &budget).is_err());
        let mut raw = source.as_ref().clone();
        let mut rows = raw.call_evidence.to_vec();
        let mut arguments = rows[0].arguments.to_vec();
        arguments[0].expectation = CallExpectation::FirmReference {
            kind: BorrowKind::Shared,
        };
        rows[0].arguments = arguments.into();
        raw.call_evidence = rows.into();
        let raw = Arc::new(raw);
        let legacy = bind_call_evidence(raw.clone(), facts.clone(), &budget).unwrap();
        assert!(legacy.iter().all(|capsule| capsule.original().is_none()));
        assert!(publication.validate_source(&raw, facts, &budget).is_err());
        let same_types = Arc::new(source.as_ref().clone());
        assert!(
            publication
                .validate_source(&same_types, facts, &budget)
                .is_err()
        );
        let mut edited = source.clone();
        Arc::make_mut(&mut edited).name.push_str("_changed");
        assert!(!Arc::ptr_eq(&edited, source));
        assert!(
            publication
                .validate_source(&edited, facts, &budget)
                .is_err()
        );
        let equal_facts = Arc::new(facts.as_ref().clone());
        assert!(
            publication
                .validate_source(source, &equal_facts, &budget)
                .is_err()
        );
        assert!(
            publication
                .validate_source(source, facts, &PlanBudget::default())
                .is_err()
        );
        publication.validate_source(source, facts, &budget).unwrap();
        // Exhaust this exact ledger, not a fresh low-budget foreign authority.
        let used = budget.usage.lock().unwrap().work;
        budget.charge_work(budget.limits().work - used).unwrap();
        let error = publication
            .validate_source(source, facts, &budget)
            .unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::Resource);
    }
}

#[cfg(test)]
mod tests {
    fn fixture_modules() -> std::sync::Arc<super::super::module_identity::ModuleArena> {
        super::super::module_identity::ModuleArenaBuilder::for_catalog(
            super::super::expansion_budget::ExpansionBudget::default(),
        )
        .unwrap()
        .freeze()
        .unwrap()
    }
    #[test]
    fn visibility_scopes_use_module_boundaries_and_cannot_widen() {
        use crate::compiler::expansion_budget::ExpansionBudget;
        use crate::compiler::module_identity::{ModuleArenaBuilder, ModuleError};
        use crate::compiler::semantic_name::NameStoreBuilder;
        use crate::compiler::token::Edition;
        use crate::compiler::token_session::{SourceSnapshot, TokenSession};
        let budget = ExpansionBudget::default();
        let source =
            SourceSnapshot::from_text(std::path::Path::new("visibility.rs"), "", &budget).unwrap();
        let origin = source.span(source.file(0).unwrap(), 0, 0).unwrap();
        let (mut session, session_cap) = TokenSession::standalone(source, budget).unwrap();
        let origin = session.original_span(origin).unwrap();
        let mut names = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let mut builder = ModuleArenaBuilder::for_source(&names).unwrap();
        let root = builder.source_root().unwrap();
        let mut child = |parent, text| {
            let ident = names.intern_root(text, origin).unwrap();
            builder.intern_child(&names, parent, ident).unwrap()
        };
        let a_id = child(root, "a");
        let ab_id = child(a_id, "b");
        let child_id = child(ab_id, "child");
        let ac_id = child(a_id, "c");
        let aa_id = child(root, "ab");
        let underscore_id = child(root, "a_b");
        let other_id = child(root, "other");
        let bc_id = child(a_id, "bc");
        let modules = builder.freeze().unwrap();
        let public = super::Visibility::Public;
        let krate = super::Visibility::Restricted(root);
        let a = super::Visibility::Restricted(a_id);
        let ab = super::Visibility::Restricted(ab_id);
        let sibling = super::Visibility::Restricted(ac_id);
        for module in [root, a_id, ab_id, child_id, aa_id, other_id] {
            assert!(public.accessible_from(&modules, module).unwrap());
            assert!(krate.accessible_from(&modules, module).unwrap());
        }
        for module in [a_id, ab_id, child_id, ac_id] {
            assert!(a.accessible_from(&modules, module).unwrap());
        }
        for module in [root, aa_id, underscore_id, other_id, bc_id] {
            assert!(!ab.accessible_from(&modules, module).unwrap());
        }
        assert!(ab.accessible_from(&modules, ab_id).unwrap());
        assert!(ab.accessible_from(&modules, child_id).unwrap());
        assert!(public.allows_reexport(&modules, &krate).unwrap());
        assert!(public.allows_reexport(&modules, &ab).unwrap());
        assert!(krate.allows_reexport(&modules, &ab).unwrap());
        assert!(a.allows_reexport(&modules, &ab).unwrap());
        assert!(ab.allows_reexport(&modules, &ab).unwrap());
        assert!(!ab.allows_reexport(&modules, &a).unwrap());
        assert!(!ab.allows_reexport(&modules, &sibling).unwrap());
        assert!(!krate.allows_reexport(&modules, &public).unwrap());
        assert!(!a.allows_reexport(&modules, &public).unwrap());
        assert!(
            !krate
                .accessible_from(&modules, modules.core_root())
                .unwrap()
        );
        assert!(a.validate_for(&modules, child_id).is_ok());
        assert!(matches!(
            ab.validate_for(&modules, a_id),
            Err(ModuleError::InvalidRestriction)
        ));
        let foreign = fixture_modules();
        assert!(matches!(
            public.accessible_from(&modules, foreign.core_root()),
            Err(ModuleError::ForeignOwner)
        ));
        assert!(matches!(
            public.validate_for(&modules, foreign.core_root()),
            Err(ModuleError::ForeignOwner)
        ));
        assert!(matches!(
            public.allows_reexport(
                &modules,
                &super::Visibility::Restricted(foreign.core_root())
            ),
            Err(ModuleError::ForeignOwner)
        ));
        assert!(public.is_public());
        assert!(!krate.is_public());
    }

    #[test]
    fn public_reexport_checks_poisoned_module_authority() {
        use crate::compiler::expansion_budget::{Counter, ExpansionLimits};
        let modules = fixture_modules();
        let first = modules
            .budget()
            .charge(
                Counter::Work,
                ExpansionLimits::default().counters[Counter::Work as usize],
            )
            .unwrap_err();
        let error = super::Visibility::Public
            .allows_reexport(&modules, &super::Visibility::Public)
            .unwrap_err();
        let super::super::module_identity::ModuleError::Token(
            super::super::token_session::TokenError::Budget(error),
        ) = error
        else {
            panic!("lost budget failure kind")
        };
        assert_eq!(first, error);
    }

    #[test]
    fn trait_default_self_place_is_not_owned_value_authority() {
        for sized in [false, true] {
            let source = if sized {
                "trait V:Sized{fn value(&self)->i32{7}}fn main(){}"
            } else {
                "trait V{fn value(&self)->i32{7}}fn main(){}"
            };
            let program = super::super::frontend::parse(source).unwrap();
            let budget = PlanBudget::default();
            let plan = super::super::frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = (0..plan.callable_count())
                .map(|index| plan.callable_decl(index).unwrap())
                .find(|id| plan.declaration(*id).unwrap().kind == DeclKind::AssociatedFunction)
                .unwrap();
            let facts = plan.callable_facts(declaration).unwrap();
            let param = Ty::Param(facts.env().ordered_params().next().unwrap());
            let authority = FactsCtx::from_validated(&facts);
            authority.validate_ty(&param).unwrap();
            assert_eq!(authority.validate_value_ty(&param).is_ok(), sized);
            assert_eq!(
                authority
                    .validate_value_ty_with_budget(&param, &budget)
                    .is_ok(),
                sized
            );
            authority
                .validate_value_ty(&Ty::Ref {
                    kind: BorrowKind::Shared,
                    referent: Arc::new(param),
                })
                .unwrap();
        }
    }

    #[test]
    fn budget_telemetry_preserves_first_failure_and_external_callsite() {
        let budget = PlanBudget::new(PlanLimits {
            work: 2,
            ..PlanLimits::default()
        });
        budget.charge_work(1).unwrap();
        let line = line!() + 1;
        assert!(budget.charge_work(2).is_err());
        let first = budget.first_resource_failure().unwrap();
        assert_eq!(first.kind, ResourceBudgetKind::Work);
        assert_eq!((first.requested, first.used, first.limit), (2, 1, 2));
        assert_eq!((first.file, first.line), (file!(), line));
        assert!(!first.arithmetic_overflow);
        assert_eq!(budget.usage_snapshot().work, 1);
        // Diagnostic construction and other shared users cannot replace it.
        assert!(budget.clone().charge_diagnostic_bytes(usize::MAX).is_err());
        assert!(budget.check_depth(usize::MAX).is_err());
        assert_eq!(budget.first_resource_failure(), Some(first));
    }

    #[test]
    fn budget_telemetry_graph_task_preserves_external_callsite() {
        struct Probe(PlanBudget);
        impl crate::Provider for Probe {
            fn outputs(&self, _: &crate::Key) -> usize {
                1
            }
            fn start(&self, cx: crate::Context, _: crate::Key) -> crate::TaskFuture {
                let budget = self.0.clone();
                Box::pin(async move {
                    budget.charge_work(1).map_err(crate::Error::new)?;
                    let line = line!() + 1;
                    assert!(budget.charge_work(2).is_err());
                    cx.publish(0, u64::from(line))
                })
            }
        }
        for workers in [1, 8] {
            let budget = PlanBudget::new(PlanLimits {
                work: 2,
                ..PlanLimits::default()
            });
            let engine = crate::Engine::new(Probe(budget.clone()));
            let demand = engine.request(crate::Key::new(42, vec![7]));
            let report = engine.run(workers).unwrap();
            assert_eq!(report.failures, 0);
            let failure = budget.first_resource_failure().unwrap();
            assert_eq!(
                (failure.file, u64::from(failure.line)),
                (file!(), demand.result(0).unwrap())
            );
            assert_eq!((failure.requested, failure.used, failure.limit), (2, 1, 2));
            assert!(failure.graph_id.is_some() && failure.task_id.is_some());
        }
    }

    #[test]
    fn budget_telemetry_shared_workers_preserve_one_first_failure() {
        let budget = PlanBudget::new(PlanLimits {
            work: 0,
            ..PlanLimits::default()
        });
        std::thread::scope(|scope| {
            for requested in 1..=4 {
                let budget = &budget;
                scope.spawn(move || {
                    assert!(budget.charge_work(requested).is_err());
                    let first = budget.first_resource_failure().unwrap();
                    assert!((1..=4).contains(&first.requested));
                    assert_eq!(first.kind, ResourceBudgetKind::Work);
                });
            }
        });
        let first = budget.first_resource_failure().unwrap();
        assert_eq!((first.used, first.limit), (0, 0));
        assert_eq!(budget.usage_snapshot().work, 0);
        assert!(budget.charge_nodes(usize::MAX).is_err());
        assert_eq!(budget.first_resource_failure(), Some(first));
    }

    #[test]
    fn budget_telemetry_classifies_every_plan_counter_and_overflow() {
        let budget = PlanBudget::new(PlanLimits {
            nodes: 1,
            ..PlanLimits::default()
        });
        budget.charge_nodes(1).unwrap();
        assert!(budget.charge_nodes(usize::MAX).is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ResourceBudgetKind::Nodes);
        assert_eq!(
            (failure.requested, failure.used, failure.limit),
            (usize::MAX, 1, 1)
        );
        assert!(failure.arithmetic_overflow);
        assert_eq!(budget.usage_snapshot().nodes, 1);

        let budget = PlanBudget::new(PlanLimits {
            applications: 0,
            ..PlanLimits::default()
        });
        assert!(budget.charge_application().is_err());
        assert_eq!(
            budget.first_resource_failure().unwrap().kind,
            ResourceBudgetKind::Applications
        );
        let budget = PlanBudget::default();
        for _ in 0..4096 {
            budget.charge_obligation().unwrap();
        }
        assert!(budget.charge_obligation().is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ResourceBudgetKind::Obligations);
        assert_eq!(
            (failure.requested, failure.used, failure.limit),
            (1, 4096, 4096)
        );
        let budget = PlanBudget::default();
        assert!(budget.charge_diagnostic_bytes(65537).is_err());
        assert_eq!(
            budget.first_resource_failure().unwrap().kind,
            ResourceBudgetKind::DiagnosticBytes
        );
        let budget = PlanBudget::new(PlanLimits {
            depth: 3,
            ..PlanLimits::default()
        });
        budget.check_depth(3).unwrap();
        let line = line!() + 1;
        assert!(budget.check_depth(4).is_err());
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ResourceBudgetKind::Depth);
        assert_eq!((failure.requested, failure.used, failure.limit), (4, 0, 3));
        assert_eq!(failure.line, line);
    }

    #[test]
    fn retained_receipt_is_paid_once_and_never_refunds_direct_admission() {
        let budget = PlanBudget::new(PlanLimits {
            bytes: 11,
            ..PlanLimits::default()
        });
        let ledger = Arc::downgrade(&budget.usage);
        let authority = budget.authority().clone();
        assert!(authority.same_authority(budget.clone().authority()));
        assert!(!authority.same_authority(PlanBudget::default().authority()));
        assert_eq!(authority.limits().bytes, 11);
        let scratch = budget.reserve_bytes(3).unwrap();
        let receipt = budget.reserve_bytes(8).unwrap().retain().unwrap();
        assert_eq!(receipt.bytes(), 8);
        assert!(receipt.authority().same_authority(&authority));
        let usage = budget.usage_snapshot();
        assert_eq!(usage.bytes, 11);
        assert_eq!(usage.reservations.current_bytes, 11);
        assert_eq!(usage.reservations.current_reservations, 1);
        assert_eq!(usage.reservations.retentions, 1);
        assert_eq!(usage.reservations.cumulative_bytes, 11);
        let clone = receipt.clone();
        drop(receipt);
        assert_eq!(budget.usage_snapshot().reservations, usage.reservations);
        assert!(budget.reserve_bytes(1).is_err());
        drop(scratch);
        assert_eq!(budget.usage_snapshot().bytes, 8);
        let last = budget.reserve_bytes(3).unwrap();
        assert!(budget.reserve_bytes(usize::MAX).is_err());
        drop(last);
        drop(clone);
        assert_eq!(budget.usage_snapshot().bytes, 8);
        let survivor = budget.reserve_bytes(0).unwrap().retain().unwrap();
        drop(budget);
        // Neither authority nor receipt keeps the accounting ledger alive.
        assert!(ledger.upgrade().is_none());
        assert_eq!(survivor.bytes(), 0);
        assert!(survivor.authority().same_authority(&authority));
    }

    #[test]
    fn retained_receipt_preserves_originating_task_byte_admission() {
        use crate::engine::{Context, Engine, Key, Provider, TaskFuture};
        struct Receipts;
        impl Provider for Receipts {
            fn outputs(&self, _: &Key) -> usize {
                1
            }
            fn start(&self, cx: Context, _: Key) -> TaskFuture {
                Box::pin(async move {
                    let budget = PlanBudget::new(PlanLimits {
                        bytes: 11,
                        ..PlanLimits::default()
                    });
                    let scratch = budget.reserve_bytes(3).unwrap();
                    let receipt = budget.reserve_bytes(8).unwrap().retain().unwrap();
                    let clone = receipt.clone();
                    drop(receipt);
                    assert!(budget.reserve_bytes(1).is_err());
                    drop(scratch);
                    let last = budget.reserve_bytes(3).unwrap();
                    assert!(budget.reserve_bytes(1).is_err());
                    drop(last);
                    let meter = crate::engine::MeterHandle::current().unwrap();
                    assert_eq!(
                        meter
                            .meter()
                            .bytes
                            .load(std::sync::atomic::Ordering::Relaxed),
                        8
                    );
                    drop(budget);
                    cx.publish_fact(0, clone)?;
                    assert_eq!(
                        meter
                            .meter()
                            .bytes
                            .load(std::sync::atomic::Ordering::Relaxed),
                        8
                    );
                    Ok(())
                })
            }
        }
        let engine = Engine::new(Receipts);
        let root = engine.request(Key::new(0, vec![]));
        let report = engine.run(1).unwrap();
        assert_eq!(report.failures, 0);
        assert!(root.result(0).is_ok());
    }

    #[test]
    fn retained_receipt_promotion_rejects_poison_without_fabricating_evidence() {
        let budget = PlanBudget::default();
        let reservation = budget.reserve_bytes(7).unwrap();
        let usage = budget.usage.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = usage.lock().unwrap();
                panic!("poison test ledger");
            })
            .join()
            .is_err()
        );
        assert!(reservation.retain().is_err());
        let snapshot = budget.usage_snapshot();
        assert!(snapshot.poisoned);
        assert_eq!(snapshot.bytes, 7);
        assert_eq!(snapshot.reservations.retentions, 0);
    }

    #[test]
    fn budget_telemetry_tracks_reservations_release_and_retention() {
        let budget = PlanBudget::new(PlanLimits {
            bytes: 10,
            ..PlanLimits::default()
        });
        let first = budget.reserve_bytes(4).unwrap();
        let second = budget.clone().reserve_bytes(3).unwrap();
        budget.retain_bytes(2).unwrap();
        let usage = budget.usage_snapshot();
        assert_eq!(usage.bytes, 9);
        assert_eq!(
            usage.reservations,
            ReservationAccounting {
                reservations: 2,
                current_reservations: 2,
                peak_reservations: 2,
                retentions: 1,
                current_bytes: 9,
                peak_bytes: 9,
                cumulative_bytes: 9,
                counters_saturated: false,
            }
        );
        assert!(budget.reserve_bytes(2).is_err());
        assert_eq!(budget.usage_snapshot().reservations, usage.reservations);
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ResourceBudgetKind::Bytes);
        assert_eq!((failure.requested, failure.used, failure.limit), (2, 9, 10));
        drop(first);
        drop(second);
        let usage = budget.usage_snapshot();
        assert_eq!((usage.bytes, usage.reservations.current_bytes), (2, 2));
        assert_eq!(usage.reservations.current_reservations, 0);
        assert_eq!(
            (
                usage.reservations.peak_bytes,
                usage.reservations.cumulative_bytes
            ),
            (9, 9)
        );
        assert_eq!(usage.first_failure, Some(failure));
    }

    #[test]
    fn budget_telemetry_zero_reservations_and_counter_saturation_are_explicit() {
        let budget = PlanBudget::new(PlanLimits {
            bytes: 0,
            ..PlanLimits::default()
        });
        let reservation = budget.reserve_bytes(0).unwrap();
        assert_eq!(budget.usage_snapshot().reservations.current_reservations, 1);
        drop(reservation);
        assert_eq!(budget.usage_snapshot().reservations.current_reservations, 0);
        assert!(budget.first_resource_failure().is_none());

        let mut accounting = ReservationAccounting {
            cumulative_bytes: usize::MAX,
            ..ReservationAccounting::default()
        };
        accounting.record(1, false);
        assert!(accounting.counters_saturated);
        assert_eq!(accounting.cumulative_bytes, usize::MAX);
        assert_eq!(accounting.current_bytes, 1);
    }

    #[test]
    fn budget_telemetry_json_is_bounded_escaped_and_non_mutating() {
        let budget = PlanBudget::new(PlanLimits {
            work: 0,
            ..PlanLimits::default()
        });
        assert!(budget.charge_work(1).is_err());
        let snapshot = budget.usage_snapshot();
        assert!(snapshot.to_json().contains("\"kind\":\"work\""));
        assert_eq!(snapshot, budget.usage_snapshot());
        let mut failure = snapshot.first_failure.unwrap();
        failure.file = "a\"b\\c\n\u{0}\u{1f}é.rs";
        let json = failure.to_json();
        assert!(json.contains("a\\\"b\\\\c\\u000a\\u0000\\u001fé.rs"));
        assert!(json.contains("\"file_truncated\":false"));
        failure.file = concat!(
            "012345678901234567890123456789012345678901234567890123456789012345",
            "012345678901234567890123456789012345678901234567890123456789012345",
            "012345678901234567890123456789012345678901234567890123456789012345",
            "012345678901234567890123456789012345678901234567890123456789012345",
            "DO_NOT_SERIALIZE"
        );
        let json = failure.to_json();
        assert!(json.contains("\"file_truncated\":true"));
        assert!(!json.contains("DO_NOT_SERIALIZE"));
        assert!(json.len() < 2048);
        struct Reject;
        impl std::fmt::Write for Reject {
            fn write_str(&mut self, _: &str) -> std::fmt::Result {
                Err(std::fmt::Error)
            }
        }
        assert!(snapshot.write_json(&mut Reject).is_err());
        assert_eq!(budget.first_resource_failure(), snapshot.first_failure);
    }

    #[test]
    fn budget_telemetry_poisoned_ledger_retains_available_failure_evidence() {
        let budget = PlanBudget::new(PlanLimits {
            work: 0,
            ..PlanLimits::default()
        });
        assert!(budget.charge_work(1).is_err());
        let first = budget.first_resource_failure();
        let ledger = budget.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = ledger.usage.lock().unwrap();
                panic!("deliberately poison the test ledger");
            })
            .join()
            .is_err()
        );
        assert!(budget.usage_snapshot().poisoned);
        assert_eq!(budget.first_resource_failure(), first);
        assert_eq!(
            budget.charge_work(0).unwrap_err(),
            "generic planning budget poisoned"
        );
    }

    #[test]
    fn generic_carrier_layouts_preserve_component_identity_and_shape_limits() {
        let budget = PlanBudget::default();
        let scope = Arc::new(
            ParamEnv::new(
                BinderId {
                    declaration: test_decl(),
                },
                1,
            )
            .unwrap(),
        );
        let env = GenericEnv::new(std::slice::from_ref(&scope), &budget).unwrap();
        let parameter = env.ordered_params().next().unwrap();
        let reference = |kind, ty| Ty::Ref {
            kind,
            referent: Arc::new(ty),
        };
        let nested = reference(
            BorrowKind::Mut,
            reference(BorrowKind::Mut, Ty::Param(parameter)),
        );
        FactsCtx::Universal(&env)
            .validate_ty_with_budget(&nested, &budget)
            .unwrap();
        let family = CarrierLocation {
            path: CarrierPath(Arc::from([])),
            channel: CarrierChannel::ParamTail {
                parameter,
                prefix: 2,
            },
        };
        assert!(
            carrier_locations(&nested, &budget)
                .unwrap()
                .contains(&family)
        );
        assert_eq!(
            carrier_at(&nested, &family, &budget).unwrap(),
            &Ty::Param(parameter)
        );
        let components = Ty::Tuple(Arc::from([
            reference(BorrowKind::Shared, Ty::Bool),
            Ty::Array {
                element: Arc::new(reference(BorrowKind::Shared, Ty::Int(IntTy::I32))),
                len: 2,
            },
        ]));
        let locations = carrier_locations(&components, &budget).unwrap();
        assert_eq!(locations.len(), 3);
        assert_ne!(locations[1], locations[2]);
        for location in locations.iter() {
            assert!(matches!(
                carrier_at(&components, location, &budget).unwrap(),
                Ty::Ref { .. }
            ));
        }
        assert!(validate_root_reference_actual(&components, &budget).is_err());
        assert!(
            validate_root_reference_actual(&reference(BorrowKind::Mut, Ty::Bool), &budget).is_ok()
        );
        let projected = Ty::Tuple(Arc::from([reference(BorrowKind::Shared, Ty::Bool)]));
        assert_eq!(
            projected_ty(&projected, &[Projection::Field(0), Projection::Deref]).unwrap(),
            &Ty::Bool
        );
        let oversized = Ty::Tuple(Arc::from([
            Ty::Array {
                element: Arc::new(reference(BorrowKind::Shared, Ty::Bool)),
                len: MAX_ARRAY_LEN,
            },
            reference(BorrowKind::Shared, Ty::Bool),
        ]));
        assert!(carrier_locations(&oversized, &budget).is_err());
    }

    #[test]
    fn generic_carrier_original_call_evidence_rejects_missing_forged_and_trait_method_rows() {
        use crate::compiler::frontend;
        let budget = PlanBudget::default();
        let program =
            frontend::parse("fn id<T:Copy>(x:T)->T{x} fn main(){let x=7;let r=id::<&i32>(&x);}")
                .unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let source =
            frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
        let facts = plan
            .instance_facts(&CallTarget {
                declaration,
                type_args: Arc::from([]),
            })
            .unwrap();
        validate_call_evidence(&source, &facts, &budget).unwrap();
        let call = source
            .call_evidence
            .iter()
            .find(|evidence| !evidence.type_actual_regions.is_empty())
            .unwrap()
            .call;
        let mut missing = source.clone();
        missing.call_evidence = Arc::from([]);
        assert!(
            validate_call_evidence(&missing, &facts, &budget)
                .unwrap_err()
                .contains("missing original-stage")
        );
        let mut forged = source.clone();
        let mut rows = source.call_evidence.to_vec();
        let row = rows.iter_mut().find(|row| row.call == call).unwrap();
        let mut arguments = row.arguments.to_vec();
        arguments[0].expectation = CallExpectation::NoFirmReference;
        row.arguments = arguments.into();
        forged.call_evidence = rows.into();
        assert!(
            validate_call_evidence(&forged, &facts, &budget)
                .unwrap_err()
                .contains("firm argument")
        );
        // Completeness must run for ordinary trait-method arguments even before
        // canonical target resolution rejects this deliberately raw metadata.
        let ExprKind::Call { target, .. } = &mut missing.exprs[call.0].kind else {
            panic!("call fixture");
        };
        *target = Callee::TraitItem(TraitCall {
            trait_ref: TraitRef {
                declaration,
                self_ty: Ty::Bool,
                args: Arc::from([]),
            },
            item: declaration,
            method_args: Arc::from([Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(Ty::Bool),
            }]),
        });
        assert!(
            validate_call_evidence(&missing, &facts, &budget)
                .unwrap_err()
                .contains("missing original-stage")
        );
    }

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
        // Internal structural types are admitted independently of ordinary
        // generic-actual syntax. This zero-length value has no carrier slots.
        assert!(carrier_locations(&nested, &budget).unwrap().is_empty());
        assert!(validate_root_reference_actual(&nested, &budget).is_err());
        assert_eq!(
            fixture_derive_return_contract(
                &[nested],
                &Ty::Unit,
                &RefContract {
                    input_regions: fixture_input_regions(Arc::from([None])),
                    ..RefContract::default()
                },
                &budget
            )
            .unwrap(),
            ReturnContract::NoReference
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

        let projection = AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: test_decl(),
                self_ty: params[0].clone(),
                args: Arc::from([params[1].clone(), params[0].clone()]),
            },
            item: test_decl(),
        };
        let substituted = substitute_ty(
            &Ty::AssociatedType(Arc::new(projection.clone())),
            &env,
            &actuals,
            FactsCtx::Universal(&env),
            &mut NoNominals,
            &budget,
        )
        .unwrap();
        let Ty::AssociatedType(actual) = substituted else {
            panic!("raw projection was erased");
        };
        assert_eq!(actual.item, projection.item);
        assert_eq!(
            actual.trait_ref.declaration,
            projection.trait_ref.declaration
        );
        assert_eq!(actual.trait_ref.self_ty, params[1]);
        assert_eq!(
            &*actual.trait_ref.args,
            &[params[0].clone(), params[1].clone()]
        );
    }

    fn projection_fixture() -> Ty {
        Ty::AssociatedType(Arc::new(AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: test_decl(),
                self_ty: Ty::Bool,
                args: Arc::from([Ty::Int(IntTy::U128)]),
            },
            item: test_decl(),
        }))
    }

    #[test]
    fn equality_predicate_metadata_includes_complete_application_and_rhs() {
        let projection = AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: test_decl(),
                self_ty: Ty::Bool,
                args: Arc::from([Ty::Int(IntTy::U128), Ty::Unit]),
            },
            item: test_decl(),
        };
        let equality = Predicate::ProjectionEq {
            projection: projection.clone(),
            ty: Ty::Never,
        };
        assert_eq!(equality.trait_ref(), &projection.trait_ref);
        assert_eq!(
            equality.metadata_types().cloned().collect::<Vec<_>>(),
            [Ty::Bool, Ty::Int(IntTy::U128), Ty::Unit, Ty::Never]
        );
        let membership = Predicate::Implements(projection.trait_ref);
        assert_eq!(
            membership.metadata_types().cloned().collect::<Vec<_>>(),
            [Ty::Bool, Ty::Int(IntTy::U128), Ty::Unit]
        );
    }

    #[test]
    fn equality_predicate_keys_preserve_kind_item_application_and_rhs() {
        let projection = AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: test_decl(),
                self_ty: Ty::Bool,
                args: Arc::from([Ty::Int(IntTy::U128), Ty::Unit]),
            },
            item: test_decl(),
        };
        let equality = Predicate::ProjectionEq {
            projection: projection.clone(),
            ty: Ty::Int(IntTy::I128),
        };
        let mut keys = std::collections::HashSet::from([equality.clone()]);
        assert!(!keys.insert(equality.clone()));
        assert!(keys.insert(Predicate::Implements(projection.trait_ref.clone())));
        assert!(keys.insert(Predicate::ProjectionEq {
            projection: projection.clone(),
            ty: Ty::Bool,
        }));
        for changed in 0..4 {
            let mut other = projection.clone();
            match changed {
                0 => other.item = test_decl(),
                1 => other.trait_ref.declaration = test_decl(),
                2 => other.trait_ref.self_ty = Ty::Unit,
                3 => other.trait_ref.args = Arc::from([Ty::Unit, Ty::Int(IntTy::U128)]),
                _ => unreachable!(),
            }
            assert!(keys.insert(Predicate::ProjectionEq {
                projection: other,
                ty: Ty::Int(IntTy::I128),
            }));
        }
        assert_eq!(keys.len(), 7);
        assert!(keys.contains(&equality));
    }

    #[test]
    fn equality_metadata_walk_reaches_invalid_rhs_before_capability_checks() {
        let budget = PlanBudget::default();
        let predicate = Predicate::ProjectionEq {
            projection: AssociatedTypeProjection {
                trait_ref: TraitRef {
                    declaration: test_decl(),
                    self_ty: projection_fixture(),
                    args: Arc::from([Ty::Unit]),
                },
                item: test_decl(),
            },
            ty: Ty::Array {
                element: Arc::new(Ty::Param(ParamId {
                    owner: BinderId {
                        declaration: test_decl(),
                    },
                    index: 0,
                })),
                len: 0,
            },
        };
        let error = predicate
            .metadata_types()
            .try_for_each(|ty| FactsCtx::Concrete.validate_ty_metadata_with_budget(ty, &budget))
            .unwrap_err();
        assert!(
            error.contains("outside its authoritative binder"),
            "{error}"
        );
        assert!(!error.contains("unsupported"), "{error}");

        let poisoned = PlanBudget::default();
        assert!(poisoned.check_depth(poisoned.limits().depth + 1).is_err());
        assert!(
            predicate
                .metadata_types()
                .try_for_each(|ty| {
                    FactsCtx::Concrete.validate_ty_metadata_with_budget(ty, &poisoned)
                })
                .is_err()
        );
        assert!(poisoned.has_resource_failure());
    }

    #[test]
    fn projection_identity_is_not_structural_schema_equality() {
        let budget = PlanBudget::default();
        let original = projection_fixture();
        let Ty::AssociatedType(value) = &original else {
            unreachable!()
        };
        let copied = Ty::AssociatedType(Arc::new((**value).clone()));
        assert!(identical_type(&original, &original.clone(), &budget).unwrap());
        assert!(!identical_type(&original, &copied, &budget).unwrap());
        assert!(ref_types_equal(&original, &copied, &budget, 0).unwrap());
        let mut different = (**value).clone();
        different.trait_ref.args = Arc::from([Ty::Int(IntTy::I128)]);
        assert!(
            !ref_types_equal(
                &original,
                &Ty::AssociatedType(Arc::new(different)),
                &budget,
                0
            )
            .unwrap()
        );
    }

    #[test]
    fn residual_projection_has_no_copy_carrier_place_or_reference_shape() {
        let projection = projection_fixture();
        let zero = Ty::Array {
            element: Arc::new(projection.clone()),
            len: 0,
        };
        for ty in [
            projection.clone(),
            zero.clone(),
            Ty::Tuple(Arc::from([zero])),
        ] {
            let budget = PlanBudget::default();
            assert!(!ty.is_copy());
            assert!(
                FactsCtx::Concrete
                    .validate_ty_metadata_with_budget(&ty, &budget)
                    .is_ok()
            );
            assert!(
                FactsCtx::Concrete
                    .validate_ty_with_budget(&ty, &budget)
                    .is_err()
            );
            assert!(carrier_locations(&ty, &budget).is_err());
            assert!(reject_references(&ty, &budget).is_err());
            assert!(projected_ty(&ty, &[]).is_err());
            assert!(root_reference_layers(&ty).is_err());
            assert!(reference_layers(&ty).is_err());
            assert!(slice_boundary_walk(&ty, true, Some(&budget), 0, &mut 0).is_err());
        }
        let budget = PlanBudget::default();
        assert!(
            carrier_at(
                &projection,
                &CarrierLocation {
                    path: CarrierPath::default(),
                    channel: CarrierChannel::ParamTail {
                        prefix: 0,
                        parameter: ParamId {
                            owner: BinderId {
                                declaration: test_decl()
                            },
                            index: 0
                        }
                    },
                },
                &budget
            )
            .is_err()
        );
    }

    #[test]
    fn residual_guard_memoizes_shared_dags_and_replays_depth_and_ledger() {
        let mut shared = Ty::Bool;
        for _ in 0..40 {
            shared = Ty::Tuple(Arc::from([shared.clone(), shared]));
        }
        let budget = PlanBudget::default();
        reject_associated_types(&shared, Some(&budget)).unwrap();
        FactsCtx::Concrete
            .validate_ty_metadata_with_budget(&shared, &budget)
            .unwrap();
        assert_eq!(budget.usage_snapshot().bytes, 0);
        let tiny = PlanBudget::new(PlanLimits {
            work: 0,
            ..PlanLimits::default()
        });
        assert!(reject_associated_types(&shared, Some(&tiny)).is_err());
        assert!(tiny.has_resource_failure());
        let poisoned = PlanBudget::default();
        assert!(poisoned.check_depth(poisoned.limits().depth + 1).is_err());
        assert!(reject_associated_types(&Ty::Bool, Some(&poisoned)).is_err());
        assert!(
            fixture()
                .preflight_plan(FactsCtx::Concrete, &poisoned)
                .is_err()
        );

        let mut deep = Ty::Bool;
        for _ in 0..100 {
            deep = Ty::Tuple(Arc::from([deep]));
        }
        let mut later = deep.clone();
        for _ in 0..30 {
            later = Ty::Tuple(Arc::from([later]));
        }
        let root = Ty::Tuple(Arc::from([deep, later]));
        assert!(reject_associated_types(&root, None).is_err());
    }

    #[test]
    fn executable_projection_rejection_waits_for_later_dead_metadata() {
        let mut function = fixture();
        function.result = projection_fixture();
        function.locals[1].ty = Ty::Param(ParamId {
            owner: BinderId {
                declaration: test_decl(),
            },
            index: 0,
        });
        for error in [
            function.validate().unwrap_err(),
            function
                .preflight_plan(FactsCtx::Concrete, &PlanBudget::default())
                .unwrap_err(),
        ] {
            assert!(
                error.contains("outside its authoritative binder"),
                "{error}"
            );
            assert!(!error.contains("unsupported"), "{error}");
        }
        function.locals[1].ty = Ty::Bool;
        function.exprs.push(TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Returns,
            kind: ExprKind::Call {
                target: Callee::Direct(CallTarget {
                    declaration: test_decl(),
                    type_args: Arc::from([Ty::Param(ParamId {
                        owner: BinderId {
                            declaration: test_decl(),
                        },
                        index: 0,
                    })]),
                }),
                name: "dead".into(),
                args: vec![],
                signature: Signature {
                    params: vec![],
                    result: Ty::Unit,
                    ref_contract: Default::default(),
                },
            },
        });
        assert!(
            function
                .validate()
                .unwrap_err()
                .contains("outside its authoritative binder")
        );
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
        FactsCtx::Universal(&env).validate_ty(&hidden).unwrap();
        // Structural facts do not grant ordinary aggregate-reference actual
        // admission, nor invent an element in a zero-length array.
        assert!(
            carrier_locations(&hidden, &PlanBudget::default())
                .unwrap()
                .is_empty()
        );
        assert!(validate_root_reference_actual(&hidden, &PlanBudget::default()).is_err());
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
        function.locals[0].mutable = true;
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
        let tail = ExprId(function.exprs.len());
        function.exprs.push(function.exprs[0].clone());
        let panic = ExprId(function.exprs.len());
        function.exprs.push(TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Panic(Box::new(PanicMessage {
                message: "stop".into(),
                location: "fixture".into(),
            })),
        });
        function.exprs[0].kind = ExprKind::Block(Block {
            statements: vec![Stmt::Expr(panic)],
            tail: Some(tail),
        });
        function.exprs[0].completion = Completion::Never;
        function.validate().unwrap();
        let Stmt::AssignIndexOp { index, .. } = &mut function.body.statements[1] else {
            unreachable!()
        };
        *index = ExprId(usize::MAX);
        assert!(function.validate().is_err());
    }

    fn unused_binary_fixture(op: BinaryOp, left: Ty, right: Ty) -> CheckedFunction {
        let mut function = fixture();
        function.locals = [left.clone(), right.clone()]
            .into_iter()
            .enumerate()
            .map(|(index, ty)| LocalDecl {
                name: format!("operand{index}"),
                ty,
                mutable: false,
            })
            .collect();
        function.params.clear();
        function.exprs = vec![
            TypedExpr {
                ty: left,
                completion: Completion::Returns,
                kind: ExprKind::Local(LocalId(0)),
            },
            TypedExpr {
                ty: right,
                completion: Completion::Returns,
                kind: ExprKind::Local(LocalId(1)),
            },
            TypedExpr {
                ty: Ty::Bool,
                completion: Completion::Returns,
                kind: ExprKind::Binary {
                    op,
                    left: ExprId(0),
                    right: ExprId(1),
                },
            },
        ];
        // None of these rows is reachable from the body.
        function.body = Block {
            statements: vec![],
            tail: None,
        };
        function
    }

    #[test]
    fn equality_validates_unused_rows_exact_domains_results_and_eager_completion() {
        for op in [BinaryOp::Eq, BinaryOp::Ne] {
            for ty in [
                Ty::Unit,
                Ty::Bool,
                Ty::Int(IntTy::I32),
                Ty::Int(IntTy::U64),
                Ty::Float(FloatTy::F32),
                Ty::Float(FloatTy::F64),
            ] {
                let function = unused_binary_fixture(op, ty.clone(), ty);
                function.validate().unwrap();
                let mut wrong_result = function.clone();
                wrong_result.exprs[2].ty = Ty::Int(IntTy::I32);
                assert!(wrong_result.validate().is_err());
                for left in [Completion::Returns, Completion::Never] {
                    for right in [Completion::Returns, Completion::Never] {
                        let mut flow = function.clone();
                        flow.exprs[0].completion = left;
                        flow.exprs[1].completion = right;
                        let expected =
                            if left == Completion::Returns && right == Completion::Returns {
                                Completion::Returns
                            } else {
                                Completion::Never
                            };
                        flow.exprs[2].completion = expected;
                        flow.validate().unwrap();
                        flow.exprs[2].completion = if expected == Completion::Returns {
                            Completion::Never
                        } else {
                            Completion::Returns
                        };
                        assert!(flow.validate().is_err());
                    }
                }
            }
            for (left, right) in [
                (Ty::Int(IntTy::I32), Ty::Unit),
                (Ty::Unit, Ty::Int(IntTy::I32)),
                (Ty::Int(IntTy::I32), Ty::Int(IntTy::U32)),
                (Ty::Int(IntTy::U64), Ty::Int(IntTy::Usize)),
                (Ty::Float(FloatTy::F32), Ty::Float(FloatTy::F64)),
                (Ty::Bool, Ty::Int(IntTy::I32)),
            ] {
                assert!(unused_binary_fixture(op, left, right).validate().is_err());
            }
            for ty in [
                Ty::Never,
                Ty::Tuple(Arc::from([Ty::Bool])),
                Ty::Array {
                    element: Arc::new(Ty::Bool),
                    len: 1,
                },
                Ty::Ref {
                    kind: BorrowKind::Shared,
                    referent: Arc::new(Ty::Bool),
                },
            ] {
                let mut function = unused_binary_fixture(op, ty.clone(), ty);
                function.exprs[0].completion = Completion::Never;
                function.exprs[1].completion = Completion::Never;
                function.exprs[2].completion = Completion::Never;
                assert!(function.validate().is_err());
            }
            let mut bad_child = unused_binary_fixture(op, Ty::Bool, Ty::Bool);
            let ExprKind::Binary { right, .. } = &mut bad_child.exprs[2].kind else {
                unreachable!()
            };
            *right = ExprId(usize::MAX);
            assert!(bad_child.validate().is_err());
        }
    }

    #[test]
    fn logical_validates_unused_rows_and_preserves_short_circuit_never_coercion() {
        for op in [BinaryOp::And, BinaryOp::Or] {
            for left in [Ty::Bool, Ty::Never] {
                for right in [Ty::Bool, Ty::Never] {
                    for left_completion in [Completion::Returns, Completion::Never] {
                        for right_completion in [Completion::Returns, Completion::Never] {
                            let mut function =
                                unused_binary_fixture(op, left.clone(), right.clone());
                            function.exprs[0].completion = left_completion;
                            function.exprs[1].completion = right_completion;
                            function.exprs[2].completion = left_completion;
                            let admitted = (left != Ty::Never
                                || left_completion == Completion::Never)
                                && (right != Ty::Never || right_completion == Completion::Never);
                            assert_eq!(function.validate().is_ok(), admitted);
                            if admitted {
                                function.exprs[2].completion =
                                    if left_completion == Completion::Returns {
                                        Completion::Never
                                    } else {
                                        Completion::Returns
                                    };
                                assert!(function.validate().is_err());
                            }
                        }
                    }
                }
            }
            for ty in [Ty::Int(IntTy::I32), Ty::Unit, Ty::Float(FloatTy::F32)] {
                assert!(
                    unused_binary_fixture(op, ty.clone(), ty)
                        .validate()
                        .is_err()
                );
            }
            let mut wrong_result = unused_binary_fixture(op, Ty::Bool, Ty::Bool);
            wrong_result.exprs[2].ty = Ty::Unit;
            assert!(wrong_result.validate().is_err());
        }
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
            IntTy::I128,
            IntTy::U128,
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
                        let saved = function.exprs[endpoint].clone();
                        let tail = ExprId(function.exprs.len());
                        function.exprs.push(saved.clone());
                        let panic = ExprId(function.exprs.len());
                        function.exprs.push(TypedExpr {
                            ty: Ty::Never,
                            completion: Completion::Never,
                            kind: ExprKind::Panic(Box::new(PanicMessage {
                                message: "stop".into(),
                                location: "fixture".into(),
                            })),
                        });
                        function.exprs[endpoint].kind = ExprKind::Block(Block {
                            statements: vec![Stmt::Expr(panic)],
                            tail: Some(tail),
                        });
                        function.exprs[endpoint].completion = Completion::Never;
                        assert!(function.validate().unwrap_err().contains("completion"));
                        function.exprs[2].completion = Completion::Never;
                        function.validate().unwrap();
                        function.exprs[endpoint] = saved;
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
    fn float_constants_and_operations_validate_exact_bits_types_and_completion() {
        for float in [FloatValue::f32(-0.0), FloatValue::f64(f64::NAN)] {
            let mut function = fixture();
            function.exprs[0] = TypedExpr {
                ty: Ty::Float(float.ty),
                completion: Completion::Returns,
                kind: ExprKind::Float(float),
            };
            function.validate().unwrap();
            function.exprs.push(TypedExpr {
                ty: Ty::Float(float.ty),
                completion: Completion::Returns,
                kind: ExprKind::Unary {
                    op: UnaryOp::Neg,
                    value: ExprId(0),
                },
            });
            function.validate().unwrap();
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
                let comparison = matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                );
                let mut valid = function.clone();
                valid.exprs.push(TypedExpr {
                    ty: if comparison {
                        Ty::Bool
                    } else {
                        Ty::Float(float.ty)
                    },
                    completion: Completion::Returns,
                    kind: ExprKind::Binary {
                        op,
                        left: ExprId(0),
                        right: ExprId(1),
                    },
                });
                valid.validate().unwrap();
                let mut invalid = valid.clone();
                invalid.exprs[2].completion = Completion::Never;
                assert!(invalid.validate().is_err());
                invalid = valid.clone();
                invalid.exprs[2].ty = Ty::Int(IntTy::I32);
                assert!(invalid.validate().is_err());
            }
            function.exprs[1].kind = ExprKind::Unary {
                op: UnaryOp::Not,
                value: ExprId(0),
            };
            assert!(function.validate().is_err());
            function.exprs[1].kind = ExprKind::Binary {
                op: BinaryOp::BitAnd,
                left: ExprId(0),
                right: ExprId(0),
            };
            assert!(function.validate().is_err());
        }
        assert!(
            FloatValue {
                ty: FloatTy::F32,
                bits: 1u64 << 32
            }
            .validate()
            .is_err()
        );
        assert_ne!(FloatValue::f32(0.0), FloatValue::f32(-0.0));
        assert_eq!(FloatValue::f64(f64::NAN), FloatValue::f64(f64::NAN));
    }

    #[test]
    fn character_casts_validate_exact_endpoints_dead_constants_and_completion() {
        let types = [
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
        ];
        for to in types {
            let mut function = fixture();
            function.exprs[0] = TypedExpr {
                ty: Ty::Char,
                completion: Completion::Returns,
                kind: ExprKind::Char(CharValue { value: 0x10ffff }),
            };
            function.exprs.push(TypedExpr {
                ty: Ty::Int(to),
                completion: Completion::Returns,
                kind: ExprKind::CharToInt {
                    value: ExprId(0),
                    to,
                },
            });
            function.validate().unwrap();
            for scalar in [0xd800, 0xdfff, 0x110000, u32::MAX] {
                let mut invalid = function.clone();
                invalid.exprs[0].kind = ExprKind::Char(CharValue { value: scalar });
                assert!(invalid.validate().is_err());
            }
            let mut invalid = function.clone();
            invalid.exprs[1].ty = Ty::Char;
            assert!(
                invalid
                    .validate()
                    .unwrap_err()
                    .contains("result type mismatch")
            );
            invalid = function.clone();
            invalid.exprs[1].completion = Completion::Never;
            assert!(
                invalid
                    .validate()
                    .unwrap_err()
                    .contains("completion mismatch")
            );
            invalid = function.clone();
            invalid.exprs[1].kind = ExprKind::CharToInt {
                value: ExprId(99),
                to,
            };
            assert!(
                invalid
                    .validate()
                    .unwrap_err()
                    .contains("invalid expression 99")
            );
            function.exprs[0].kind = ExprKind::Block(Block {
                statements: vec![Stmt::Return(None)],
                tail: None,
            });
            function.exprs[0].completion = Completion::Never;
            function.exprs[1].completion = Completion::Never;
            function.validate().unwrap();
        }
        let mut function = fixture();
        function.exprs[0] = TypedExpr {
            ty: Ty::Int(IntTy::U8),
            completion: Completion::Returns,
            kind: ExprKind::Int(IntValue {
                ty: IntTy::U8,
                bits: 255,
            }),
        };
        function.exprs.push(TypedExpr {
            ty: Ty::Char,
            completion: Completion::Returns,
            kind: ExprKind::U8ToChar { value: ExprId(0) },
        });
        function.validate().unwrap();
        for source in types.into_iter().map(Ty::Int).chain([
            Ty::Bool,
            Ty::Char,
            Ty::Unit,
            Ty::Never,
            Ty::Float(FloatTy::F32),
            Ty::Float(FloatTy::F64),
            Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(Ty::Int(IntTy::U8)),
            },
        ]) {
            if source == Ty::Int(IntTy::U8) {
                continue;
            }
            let mut invalid = function.clone();
            invalid.exprs[0].ty = source;
            assert!(invalid.validate().is_err());
        }
        let mut invalid = function.clone();
        invalid.exprs[0].kind = ExprKind::Int(IntValue {
            ty: IntTy::U8,
            bits: 256,
        });
        assert!(invalid.validate().unwrap_err().contains("noncanonical"));
        invalid = function.clone();
        invalid.exprs[0].completion = Completion::Never;
        invalid.exprs[1].completion = Completion::Never;
        assert!(invalid.validate().is_err());
        invalid = function.clone();
        invalid.exprs[1].ty = Ty::Int(IntTy::U8);
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("result type mismatch")
        );
        invalid = function.clone();
        invalid.exprs[1].completion = Completion::Never;
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("completion mismatch")
        );
        invalid = function.clone();
        invalid.exprs[1].kind = ExprKind::U8ToChar { value: ExprId(99) };
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        function.exprs[0].kind = ExprKind::Block(Block {
            statements: vec![Stmt::Return(None)],
            tail: None,
        });
        function.exprs[0].completion = Completion::Never;
        function.exprs[1].completion = Completion::Never;
        function.validate().unwrap();
    }

    #[test]
    fn leading_zeros_validate_unused_operands_results_and_completion() {
        let mut function = fixture();
        function.exprs.push(TypedExpr {
            ty: Ty::Int(IntTy::U32),
            completion: Completion::Returns,
            kind: ExprKind::LeadingZeros { value: ExprId(0) },
        });
        // The operation is not referenced by the body. Its operand is still
        // authoritative, independently of its declared u32 result.
        function.validate().unwrap();
        for source in [
            Ty::Bool,
            Ty::Char,
            Ty::Float(FloatTy::F32),
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
        for result in [Ty::Int(IntTy::I32), Ty::Int(IntTy::U64), Ty::Never] {
            let mut invalid = function.clone();
            invalid.exprs[1].ty = result;
            assert!(
                invalid
                    .validate()
                    .unwrap_err()
                    .contains("result type mismatch")
            );
        }
        let mut invalid = function.clone();
        invalid.exprs[1].kind = ExprKind::LeadingZeros { value: ExprId(99) };
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("invalid expression 99")
        );
        invalid = function.clone();
        invalid.exprs[1].completion = Completion::Never;
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .contains("completion mismatch")
        );
        function.exprs[0].kind = ExprKind::Block(Block {
            statements: vec![Stmt::Return(None)],
            tail: None,
        });
        function.exprs[0].completion = Completion::Never;
        function.exprs[1].completion = Completion::Never;
        function.validate().unwrap();
        function.exprs[1].completion = Completion::Returns;
        assert!(
            function
                .validate()
                .unwrap_err()
                .contains("completion mismatch")
        );
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
            bits: 1u128 << 32,
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
    fn wide_integer_hashes_and_patterns_retain_both_limbs() {
        #[derive(Default)]
        struct Bytes(Vec<u8>);
        impl Hasher for Bytes {
            fn finish(&self) -> u64 {
                0
            }
            fn write(&mut self, bytes: &[u8]) {
                self.0.extend_from_slice(bytes);
            }
        }
        for ty in [IntTy::I128, IntTy::U128] {
            let low = IntValue { ty, bits: 7 };
            let high = IntValue {
                ty,
                bits: (1_u128 << 127) | 7,
            };
            let mut low_hash = Bytes::default();
            let mut high_hash = Bytes::default();
            low.hash(&mut low_hash);
            high.hash(&mut high_hash);
            assert_ne!(low_hash.0, high_hash.0);
            high.validate().unwrap();
            let mut function = fixture();
            function.exprs[0].ty = Ty::Int(ty);
            function.exprs[0].kind = ExprKind::Int(high);
            function.validate().unwrap();
            function
                .validate_match(
                    ExprId(0),
                    &[MatchArm {
                        guard: None,
                        pattern: MatchPattern::Int(high),
                        body: ExprId(0),
                    }],
                    &Default::default(),
                )
                .unwrap();
        }
        for ty in [IntTy::I64, IntTy::U64, IntTy::Usize] {
            assert!(
                IntValue {
                    ty,
                    bits: 1_u128 << 64
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn scalar_range_decisions_reject_reversed_mistyped_and_invalid_endpoints() {
        let decide = |domain: Ty, kind: PatternTestKind| {
            let mut function = fixture();
            function.exprs[0].ty = domain.clone();
            function.exprs[0].kind = match &domain {
                Ty::Char => ExprKind::Char(CharValue { value: 97 }),
                Ty::Int(ty) => ExprKind::Int(IntValue { ty: *ty, bits: 0 }),
                _ => unreachable!(),
            };
            function.validate().unwrap();
            let arms = [
                MatchArm {
                    guard: None,
                    pattern: MatchPattern::Decision {
                        ty: domain,
                        tests: vec![PatternTest {
                            projection: vec![],
                            kind,
                        }],
                        bindings: vec![],
                    },
                    body: ExprId(0),
                },
                MatchArm {
                    guard: None,
                    pattern: MatchPattern::Wildcard,
                    body: ExprId(0),
                },
            ];
            function.validate_match(ExprId(0), &arms, &Default::default())
        };
        let ch = |value| CharValue { value };
        let i8v = |value: i8| IntValue {
            ty: IntTy::I8,
            bits: value as u8 as u128,
        };
        decide(Ty::Char, PatternTestKind::Char(ch(97))).unwrap();
        decide(
            Ty::Char,
            PatternTestKind::CharRange {
                lo: ch(97),
                hi: ch(122),
            },
        )
        .unwrap();
        decide(
            Ty::Int(IntTy::I8),
            PatternTestKind::IntRange {
                lo: i8v(-128),
                hi: i8v(-1),
            },
        )
        .unwrap();
        for (domain, kind) in [
            // Signed order, not raw bits: -1 (0xff) is below 1.
            (
                Ty::Int(IntTy::I8),
                PatternTestKind::IntRange {
                    lo: i8v(1),
                    hi: i8v(-1),
                },
            ),
            (
                Ty::Char,
                PatternTestKind::CharRange {
                    lo: ch(122),
                    hi: ch(97),
                },
            ),
            (
                Ty::Char,
                PatternTestKind::CharRange {
                    lo: ch(0xd800),
                    hi: ch(0xdfff),
                },
            ),
            (Ty::Char, PatternTestKind::Char(ch(0x110000))),
            (Ty::Int(IntTy::I8), PatternTestKind::Char(ch(97))),
            (
                Ty::Char,
                PatternTestKind::IntRange {
                    lo: i8v(0),
                    hi: i8v(1),
                },
            ),
            (
                Ty::Int(IntTy::U8),
                PatternTestKind::IntRange {
                    lo: i8v(0),
                    hi: i8v(1),
                },
            ),
            (
                Ty::Int(IntTy::I8),
                PatternTestKind::IntRange {
                    lo: i8v(0),
                    hi: IntValue {
                        ty: IntTy::I8,
                        bits: 256,
                    },
                },
            ),
        ] {
            assert!(decide(domain, kind.clone()).is_err(), "{kind:?}");
        }
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
            (IntTy::I128, 128, true, "i128"),
            (IntTy::U128, 128, false, "u128"),
        ];
        for (ty, bits, signed, name) in types {
            assert_eq!(ty.bits(), bits);
            assert_eq!(ty.is_signed(), signed);
            assert_eq!(ty.name(), name);
            assert_eq!(ty.mask(), u128::MAX >> (128 - bits));
            assert!(Ty::Int(ty).is_copy());
            for raw in [0, ty.mask(), 1u128 << (bits - 1)] {
                let integer = IntValue { ty, bits: raw };
                integer.validate().unwrap();
                let mut function = fixture();
                function.exprs[0].ty = Ty::Int(ty);
                function.exprs[0].kind = ExprKind::Int(integer);
                function.validate().unwrap();
                let arms = [MatchArm {
                    guard: None,
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
            if bits < 128 {
                let invalid = IntValue {
                    ty,
                    bits: 1u128 << bits,
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
                    guard: None,
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
        assert_eq!(IntValue::i32(-1).bits, u32::MAX as u128);
        assert_eq!(IntValue::usize(u64::MAX).bits, u128::from(u64::MAX));
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
        let modules = fixture_modules();
        let snapshot = SnapshotId::allocate().unwrap();
        let pair = Ty::Tuple(Arc::from([Ty::Int(IntTy::I32), Ty::Int(IntTy::I32)]));
        let enumeration = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id: NominalAppId { snapshot, index: 7 },
            name: "E".into(),
            module: modules.core_root(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
            variants: Arc::from([VariantDef {
                name: "V".into(),
                shape: VariantShape::Tuple,
                fields: Arc::from([FieldDef {
                    name: "0".into(),
                    ty: Ty::Int(IntTy::I32),
                    visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
                    lifetime_bound: false,
                }]),
            }]),
        });
        let root = Ty::Struct(nominal(
            &modules,
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
            &modules,
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
        let modules = fixture_modules();
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
            &modules,
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
        let modules = fixture_modules();
        let id = NominalAppId {
            snapshot: SnapshotId::allocate().unwrap(),
            index: 0,
        };
        let ty = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id,
            name: "E".into(),
            module: modules.core_root(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
            variants: vec![
                VariantDef {
                    name: "A".into(),
                    shape: VariantShape::Tuple,
                    fields: vec![FieldDef {
                        name: "0".into(),
                        ty: Ty::Tuple(vec![Ty::Int(IntTy::I32)].into()),
                        visibility: crate::compiler::ir::Visibility::Public,
                        lifetime_bound: false,
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
            module: modules.core_root(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
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
        let modules = fixture_modules();
        let id = NominalAppId {
            snapshot: SnapshotId::allocate().unwrap(),
            index: 0,
        };
        let handle = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id,
            name: "é".repeat(1000),
            module: modules.core_root(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
            variants: vec![unit_variant("A")].into(),
        });
        let same_id = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id,
            name: "Other".into(),
            module: modules.core_root(),
            visibility: crate::compiler::ir::Visibility::Public,
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
                module: modules.core_root(),
                visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
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
            guard: None,
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
            guard: None,
            pattern: MatchPattern::Binding(LocalId(usize::MAX)),
            body: ExprId(0),
        };
        assert!(
            function
                .validate_match(ExprId(0), &[arm.clone(), invalid], &parameters)
                .is_err()
        );
        let invalid = MatchArm {
            guard: None,
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
                borrow: None,
            },
            PatternBinding {
                local: LocalId(1),
                projection: vec![1].into_iter().map(Projection::Field).collect(),
                borrow: None,
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
            borrow: None,
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
            borrow: None,
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
                borrow: None,
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
            borrow: None,
        }];
        assert!(
            function
                .validate_pattern_bindings(&b, &bindings)
                .unwrap_err()
                .contains("work limit")
        );
    }

    fn nominal(
        modules: &super::super::module_identity::ModuleArena,
        id: NominalAppId,
        name: &str,
        fields: Vec<Ty>,
    ) -> StructTy {
        StructTy::new(StructDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id,
            name: name.into(),
            module: modules.core_root(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(index, ty)| FieldDef {
                    name: format!("f{index}"),
                    ty,
                    visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
                    lifetime_bound: false,
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    #[test]
    fn nominal_identity_is_not_name_shape_or_declaration_index_alone() {
        let modules = fixture_modules();
        let snapshot = SnapshotId::allocate().unwrap();
        let first = nominal(
            &modules,
            NominalAppId { snapshot, index: 0 },
            "S",
            vec![Ty::Int(IntTy::I32)],
        );
        let same_shape = nominal(
            &modules,
            NominalAppId { snapshot, index: 1 },
            "S",
            vec![Ty::Int(IntTy::I32)],
        );
        let next_snapshot = nominal(
            &modules,
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
        let modules = fixture_modules();
        let handle = nominal(
            &modules,
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
        let modules = fixture_modules();
        let snapshot = SnapshotId::allocate().unwrap();
        let mut ty = Ty::Int(IntTy::I32);
        for index in 0..16 {
            ty = Ty::Struct(nominal(
                &modules,
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

    #[test]
    fn aggregate_match_retains_exact_nominal_domain_and_plain_field_authority() {
        let modules = fixture_modules();
        let snapshot = SnapshotId::allocate().unwrap();
        let domain = Ty::Struct(nominal(
            &modules,
            NominalAppId { snapshot, index: 0 },
            "Same",
            vec![Ty::Bool],
        ));
        let other = Ty::Struct(nominal(
            &modules,
            NominalAppId { snapshot, index: 1 },
            "Same",
            vec![Ty::Bool],
        ));
        let mut function = fixture();
        function.locals[0].ty = domain.clone();
        function.exprs[0].ty = domain.clone();
        function.exprs.push(TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Returns,
            kind: ExprKind::Block(Block {
                statements: vec![],
                tail: None,
            }),
        });
        function.exprs.push(TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Returns,
            kind: ExprKind::Match {
                scrutinee: ExprId(0),
                arms: vec![MatchArm {
                    guard: None,
                    pattern: MatchPattern::Aggregate {
                        ty: domain,
                        bindings: vec![PatternBinding {
                            local: LocalId(1),
                            projection: vec![Projection::Field(0)],
                            borrow: None,
                        }],
                    },
                    body: ExprId(1),
                }],
            },
        });
        function.body.statements = vec![Stmt::Expr(ExprId(2))];
        function.validate().unwrap();
        let mut wrong_domain = function.clone();
        let ExprKind::Match { arms, .. } = &mut wrong_domain.exprs[2].kind else {
            unreachable!()
        };
        let MatchPattern::Aggregate { ty, .. } = &mut arms[0].pattern else {
            unreachable!()
        };
        *ty = other;
        assert!(
            wrong_domain.validate().is_err(),
            "equal spelling and fields cannot replace nominal identity"
        );
        let ExprKind::Match { arms, .. } = &mut function.exprs[2].kind else {
            unreachable!()
        };
        let MatchPattern::Aggregate { bindings, .. } = &mut arms[0].pattern else {
            unreachable!()
        };
        bindings[0].projection.insert(
            0,
            Projection::Downcast(VariantId {
                owner: NominalAppId { snapshot, index: 0 },
                index: 0,
            }),
        );
        assert!(
            function.validate().is_err(),
            "aggregate bindings cannot acquire enum downcast authority"
        );
    }

    fn fixture() -> CheckedFunction {
        CheckedFunction {
            call_evidence: Arc::from([]),
            body_substitution: None,
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
    fn decision_identity_checks_nested_tag_allocation_not_only_nominal_id() {
        let modules = fixture_modules();
        let tag = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id: NominalAppId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "Tag".into(),
            module: modules.core_root(),
            visibility: Visibility::Public,
            variants: vec![unit_variant("A")].into(),
        });
        let pattern = MatchPattern::Decision {
            ty: Ty::Enum(tag.clone()),
            tests: vec![PatternTest {
                projection: vec![],
                kind: PatternTestKind::EnumVariant {
                    ty: tag.clone(),
                    variant: 0,
                },
            }],
            bindings: vec![],
        };
        let budget = PlanBudget::default();
        assert!(identical_pattern_types(&pattern, &pattern.clone(), &budget).unwrap());
        let mut replaced = pattern.clone();
        let MatchPattern::Decision { tests, .. } = &mut replaced else {
            unreachable!()
        };
        let PatternTestKind::EnumVariant { ty, .. } = &mut tests[0].kind else {
            unreachable!()
        };
        *ty = EnumTy::new(tag.def().clone());
        assert_eq!(
            pattern, replaced,
            "ordinary nominal equality only compares the application ID"
        );
        assert!(!identical_pattern_types(&pattern, &replaced, &budget).unwrap());
        assert!(
            !identical_expr_types(
                &ExprKind::Match {
                    scrutinee: ExprId(0),
                    arms: vec![MatchArm {
                        guard: None,
                        pattern,
                        body: ExprId(1)
                    }]
                },
                &ExprKind::Match {
                    scrutinee: ExprId(0),
                    arms: vec![MatchArm {
                        guard: None,
                        pattern: replaced,
                        body: ExprId(1)
                    }]
                },
                &budget,
            )
            .unwrap()
        );
    }

    #[test]
    fn decision_preflight_visits_dead_test_paths_and_nested_tag_facts() {
        let mut function = fixture();
        function.exprs[0] = TypedExpr {
            ty: Ty::Never,
            completion: Completion::Never,
            kind: ExprKind::Block(Block {
                statements: vec![Stmt::Return(None)],
                tail: None,
            }),
        };
        function.exprs.push(TypedExpr {
            ty: Ty::Unit,
            completion: Completion::Never,
            kind: ExprKind::Match {
                scrutinee: ExprId(0),
                arms: vec![MatchArm {
                    guard: None,
                    pattern: MatchPattern::Decision {
                        ty: Ty::Bool,
                        tests: vec![PatternTest {
                            projection: vec![Projection::Field(0), Projection::Field(0)],
                            kind: PatternTestKind::Bool(true),
                        }],
                        bindings: vec![],
                    },
                    body: ExprId(0),
                }],
            },
        });
        let budget = PlanBudget::new(PlanLimits {
            depth: 1,
            ..PlanLimits::default()
        });
        assert!(
            function
                .preflight_plan(FactsCtx::Concrete, &budget)
                .is_err()
        );
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ResourceBudgetKind::Depth);
        assert_eq!(failure.requested, 2);
        let ExprKind::Match { arms, .. } = &mut function.exprs[1].kind else {
            unreachable!()
        };
        let MatchPattern::Decision { tests, .. } = &mut arms[0].pattern else {
            unreachable!()
        };
        tests[0].projection.clear();
        let modules = fixture_modules();
        tests[0].kind = PatternTestKind::EnumVariant {
            ty: EnumTy::new(EnumDef {
                source_decl: test_decl(),
                // Empty enums are valid. Exceed the actual nominal-argument facts
                // bound so this test isolates traversal of nested tag metadata.
                type_args: vec![Ty::Bool; 33].into(),
                id: NominalAppId {
                    snapshot: SnapshotId::allocate().unwrap(),
                    index: 0,
                },
                name: "InvalidArguments".into(),
                module: modules.core_root(),
                visibility: Visibility::Public,
                variants: Arc::from([]),
            }),
            variant: 0,
        };
        assert!(
            function
                .preflight_plan(FactsCtx::Concrete, &PlanBudget::default())
                .is_err()
        );
    }

    #[test]
    fn decision_substitution_remaps_test_and_binding_downcasts() {
        struct Remap {
            original: EnumTy,
            replacement: EnumTy,
        }
        impl SubstitutionCx for Remap {
            fn nominal_application(&self, id: NominalAppId) -> Result<NominalAppInfo, String> {
                if id != self.original.def().id {
                    return Err("unexpected nominal application".into());
                }
                Ok(NominalAppInfo {
                    source_decl: self.original.def().source_decl,
                    type_args: self.original.def().type_args.clone(),
                })
            }
            fn intern_nominal(
                &mut self,
                declaration: DeclId,
                args: &[Ty],
                _: FactsCtx<'_>,
                _: &PlanBudget,
            ) -> Result<Ty, String> {
                if declaration != self.original.def().source_decl || !args.is_empty() {
                    return Err("unexpected nominal substitution".into());
                }
                Ok(Ty::Enum(self.replacement.clone()))
            }
            fn instantiate_signature(
                &mut self,
                _: &CallTarget,
                _: FactsCtx<'_>,
                _: &PlanBudget,
            ) -> Result<Signature, String> {
                Err("unexpected callable substitution".into())
            }
        }
        let modules = fixture_modules();
        let original = EnumTy::new(EnumDef {
            source_decl: test_decl(),
            type_args: Arc::from([]),
            id: NominalAppId {
                snapshot: SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "Tag".into(),
            module: modules.core_root(),
            visibility: Visibility::Public,
            variants: vec![VariantDef {
                name: "A".into(),
                shape: VariantShape::Tuple,
                fields: vec![FieldDef {
                    name: "0".into(),
                    ty: Ty::Bool,
                    visibility: Visibility::Public,
                    lifetime_bound: false,
                }]
                .into(),
            }]
            .into(),
        });
        let mut definition = original.def().clone();
        definition.id.index = 1;
        let replacement = EnumTy::new(definition);
        let old = Projection::Downcast(VariantId {
            owner: original.def().id,
            index: 0,
        });
        let mut pattern = MatchPattern::Decision {
            ty: Ty::Enum(original.clone()),
            tests: vec![
                PatternTest {
                    projection: vec![],
                    kind: PatternTestKind::EnumVariant {
                        ty: original.clone(),
                        variant: 0,
                    },
                },
                PatternTest {
                    projection: vec![old, Projection::Field(0)],
                    kind: PatternTestKind::Bool(true),
                },
            ],
            bindings: vec![PatternBinding {
                local: LocalId(1),
                projection: vec![old, Projection::Field(0)],
                borrow: None,
            }],
        };
        let budget = PlanBudget::default();
        let env = GenericEnv::new(&[], &budget).unwrap();
        let mut context = Remap {
            original,
            replacement: replacement.clone(),
        };
        Instantiate {
            env: &env,
            args: &[],
            facts: FactsCtx::Concrete,
            cx: &mut context,
            budget: &budget,
        }
        .pattern(&mut pattern)
        .unwrap();
        let MatchPattern::Decision {
            ty,
            tests,
            bindings,
        } = pattern
        else {
            unreachable!()
        };
        assert_eq!(ty, Ty::Enum(replacement.clone()));
        let PatternTestKind::EnumVariant { ty, .. } = &tests[0].kind else {
            unreachable!()
        };
        assert_eq!(ty.def().id, replacement.def().id);
        let new = Projection::Downcast(VariantId {
            owner: replacement.def().id,
            index: 0,
        });
        assert_eq!(tests[1].projection, [new, Projection::Field(0)]);
        assert_eq!(bindings[0].projection, [new, Projection::Field(0)]);
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
