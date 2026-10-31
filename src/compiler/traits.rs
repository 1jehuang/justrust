//! A single validated trait world, obligation engine and fallible facts authority.
//!
//! Raw schema callbacks are structural only. Neither raw schemas nor a caller's
//! claimed successful analysis can manufacture body facts or dispatch evidence.
use super::ir::{
    self, CallTarget, Callee, DeclId, DeclKind, GenericEnv, ParamEnv, ParamId, PlanBudget,
    Predicate, ReceiverMode, Signature, SignatureSubject, SnapshotId, TraitCall, TraitCx, TraitRef,
    Ty,
};
use super::module_identity::{ModuleArena, ModuleError, ModuleId};
use std::sync::Arc;

pub(super) mod alignment_provider;
mod associated_bounds;
mod associated_constants;
mod associated_types;
mod body_validation;
mod callable_inputs;
mod carrier_regions;
mod catalog;
#[path = "traits/closure_dispatch.rs"]
mod closure_dispatch;
mod closure_owner_instance;
mod closure_owner_substitution;
mod closure_projections;
mod closure_provider_domain;
mod closure_results;
mod closures;
mod constant_members;
mod dynamic_objects;
mod equality;
mod fn_items;
mod generic_methods;
mod impl_constraints;
mod inherent_constants;
mod nominal_declarations;
mod nominal_realization;
fn nominal_fields(fields: &ir::NominalFields, budget: &PlanBudget) -> Result<Arc<[ir::FieldDef]>> {
    fields
        .read(budget)
        .map_err(|error| FactsError::from_message(budget, &error))
}
pub use nominal_declarations::{
    NominalDeclarationProof, NominalDeclarationSchema, NominalDefaultSchema,
};
pub(in crate::compiler) use nominal_realization::ClosedNominalInput;
mod nominal_carriers;
pub(in crate::compiler) use nominal_carriers::NominalCarrierSource;
mod nominal_normalization;
mod normal_drop;
mod opaque;
mod parameter_equality;
mod physical_carriers;
mod predicate_subjects;
mod projected_methods;
mod schema_instantiation;
mod symbolic_equalities;
mod universal_carriers;
mod universal_methods;
mod universal_projections;
pub use associated_constants::{
    AssociatedConstHeader, AssociatedConstantType, ClosedAssociatedConstant,
};
pub use associated_types::NormalizedAssociatedType;
pub use catalog::{
    CallableProvider, CatalogDecl, CatalogItem, CoreCatalog, CoreCatalogBuilder, DeclOrigin,
    RecipeDescriptor, RecipeShapeInventory,
};
pub use constant_members::{ConstantMemberType, SelectedAssociatedConstant};
pub use equality::NormalizedProjectionEquality;
pub use inherent_constants::InherentConstantType;
pub use normal_drop::{DropGlue, NormalDropPlan, ValidatedDestructor};

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
    Infrastructure,
}
#[cfg(test)]
mod infrastructure_diagnostic_tests {
    use super::*;
    #[test]
    fn precise_infrastructure_prefix_preserves_other_diagnostic_classes() {
        let budget = PlanBudget::default();
        for (message, expected) in [
            (
                "infrastructure: function pointer scratch allocation failed",
                FactsErrorKind::Infrastructure,
            ),
            (
                "infrastructure: allocator resource exhausted",
                FactsErrorKind::Infrastructure,
            ),
            (
                "generic planning resource limit exceeded",
                FactsErrorKind::Resource,
            ),
            (
                "unsupported: function pointer call",
                FactsErrorKind::Unsupported,
            ),
            (
                "invalid signature metadata",
                FactsErrorKind::InvalidMetadata,
            ),
            (
                "invalid infrastructure: source spelling",
                FactsErrorKind::InvalidMetadata,
            ),
        ] {
            assert_eq!(
                FactsError::from_message(&budget, message).kind(),
                expected,
                "{message}"
            );
        }
        assert_eq!(
            FactsError::infrastructure(&budget, "allocation failed").kind(),
            FactsErrorKind::Infrastructure
        );
    }
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
    // Exact PlanBudget messages only, never caller/source text. These bounded
    // static spellings preserve context after the diagnostic ledger is exhausted.
    fn generic_resource_message(message: &str) -> Option<&'static str> {
        match message {
            "generic planning resource limit exceeded" => {
                Some("generic planning resource limit exceeded")
            }
            "generic planning depth limit exceeded" => {
                Some("generic planning depth limit exceeded")
            }
            _ => None,
        }
    }

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
    pub fn infrastructure(b: &PlanBudget, m: &str) -> Self {
        Self::make(b, FactsErrorKind::Infrastructure, m)
    }
    pub fn from_message(b: &PlanBudget, m: &str) -> Self {
        if let Some(message) = Self::generic_resource_message(m) {
            return Self {
                kind: FactsErrorKind::Resource,
                diagnostic: Diagnostic::Static(message),
            };
        }
        if b.charge_work(m.len()).is_err() {
            return Self::resource(b, "trait diagnostic classification resource boundary");
        }
        let kind = if m.starts_with("infrastructure:") {
            FactsErrorKind::Infrastructure
        } else if m.contains("limit")
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
    /// unmetered transport exceptions are fixed, source-independent resource
    /// messages below (under 80 bytes), approved for legacy String boundaries.
    /// Diagnostic exhaustion must not erase an error or reset semantic budgets.
    pub fn into_legacy_diagnostic(self, budget: &PlanBudget) -> String {
        const EXHAUSTED: &str =
            "trait planning resource limit exceeded (diagnostic transport exhausted)";
        let Self { kind, diagnostic } = self;
        let fallback = match &diagnostic {
            Diagnostic::Static(message) if kind == FactsErrorKind::Resource => {
                Self::generic_resource_message(message).unwrap_or(EXHAUSTED)
            }
            _ => EXHAUSTED,
        };
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
                let _ = budget.retain_bytes(fallback.len());
                let _ = budget.charge_work(fallback.len());
                fallback.to_owned()
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
fn checked_module<T>(
    budget: &PlanBudget,
    result: std::result::Result<T, ModuleError>,
) -> Result<T> {
    result.map_err(|error| match error {
        ModuleError::Token(super::token_session::TokenError::Budget(_))
        | ModuleError::Name(super::semantic_name::NameError::Token(
            super::token_session::TokenError::Budget(_),
        )) => FactsError::resource(budget, "module authority resource boundary"),
        ModuleError::SourceUnavailable => FactsError::unsupported(
            budget,
            "source module root unavailable in catalog-only authority",
        ),
        _ => FactsError::invalid(budget, "invalid or foreign module/visibility authority"),
    })
}
fn validate_nominal_modules(ty: &Ty, modules: &ModuleArena, budget: &PlanBudget) -> Result<()> {
    charged(budget, budget.charge_work(1))?;
    match ty {
        Ty::Struct(nominal) => {
            let definition = nominal.def();
            checked_module(
                budget,
                definition
                    .visibility
                    .validate_for(modules, definition.module),
            )?;
            for field in nominal_fields(&definition.fields, budget)?.iter() {
                charged(budget, budget.charge_work(1))?;
                checked_module(
                    budget,
                    field.visibility.validate_for(modules, definition.module),
                )?;
            }
        }
        Ty::Enum(nominal) => {
            let definition = nominal.def();
            checked_module(
                budget,
                definition
                    .visibility
                    .validate_for(modules, definition.module),
            )?;
            for variant in definition.variants.iter() {
                charged(budget, budget.charge_work(1))?;
                for field in nominal_fields(&variant.fields, budget)?.iter() {
                    charged(budget, budget.charge_work(1))?;
                    checked_module(
                        budget,
                        field.visibility.validate_for(modules, definition.module),
                    )?;
                }
            }
        }
        _ => {}
    }
    Ok(())
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
/// First-position index for an append-only declaration table. Lookups were
/// linear scans charging the shared ledger per element, which made each
/// lookup O(program) and every body check O(program^2) under one mutex.
#[derive(Debug, Default)]
pub(super) struct DeclIndex(std::collections::HashMap<DeclId, usize>);
impl DeclIndex {
    /// Record `declaration` at `position` unless an earlier row already has
    /// it, preserving the previous first-match lookup semantics.
    fn note(&mut self, b: &PlanBudget, declaration: DeclId, position: usize) -> Result<()> {
        charged(b, b.charge_work(1))?;
        self.0
            .try_reserve(1)
            .map_err(|_| FactsError::resource(b, "declaration index allocation"))?;
        self.0.entry(declaration).or_insert(position);
        Ok(())
    }
    fn get(&self, b: &PlanBudget, declaration: DeclId) -> Result<Option<usize>> {
        charged(b, b.charge_work(1))?;
        Ok(self.0.get(&declaration).copied())
    }
}
fn push_indexed_environment(
    b: &PlanBudget,
    environments: &mut Vec<Arc<PredicateEnv>>,
    index: &mut DeclIndex,
    env: Arc<PredicateEnv>,
) -> Result<()> {
    if let Some(declaration) = env.declaration {
        index.note(b, declaration, environments.len())?;
    }
    push(b, environments, env)
}
fn push<T>(b: &PlanBudget, values: &mut Vec<T>, value: T) -> Result<()> {
    // Two retained slots per element pay for geometric capacity growth.
    // Admit the old allocation separately while relocation is in progress.
    retain::<T>(b, 2)?;
    if values.len() == values.capacity() {
        charged(b, b.charge_work(values.len()))?;
        let bytes = values
            .len()
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| FactsError::resource(b, "trait relocation size overflow"))?;
        let _relocation = charged(b, b.reserve_bytes(bytes))?;
        values
            .try_reserve_exact(values.len().max(1))
            .map_err(|_| FactsError::resource(b, "trait metadata allocation failed"))?;
    }
    values.push(value);
    Ok(())
}
/// Storage for private memo rows, never for a returned semantic payload.
/// The caller declares `leases` after `values`, so rows die before their leases.
fn push_scratch_row<T>(
    budget: &PlanBudget,
    values: &mut Vec<T>,
    leases: &mut Vec<ir::PlanReservation>,
    value: T,
) -> Result<()> {
    push_scratch_row_with(budget, values, leases, value, || Ok(()))
}

fn scratch_row_bytes<T>() -> Option<usize> {
    std::mem::size_of::<T>()
        .checked_mul(2)?
        .checked_add(std::mem::size_of::<ir::PlanReservation>().checked_mul(2)?)?
        .checked_add(32)
}

fn push_scratch_row_with<T>(
    budget: &PlanBudget,
    values: &mut Vec<T>,
    leases: &mut Vec<ir::PlanReservation>,
    value: T,
    after_row_growth: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let bytes = scratch_row_bytes::<T>()
        .ok_or_else(|| FactsError::resource(budget, "scratch row storage overflow"))?;
    let storage = charged(budget, budget.reserve_bytes(bytes))?;
    let row_growth = TypeWalk::growth_slots(values.len(), values.capacity())
        .ok_or_else(|| FactsError::resource(budget, "scratch row capacity overflow"))?;
    let lease_growth = TypeWalk::growth_slots(leases.len(), leases.capacity())
        .ok_or_else(|| FactsError::resource(budget, "scratch lease capacity overflow"))?;
    let old_rows = if row_growth == 0 { 0 } else { values.len() };
    let old_leases = if lease_growth == 0 { 0 } else { leases.len() };
    let relocation_bytes = old_rows
        .checked_mul(std::mem::size_of::<T>())
        .and_then(|bytes| {
            old_leases
                .checked_mul(std::mem::size_of::<ir::PlanReservation>())
                .and_then(|other| bytes.checked_add(other))
        })
        .ok_or_else(|| FactsError::resource(budget, "scratch relocation storage overflow"))?;
    let _relocation = charged(budget, budget.reserve_bytes(relocation_bytes))?;
    let work = old_rows
        .checked_add(old_leases)
        .ok_or_else(|| FactsError::resource(budget, "scratch relocation work overflow"))?;
    charged(budget, budget.charge_work(work))?;
    let grow = (|| {
        if row_growth != 0 {
            values
                .try_reserve_exact(row_growth)
                .map_err(|_| FactsError::resource(budget, "scratch row allocation failed"))?;
        }
        // A no-op in production. Tests deny this exact post-growth boundary.
        after_row_growth()?;
        if lease_growth != 0 {
            leases
                .try_reserve_exact(lease_growth)
                .map_err(|_| FactsError::resource(budget, "scratch lease allocation failed"))?;
        }
        let paid_slots = values
            .len()
            .checked_add(1)
            .and_then(|len| len.checked_mul(2))
            .ok_or_else(|| FactsError::resource(budget, "scratch paid capacity overflow"))?;
        if values.len() != leases.len()
            || values.capacity() > paid_slots
            || leases.capacity() > paid_slots
        {
            return Err(FactsError::resource(
                budget,
                "scratch allocator exceeded paid capacity",
            ));
        }
        Ok(())
    })();
    if let Err(error) = grow {
        // A first growth may have succeeded before a later allocation failed.
        // Destroy all surviving buffers while BOTH old and new leases are live.
        // Never refund capacity that an error path leaves allocated in the owner.
        drop(std::mem::take(values));
        drop(std::mem::take(leases));
        return Err(error);
    }
    values.push(value);
    leases.push(storage);
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
    pub constants: Arc<[AssociatedConstHeader]>,
    pub traits: Arc<[TraitHeader]>,
    pub associated_types: Arc<[AssociatedTypeHeader]>,
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
    pub associated_types: Arc<[DeclId]>,
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
    pub associated_types: Arc<[AssociatedTypeValue]>,
}
#[derive(Clone, Debug)]
pub struct AssociatedTypeHeader {
    pub declaration: DeclId,
    pub trait_decl: DeclId,
    pub env: Arc<GenericEnv>,
    pub bounds: Arc<[DeclId]>,
    /// Declared `?Sized`: no implicit `Sized` on the family or its values.
    pub relaxed_sized: bool,
}
#[derive(Clone, Debug)]
pub struct AssociatedTypeValue {
    pub implementation: DeclId,
    pub associated: DeclId,
    pub rhs: Ty,
    pub source: Arc<AssociatedTypeSource>,
}
/// Original assignment metadata, not a loan or an executable carrier proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssociatedSourceRegion {
    Named(ir::SourceRegionId),
    Static,
    Elided,
}
#[derive(Clone, Debug)]
pub struct AssociatedTypeSource {
    pub implementation: DeclId,
    pub associated: DeclId,
    pub env: Arc<GenericEnv>,
    pub region_count: u16,
    pub outlives: Arc<[(AssociatedSourceRegion, AssociatedSourceRegion)]>,
    /// Exact root reference layers. No nested nominal origins are inferred.
    pub references: Arc<[(ir::BorrowKind, AssociatedSourceRegion)]>,
    pub parameter_tail: Option<ParamId>,
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
    pub module: ModuleId,
    pub visibility: super::ir::Visibility,
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
/// Private-to-the-computation canonical reconstruction scratch. It never
/// supplies proof on its own: every node still authenticates its exact payload
/// and caller. Keeping it across one walk avoids rebuilding every descendant
/// application independently for each enclosing nominal.
pub trait NominalValidationSession: Send {
    fn validate_node(
        &mut self,
        ty: &Ty,
        env: &GenericEnv,
        caller: Option<&ValidatedFacts>,
        budget: &PlanBudget,
    ) -> Result<()>;
}

pub trait SchemaCx {
    fn nominal_validation_session(&self) -> Result<Option<Box<dyn NominalValidationSession>>> {
        Ok(None)
    }
    /// An alternate adapter cannot manufacture paired raw exchange authority.
    fn native_sequence_swap(
        &self,
        declaration: DeclId,
    ) -> Result<Option<Arc<super::frontend::NativeSequenceSwap>>> {
        let _ = declaration;
        self.budget()
            .charge_work(1)
            .map_err(|message| FactsError::from_message(self.budget(), &message))?;
        Ok(None)
    }
    /// Default deny. Declaration identity alone never proves a raw slot recipe.
    fn native_sequence_slot(
        &self,
        declaration: DeclId,
    ) -> Result<Option<Arc<super::frontend::NativeSequenceSlot>>> {
        let _ = declaration;
        self.budget()
            .charge_work(1)
            .map_err(|message| FactsError::from_message(self.budget(), &message))?;
        Ok(None)
    }
    fn budget(&self) -> &PlanBudget;
    fn registry_snapshot(&self) -> SnapshotId;
    fn core_catalog(&self) -> &Arc<CoreCatalog>;
    fn declaration_info(&self, id: DeclId) -> Result<SchemaDecl>;
    fn declaration_schema(&self, id: DeclId) -> Result<GenericSchema>;
    /// Original declaration safety, never a caller-supplied header flag. A
    /// source adapter authenticates the exact implementation/declared-trait
    /// pair. An omitted service cannot erase an unsafe source obligation.
    fn impl_trait_safety(
        &self,
        implementation: DeclId,
        trait_decl: DeclId,
    ) -> Result<(bool, bool)> {
        let _ = (implementation, trait_decl);
        Err(FactsError::unsupported(
            self.budget(),
            "unsupported: original trait implementation safety authority unavailable",
        ))
    }
    /// Default deny: a catalog or alternate schema adapter cannot authenticate
    /// original loader/parser function occurrences using declaration IDs alone.
    fn replay_packed_reserve_authority(
        &self,
        authority: &super::frontend::PackedReserveAuthority,
        declaration: DeclId,
    ) -> Result<()> {
        let _ = (authority, declaration);
        Err(FactsError::invalid(
            self.budget(),
            "packed reserve source authority is unavailable in this schema adapter",
        ))
    }
    fn closure_trait_kind(&self, declaration: DeclId) -> Result<Option<ir::ClosureKind>> {
        let _ = declaration;
        Ok(None)
    }
    fn closure_owner_site(&self, provider: DeclId) -> Result<Option<(DeclId, u32)>> {
        let _ = provider;
        Ok(None)
    }
    fn associated_constant_schema(&self, declaration: DeclId) -> Result<AssociatedConstHeader> {
        let _ = declaration;
        Err(FactsError::invalid(
            self.budget(),
            "associated constant has no retained source schema",
        ))
    }
    fn instantiate_constant_metadata(
        &self,
        declaration: DeclId,
        args: &[Ty],
        caller: &GenericEnv,
        budget: &PlanBudget,
    ) -> Result<(Option<Ty>, Ty, Arc<[Predicate]>)> {
        let _ = (declaration, args, caller);
        Err(FactsError::invalid(
            budget,
            "constant metadata has no retained substitution provider",
        ))
    }
    fn instantiate_constant_type(
        &self,
        declaration: DeclId,
        args: &[Ty],
        caller: &GenericEnv,
        budget: &PlanBudget,
    ) -> Result<Ty> {
        let _ = (declaration, args, caller);
        Err(FactsError::invalid(
            budget,
            "associated constant has no canonical type substitution provider",
        ))
    }
    fn closure_recipe(&self, provider: DeclId) -> Result<Option<ir::ClosureRecipeId>> {
        let _ = provider;
        Ok(None)
    }
    fn associated_type_bounds(&self, associated: DeclId) -> Result<Arc<[DeclId]>> {
        let _ = associated;
        Ok(Arc::from([]))
    }
    fn associated_type_is_sized(&self, associated: DeclId) -> Result<bool> {
        self.declaration_info(associated)?;
        Ok(true)
    }
    fn associated_type_predicates(&self, associated: DeclId) -> Result<Arc<[Predicate]>> {
        let info = self.declaration_info(associated)?;
        let owner = info.owner.ok_or_else(|| {
            FactsError::invalid(self.budget(), "associated bounds have no trait owner")
        })?;
        let declaration = self.declaration_info(owner)?;
        let mut parameters = declaration.env.ordered_params();
        let self_ty = Ty::Param(parameters.next().ok_or_else(|| {
            FactsError::invalid(self.budget(), "associated owner has no Self binder")
        })?);
        let mut arguments = Vec::new();
        for parameter in parameters {
            push(self.budget(), &mut arguments, Ty::Param(parameter))?;
        }
        let subject = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: owner,
                self_ty,
                args: arguments.into(),
            },
            item: associated,
        }));
        let mut predicates = Vec::new();
        for bound in self.associated_type_bounds(associated)?.iter() {
            push(
                self.budget(),
                &mut predicates,
                Predicate::Implements(TraitRef {
                    declaration: *bound,
                    self_ty: subject.clone(),
                    args: Arc::from([]),
                }),
            )?;
        }
        Ok(predicates.into())
    }
    fn associated_type_rhs(&self, implementation: DeclId, associated: DeclId) -> Result<Ty> {
        let _ = (implementation, associated);
        Err(FactsError::invalid(
            self.budget(),
            "associated type assignment has no authoritative RHS",
        ))
    }
    fn associated_type_source(
        &self,
        implementation: DeclId,
        associated: DeclId,
    ) -> Result<Arc<AssociatedTypeSource>> {
        let _ = (implementation, associated);
        Err(FactsError::invalid(
            self.budget(),
            "associated assignment has no original source metadata",
        ))
    }
    fn source_region_env(&self, id: DeclId) -> Result<Arc<ir::SourceRegionEnv>>;
    fn instantiate_schema(
        &self,
        schema: &GenericSchema,
        args: &[Ty],
        caller: &GenericEnv,
        budget: &PlanBudget,
    ) -> Result<InstantiatedSchema>;
    fn nominal_declaration_schema(&self, _declaration: DeclId) -> Result<NominalDeclarationSchema> {
        Err(FactsError::unsupported(
            self.budget(),
            "nominal declaration source schema is unavailable",
        ))
    }
    fn validate_nominal(&self, ty: &Ty, env: &GenericEnv, budget: &PlanBudget) -> Result<()>;
    /// Preserve the original authenticated caller during metadata reconstruction.
    fn validate_nominal_with_facts(
        &self,
        ty: &Ty,
        facts: &ValidatedFacts,
        budget: &PlanBudget,
    ) -> Result<()> {
        self.validate_nominal(ty, facts.env(), budget)
    }
    /// Authenticate one nominal definition. The caller must separately walk
    /// every actual and field under its ordinary type-use rules. Adapters
    /// without a node-local validator retain the full recursive check.
    fn validate_nominal_node(&self, ty: &Ty, env: &GenericEnv, budget: &PlanBudget) -> Result<()> {
        self.validate_nominal(ty, env, budget)
    }
    fn validate_nominal_node_with_facts(
        &self,
        ty: &Ty,
        facts: &ValidatedFacts,
        budget: &PlanBudget,
    ) -> Result<()> {
        self.validate_nominal_node(ty, facts.env(), budget)
    }
    /// Remint an application from its authoritative source schema after the
    /// caller normalizes actual arguments with checked projection evidence.
    /// Implementations must validate the original canonical application and
    /// preserve its source declaration and nominal kind. Field rewriting or
    /// reusing its application ID for changed actuals is not permitted.
    fn reapply_nominal(
        &self,
        original: &Ty,
        arguments: &[Ty],
        env: &GenericEnv,
        budget: &PlanBudget,
    ) -> Result<Ty> {
        let _ = (original, arguments, env);
        Err(FactsError::unsupported(
            budget,
            "unsupported: canonical nominal reapplication is unavailable",
        ))
    }
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
    AssociatedConstant(DeclId),
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
    closure: Option<Arc<ir::ClosureTy>>,
    closure_mode: Option<ir::ClosureKind>,
}

/// An original header lookup, not an executable instantiation or body proof.
#[derive(Clone, Copy, Debug)]
pub(super) enum OriginalCallableSubject<'a> {
    Direct(DeclId),
    TraitItem(&'a TraitCall),
}

/// Context-bound and deliberately not cached in the world it retains.
#[derive(Debug)]
pub(super) struct OriginalCallableView {
    facts: Arc<ValidatedFacts>,
    declaration: DeclId,
    schema: GenericSchema,
    source_regions: Arc<ir::SourceRegionEnv>,
    receiver: ReceiverMode,
    required: Option<(TraitCall, Arc<CarrierContractView>)>,
}
impl OriginalCallableView {
    #[cfg(test)]
    pub(super) fn declaration(&self) -> DeclId {
        self.declaration
    }
    #[cfg(test)]
    pub(super) fn subject(&self) -> OriginalCallableSubject<'_> {
        match &self.required {
            Some((call, _)) => OriginalCallableSubject::TraitItem(call),
            None => OriginalCallableSubject::Direct(self.declaration),
        }
    }
    pub(super) fn generic_env(&self) -> &Arc<GenericEnv> {
        &self.schema.env
    }
    pub(super) fn signature(&self) -> &Signature {
        &self.schema.signature
    }
    #[cfg(test)]
    pub(super) fn predicates(&self) -> &Arc<[Predicate]> {
        &self.schema.predicates
    }
    #[cfg(test)]
    pub(super) fn source_region_env(&self) -> &Arc<ir::SourceRegionEnv> {
        &self.source_regions
    }
    #[cfg(test)]
    pub(super) fn receiver(&self) -> ReceiverMode {
        self.receiver
    }
    #[cfg(test)]
    pub(super) fn facts(&self) -> &Arc<ValidatedFacts> {
        &self.facts
    }
    pub(super) fn validate_for(&self, facts: &ValidatedFacts) -> Result<()> {
        facts.world.check_original_context(facts)?;
        if !std::ptr::eq(self.facts.as_ref(), facts) {
            return Err(FactsError::invalid(
                facts.budget(),
                "original callable view changed its exact caller facts",
            ));
        }
        let info = facts.world.info(self.declaration)?;
        if self.source_regions.declaration != self.declaration || self.receiver != info.receiver {
            return Err(FactsError::invalid(
                facts.budget(),
                "original callable view changed its header authority",
            ));
        }
        if let Some((call, view)) = &self.required {
            view.validate_for(facts)?;
            let (application, _) = facts
                .world
                .instantiate_universal_required_item(call, facts)?;
            if !facts.world.same_env(&self.schema.env, facts.env())?
                || !facts
                    .world
                    .eq_signature(&self.schema.signature, &application.signature)?
                || *self.source_regions != *view.source_regions
            {
                return Err(FactsError::invalid(
                    facts.budget(),
                    "original required replay changed its exact caller application",
                ));
            }
        }
        Ok(())
    }
}
impl ValidatedFacts {
    pub(in crate::compiler) fn native_sequence_swap(
        &self,
        declaration: DeclId,
    ) -> Result<Option<Arc<super::frontend::NativeSequenceSwap>>> {
        self.world.check_original_context(self)?;
        if self.phase() == FactsPhase::Schema {
            return Ok(None);
        }
        self.world.schema.native_sequence_swap(declaration)
    }

    pub(in crate::compiler) fn native_sequence_slot(
        &self,
        declaration: DeclId,
    ) -> Result<Option<Arc<super::frontend::NativeSequenceSlot>>> {
        self.world.check_original_context(self)?;
        if self.phase() == FactsPhase::Schema {
            return Ok(None);
        }
        self.world.schema.native_sequence_slot(declaration)
    }

    /// Replay against the actual borrowed source adapter, not a capability's
    /// self-reported owner/snapshot. The query never retains these facts/world.
    pub(in crate::compiler) fn replay_packed_reserve_authority(
        &self,
        authority: &super::frontend::PackedReserveAuthority,
    ) -> Result<()> {
        if self.phase() == FactsPhase::Schema
            || self.closure.is_some()
            || self.closure_mode.is_some()
            || self.env().ordered_params().next().is_some()
            || self
                .target()
                .is_some_and(|target| !target.type_args.is_empty())
        {
            return Err(FactsError::invalid(
                self.budget(),
                "packed reserve requires its exact nongeneric callable facts",
            ));
        }
        self.world
            .schema
            .replay_packed_reserve_authority(authority, self.declaration())
    }

    /// Ordered method slots of a trait object's vtable for `principal`:
    /// supertrait methods first (depth-first, declaration order, each trait
    /// once), then the principal's own methods in declaration order. Every
    /// slot is `(owning trait, trait item)`. Shared by `&dyn` and `Box<dyn>`.
    /// The private native vtable is `[drop_in_place or 0, size, align,
    /// slot 0, slot 1, ...]`, eight bytes per entry.
    pub fn vtable_methods(&self, principal: DeclId) -> Result<Vec<(DeclId, DeclId)>> {
        fn visit(
            world: &ValidatedTraitWorld,
            declaration: DeclId,
            seen: &mut Vec<DeclId>,
            out: &mut Vec<(DeclId, DeclId)>,
            depth: usize,
        ) -> Result<()> {
            charged(&world.budget, world.budget.check_depth(depth))?;
            charged(&world.budget, world.budget.charge_work(1))?;
            if seen.contains(&declaration) {
                return Ok(());
            }
            push(&world.budget, seen, declaration)?;
            let header = world.trait_header(declaration)?;
            for parent in header.supertraits.iter() {
                visit(world, parent.declaration, seen, out, depth + 1)?;
            }
            for item in header.items.iter() {
                push(&world.budget, out, (declaration, *item))?;
            }
            Ok(())
        }
        let mut seen = Vec::new();
        let mut out = Vec::new();
        visit(&self.world, principal, &mut seen, &mut out, 0)?;
        Ok(out)
    }
    /// Closed implementation of every vtable slot of `principal` for the
    /// concrete Sized type `concrete` (`concrete: principal` must hold).
    pub fn vtable_targets(&self, concrete: &Ty, principal: DeclId) -> Result<Vec<CallTarget>> {
        if self.phase() != FactsPhase::Closed {
            return Err(FactsError::invalid(
                self.budget(),
                "vtable targets require closed facts",
            ));
        }
        if matches!(concrete, Ty::Dyn(_) | Ty::Slice { .. } | Ty::Param(_)) {
            return Err(FactsError::unsupported(
                self.budget(),
                "unsupported: vtable for an unsized or open concrete type",
            ));
        }
        let mut targets = Vec::new();
        for (owner, item) in self.vtable_methods(principal)? {
            let call = TraitCall {
                trait_ref: TraitRef {
                    declaration: owner,
                    self_ty: concrete.clone(),
                    args: Arc::from([]),
                },
                item,
                method_args: Arc::from([]),
            };
            let resolved = self.world.resolve_trait_call(&call, self)?;
            let Callee::Direct(target) = resolved.callee() else {
                return Err(FactsError::unsupported(
                    self.budget(),
                    "unsupported: vtable slot without a closed implementation",
                ));
            };
            push(self.budget(), &mut targets, target.clone())?;
        }
        Ok(targets)
    }
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
            FactsSubject::SchemaItem(_) | FactsSubject::AssociatedConstant(_) => FactsPhase::Schema,
            FactsSubject::UniversalCallable(_) => FactsPhase::Universal,
            FactsSubject::ClosedInstance(_) => FactsPhase::Closed,
        }
    }
    pub fn budget(&self) -> &PlanBudget {
        &self.world.budget
    }
    pub fn declaration(&self) -> DeclId {
        match &self.subject {
            FactsSubject::SchemaItem(d)
            | FactsSubject::AssociatedConstant(d)
            | FactsSubject::UniversalCallable(d) => *d,
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
    pub(super) fn original_callable_schema(
        self: &Arc<Self>,
        subject: OriginalCallableSubject<'_>,
    ) -> Result<Arc<OriginalCallableView>> {
        self.world.check_original_context(self)?;
        let declaration = match subject {
            OriginalCallableSubject::Direct(declaration) => declaration,
            OriginalCallableSubject::TraitItem(call) => {
                let (application, _) =
                    self.world.instantiate_universal_required_item(call, self)?;
                let required = self.world.universal_required_carrier_contract(call, self)?;
                retain::<OriginalCallableView>(self.budget(), 1)?;
                return Ok(Arc::new(OriginalCallableView {
                    facts: self.clone(),
                    declaration: call.item,
                    schema: GenericSchema {
                        env: self.env().clone(),
                        signature: application.signature,
                        predicates: application.predicates,
                    },
                    source_regions: required.source_regions.clone(),
                    receiver: self.world.info(call.item)?.receiver,
                    required: Some((call.clone(), required)),
                }));
            }
        };
        let info = self.world.info(declaration)?;
        if !matches!(info.kind, DeclKind::Function | DeclKind::AssociatedFunction) {
            return Err(FactsError::invalid(
                self.budget(),
                "original direct-call subject is not a callable declaration",
            ));
        }
        self.world.ensure_provider(declaration)?;
        let schema = self.world.callable_schema(declaration)?;
        if !self.world.same_env(&schema.env, &info.env)? {
            return Err(FactsError::invalid(
                self.budget(),
                "original callable header changed its authoritative binder order",
            ));
        }
        let source_regions = self.source_region_env(declaration)?;
        self.budget()
            .charge_nodes(1)
            .map_err(|error| FactsError::resource(self.budget(), &error))?;
        self.budget()
            .retain_bytes(
                std::mem::size_of::<OriginalCallableView>() + 2 * std::mem::size_of::<usize>(),
            )
            .map_err(|error| FactsError::resource(self.budget(), &error))?;
        Ok(Arc::new(OriginalCallableView {
            facts: self.clone(),
            declaration,
            schema,
            source_regions,
            receiver: info.receiver,
            required: None,
        }))
    }
    /// Provider role is derived from validated world headers and catalog, not a
    /// caller flag. Registry source ordinals remain the frontend's authority.
    pub(super) fn is_source_callable(&self, declaration: DeclId) -> Result<bool> {
        self.world.check_original_context(self)?;
        if self.world.schema.closure_owner_site(declaration)?.is_some() {
            return Ok(false);
        }
        let info = self.world.info(declaration)?;
        self.world.callable_header(declaration)?;
        self.world.ensure_provider(declaration)?;
        if let Some(record) = self.world.catalog.declaration(declaration) {
            return match record.callable_provider {
                Some(CallableProvider::Recipe { .. }) => Ok(false),
                _ => Err(FactsError::invalid(
                    self.budget(),
                    "catalog callable has no validated executable provider",
                )),
            };
        }
        if !matches!(info.kind, DeclKind::Function | DeclKind::AssociatedFunction) {
            return Err(FactsError::invalid(
                self.budget(),
                "source provider is not an executable callable declaration",
            ));
        }
        Ok(true)
    }
    pub fn validate_signature(
        &self,
        subject: SignatureSubject<'_>,
        signature: &Signature,
    ) -> Result<()> {
        TraitCx::validate_signature(self.world.as_ref(), subject, signature, self)
    }
    /// As `validate_signature`, sharing a node-local scratch across calls.
    pub fn validate_signature_with(
        &self,
        subject: SignatureSubject<'_>,
        signature: &Signature,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        self.world
            .validate_signature_with(subject, signature, self, scratch)
    }
    /// As `validate_ty_metadata_with_budget`, sharing a node-local scratch.
    pub fn validate_ty_metadata_scratch(
        &self,
        ty: &Ty,
        budget: &PlanBudget,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        if !self.budget().same_authority(budget) {
            return Err(FactsError::invalid(
                budget,
                "metadata facts changed planning budget authority",
            ));
        }
        self.world.validate_ty_metadata_scratch(ty, self, scratch)
    }
    pub fn callee_receiver(&self, callee: &Callee) -> Result<ReceiverMode> {
        TraitCx::callee_receiver(self.world.as_ref(), callee, self)
    }
    /// Receiver replay sharing only this validator's exact node-local proofs.
    pub fn callee_receiver_scratch(
        &self,
        callee: &Callee,
        scratch: &mut TraitScratch,
    ) -> Result<ReceiverMode> {
        self.world.check_facts(self, true)?;
        if let Callee::Direct(target) = callee {
            self.world.discharge_direct_with(target, self, scratch)?;
            if let Some(receiver) = self.world.closure_dispatch_receiver(target)? {
                return Ok(receiver);
            }
            return Ok(self.world.info(target.declaration)?.receiver);
        }
        self.callee_receiver(callee)
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
        if let Some(item) = self
            .world
            .headers
            .items
            .iter()
            .find(|item| item.declaration == declaration)
            && self.world.schema_contains_projection(&item.schema)?
        {
            return self
                .world
                .validate_source_regions(declaration, &item.schema.signature);
        }
        Ok(self
            .world
            .declaration_carriers(declaration)?
            .source_regions
            .clone())
    }
    pub fn validate_ty(&self, ty: &Ty) -> Result<()> {
        self.validate_ty_with_budget(ty, self.budget())
    }
    pub fn validate_ty_metadata(&self, ty: &Ty) -> Result<()> {
        self.validate_ty_metadata_with_budget(ty, self.budget())
    }
    pub fn validate_ty_metadata_with_budget(&self, ty: &Ty, budget: &PlanBudget) -> Result<()> {
        if !self.budget().same_authority(budget) {
            return Err(FactsError::invalid(
                budget,
                "metadata facts changed planning budget authority",
            ));
        }
        self.world.check_facts(self, false)?;
        self.world.validate_ty_metadata_with_facts(
            ty,
            self.env(),
            0,
            &mut NominalReplays::default(),
            Some(self),
        )
    }
    pub fn projected_ty(&self, root: &Ty, path: &[ir::Projection]) -> Result<Ty> {
        self.validate_ty(root)?;
        charged(self.budget(), self.budget().charge_work(path.len()))?;
        charged(self.budget(), ir::projected_ty(root, path, self.budget()))
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
        } else if let FactsSubject::UniversalCallable(declaration) = self.subject {
            self.world
                .source_default_item(declaration)?
                .map(|item| {
                    self.world
                        .trait_header(item.trait_decl)
                        .map(|h| h.self_param)
                })
                .transpose()?
        } else {
            None
        };
        let maybe_unsized = match maybe_unsized {
            Some(parameter)
                if self
                    .world
                    .predicates_prove_sized(parameter, &self.predicates.predicates)? =>
            {
                None
            }
            other => other,
        };
        self.world.check_facts(self, false)?;
        self.world
            .validate_ty_inner_with_facts(
                ty,
                self.env(),
                0,
                TypeUse::Root,
                &mut TypeWalk {
                    maybe_unsized,
                    ..TypeWalk::default()
                },
                Some(self),
            )
            .map(|_| ())
    }
    /// Require a Sized value in this exact executable body's obligation
    /// context. Callers must distinguish owned values from legal unsized places
    /// behind references before invoking this operation.
    pub fn require_sized(&self, ty: &Ty) -> Result<()> {
        self.world.check_facts(self, true)?;
        self.validate_ty(ty)?;
        self.world.require_in(
            &Predicate::Implements(TraitRef {
                declaration: self.world.catalog.id(CatalogItem::Sized),
                self_ty: ty.clone(),
                args: Arc::from([]),
            }),
            ProofContext::from_facts(self),
            &mut ProofScratch::default(),
        )?;
        Ok(())
    }
    /// `validate_ty` memoized in a node-local scratch.
    pub fn validate_ty_scratch(&self, ty: &Ty, scratch: &mut TraitScratch) -> Result<()> {
        scratch.for_facts(self);
        scratch.valid.check(self, ty)
    }
    /// `must_copy` memoized in a node-local scratch, sharing its proof memo.
    pub fn must_copy_scratch(&self, ty: &Ty, scratch: &mut TraitScratch) -> Result<bool> {
        if self.phase() == FactsPhase::Schema {
            return self.must_copy(ty);
        }
        scratch.for_facts(self);
        let address = type_allocation(ty);
        if let Some(address) = address
            && let Some(candidates) = scratch.copy_index.get(&address)
        {
            for &index in candidates {
                let (old, answer) = &scratch.copy[index];
                if same_type_allocation(old, ty) {
                    return Ok(*answer);
                }
            }
        }
        self.validate_ty_scratch(ty, scratch)?;
        if matches!(
            ty,
            Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Bool | Ty::Unit | Ty::Never
        ) {
            return Ok(true);
        }
        let predicate = Predicate::Implements(TraitRef {
            declaration: self.world.catalog.id(CatalogItem::Copy),
            self_ty: ty.clone(),
            args: Arc::from([]),
        });
        let answer = matches!(
            self.world.prove_goal_with(&predicate, self, scratch)?,
            ProofOutcome::Proved(_)
        );
        if let Some(address) = address
            && scratch.copy.len() < MAX_SCRATCH_OBLIGATIONS
            && scratch.copy.try_reserve(1).is_ok()
        {
            scratch
                .copy_index
                .entry(address)
                .or_default()
                .push(scratch.copy.len());
            scratch.copy.push((ty.clone(), answer));
        }
        Ok(answer)
    }
    pub fn must_copy(&self, ty: &Ty) -> Result<bool> {
        if self.phase() == FactsPhase::Schema {
            return Err(FactsError::invalid(
                self.budget(),
                "schema facts cannot grant executable Copy permission",
            ));
        }
        self.validate_ty(ty)?;
        // Scalar leaves are Copy by language definition, with no impl
        // selection or environment dependence. Composite types still run the
        // full obligation engine.
        if matches!(
            ty,
            Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Bool | Ty::Unit | Ty::Never
        ) {
            return Ok(true);
        }
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
    OpaqueDeclared,
    Closure {
        kind: ir::ClosureKind,
    },
    /// Builtin Fn/FnMut/FnOnce for a zero-sized function item type.
    FnItem {
        kind: ir::ClosureKind,
    },
    ProjectedSubject {
        evidence: ProofEvidence,
        reductions: Arc<[NormalizedAssociatedType]>,
    },
    Assumption,
    DeclaredEquality,
    ReflexiveEquality {
        membership: ProofEvidence,
    },
    ClosedEquality {
        normalization: NormalizedAssociatedType,
    },
    /// Rigid source-nominal equality selected through exact impl membership.
    ImplementationEquality {
        membership: ProofEvidence,
    },
    Builtin {
        item: CatalogItem,
    },
    Implementation {
        declaration: DeclId,
        arguments: Arc<[Ty]>,
    },
    /// `dyn P: Q` where Q is P or one of its (transitive) supertraits. The
    /// object's vtable provides the methods; dispatch is `Callee::Virtual`.
    TraitObject,
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
    _required: Option<Arc<CarrierContractView>>,
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
    finalized: bool,
    goal: Predicate,
}
#[derive(Clone, Debug)]
struct MemoEntry {
    key: GoalKey,
    result: Result<ProofOutcome>,
}
/// Node-local proof scratch: the DFS ancestry used for cycle detection plus
/// the memo tables for one proof computation. Owned by the caller and passed
/// by `&mut`; never stored in `ValidatedTraitWorld` or any published fact.
#[derive(Default)]
struct ProofScratch {
    checked: Option<Box<ProofScratch>>,
    // Scratch allocation admission ends with this arena, not the whole node.
    _checked_storage: Option<ir::PlanReservation>,
    checked_context: Option<(WorldId, usize, usize, FactsPhase, DeclId)>,
    _checked_env: Option<Arc<GenericEnv>>,
    active: Vec<GoalKey>,
    replays: NominalReplays,
    memo: Vec<MemoEntry>,
    /// Structural-hash prefilter into `memo`. A bucket hit is still
    /// confirmed by `same_key`; a miss only costs a recomputation.
    memo_index: std::collections::HashMap<u64, Vec<usize>>,
    /// Cycle detections so far. A goal whose subtree hit a cycle is
    /// provisional (it depended on its ancestry) and is not memoized.
    cycles: usize,
    projections: Vec<associated_types::NormalizationMemo>,
    normalization_validations: Vec<associated_types::NormalizationValidation>,
    normalization_validation_storage: Vec<ir::PlanReservation>,
    closedness: Vec<ClosednessValidation>,
    closedness_storage: Vec<ir::PlanReservation>,
    /// Successful raw substitutions only; all normalization and proof work
    /// remains outside this node-local retained-input cache.
    instantiations: schema_instantiation::Substitutions,
}

#[derive(Clone, Copy)]
struct Closedness {
    height: usize,
    has_parameters: bool,
    has_opaque: bool,
}

/// Successful root validation within one proof computation. Retained inputs
/// pin every allocation used in the identity test, including the environment.
struct ClosednessValidation {
    world: WorldId,
    ty: Ty,
    env: GenericEnv,
    usage: TypeUse,
    result: Closedness,
}

fn same_closedness_input(left: &Ty, right: &Ty) -> bool {
    if left.is_nominal_back_edge() || right.is_nominal_back_edge() {
        return false;
    }
    if let (Some(left_key), Some(right_key)) = (TypeWalk::key(left), TypeWalk::key(right)) {
        // Equal keys pin the full child allocation. Nominals additionally
        // require the exact realization capability, not just the same def.
        return left_key == right_key && same_type_allocation(left, right);
    }
    match (left, right) {
        (Ty::Opaque(a), Ty::Opaque(b)) => Arc::ptr_eq(a, b),
        (Ty::PhantomData(a), Ty::PhantomData(b)) => Arc::ptr_eq(a, b),
        (Ty::Cell(a), Ty::Cell(b)) => Arc::ptr_eq(a, b),
        (Ty::ManuallyDrop(a), Ty::ManuallyDrop(b)) => Arc::ptr_eq(a, b),
        (Ty::FnPointer(a), Ty::FnPointer(b)) => Arc::ptr_eq(a, b),
        (Ty::FnItem(a), Ty::FnItem(b)) => Arc::ptr_eq(a, b),
        (Ty::AssociatedType(a), Ty::AssociatedType(b)) => Arc::ptr_eq(a, b),
        (Ty::Dyn(a), Ty::Dyn(b)) => Arc::ptr_eq(a, b),
        (Ty::Slice { element: a }, Ty::Slice { element: b }) => Arc::ptr_eq(a, b),
        (
            Ty::RawPtr {
                pointee: a,
                kind: ak,
            },
            Ty::RawPtr {
                pointee: b,
                kind: bk,
            },
        ) => ak == bk && Arc::ptr_eq(a, b),
        (Ty::Param(a), Ty::Param(b)) => a == b,
        (Ty::Int(a), Ty::Int(b)) => a == b,
        (Ty::Float(a), Ty::Float(b)) => a == b,
        (Ty::Char, Ty::Char)
        | (Ty::Bool, Ty::Bool)
        | (Ty::Unit, Ty::Unit)
        | (Ty::Never, Ty::Never)
        | (Ty::Str, Ty::Str) => true,
        _ => false,
    }
}
/// Each child allocation is destroyed before its own child-only lease.
struct RawMetadataArena {
    env: GenericEnv,
    child: Box<NominalReplays>,
    _storage: ir::PlanReservation,
}

/// Buffer ownership and admission travel together. Rust drops fields in
/// declaration order: all children and the actual Vec allocation die before
/// the buffer/header lease. No lease Vec can outlive its own admission.
#[derive(Default)]
struct RawMetadataArenas {
    values: Vec<RawMetadataArena>,
    storage: Option<ir::PlanReservation>,
}
impl RawMetadataArenas {
    fn buffer_bytes(capacity: usize) -> Option<usize> {
        capacity
            .checked_mul(std::mem::size_of::<RawMetadataArena>())?
            .checked_add(2 * std::mem::size_of::<usize>())
    }

    fn push_with(
        &mut self,
        budget: &PlanBudget,
        env: &GenericEnv,
        after_row_growth: impl FnOnce() -> Result<()>,
    ) -> Result<usize> {
        let child_storage = charged(
            budget,
            budget.reserve_bytes(
                std::mem::size_of::<NominalReplays>() + 2 * std::mem::size_of::<usize>(),
            ),
        )?;
        if self.values.len() == self.values.capacity() {
            let capacity = self
                .values
                .capacity()
                .max(1)
                .checked_mul(2)
                .ok_or_else(|| FactsError::resource(budget, "raw arena capacity overflow"))?;
            // Admit the entire replacement, not the capacity delta, while the
            // old allocation and lease remain live. Pay allocator slack up to
            // twice the requested capacity and validate the returned capacity.
            let paid_slots = capacity
                .checked_mul(2)
                .ok_or_else(|| FactsError::resource(budget, "raw arena capacity overflow"))?;
            let prepaid = Self::buffer_bytes(paid_slots)
                .ok_or_else(|| FactsError::resource(budget, "raw arena storage overflow"))?;
            let storage = charged(budget, budget.reserve_bytes(prepaid))?;
            let mut next = Self {
                values: Vec::new(),
                storage: Some(storage),
            };
            charged(budget, budget.charge_work(self.values.len()))?;
            next.values
                .try_reserve_exact(capacity)
                .map_err(|_| FactsError::resource(budget, "raw arena buffer allocation failed"))?;
            // A test-only injection through a no-op production closure. Every
            // error drops next's actual buffer before its lease. Old rows stay.
            after_row_growth()?;
            charged(budget, budget.charge_work(0))?;
            if next.values.capacity() > paid_slots {
                return Err(FactsError::resource(
                    budget,
                    "raw arena allocator exceeded paid capacity",
                ));
            }
            let actual = Self::buffer_bytes(next.values.capacity())
                .ok_or_else(|| FactsError::resource(budget, "raw arena storage overflow"))?;
            charged(
                budget,
                next.storage.as_mut().unwrap().shrink(prepaid - actual),
            )?;
            // Exact returned-capacity admission now owns next. Both leases
            // remain live until old rows have moved and its buffer is freed.
            next.values.append(&mut self.values);
            std::mem::swap(self, &mut next);
            drop(next);
        }
        let index = self.values.len();
        // GenericEnv is an Arc<[Arc<ParamEnv>]> plus scalar arity. Its derived
        // clone only pins shared handles, never clones deep scope arrays.
        self.values.push(RawMetadataArena {
            env: env.clone(),
            child: Box::default(),
            _storage: child_storage,
        });
        Ok(index)
    }
}

/// An explicitly admitted replay buffer. Actual data and allocation always
/// drop before the lease, including replacement-buffer failure paths.
struct MeteredReplayRows<T> {
    values: Vec<T>,
    storage: Option<ir::PlanReservation>,
}
impl<T> Default for MeteredReplayRows<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            storage: None,
        }
    }
}
impl<T> std::ops::Deref for MeteredReplayRows<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.values
    }
}
impl<T> MeteredReplayRows<T> {
    fn buffer_bytes(capacity: usize) -> Option<usize> {
        capacity
            .checked_mul(std::mem::size_of::<T>())?
            .checked_add(std::mem::size_of::<Self>())
    }
    fn prepare(&mut self, world: &ValidatedTraitWorld) -> Result<()> {
        self.prepare_with(world, |_| Ok(()))
    }
    fn prepare_with(
        &mut self,
        world: &ValidatedTraitWorld,
        after_growth: impl FnOnce(usize) -> Result<()>,
    ) -> Result<()> {
        world.check_nominal_replay_issuer()?;
        if self.values.len() < self.values.capacity() {
            return Ok(());
        }
        let budget = &world.budget;
        let requested = self
            .values
            .capacity()
            .max(1)
            .checked_mul(2)
            .ok_or_else(|| FactsError::resource(budget, "replay capacity overflow"))?;
        let paid_slots = requested
            .checked_mul(2)
            .ok_or_else(|| FactsError::resource(budget, "replay capacity overflow"))?;
        let prepaid = Self::buffer_bytes(paid_slots)
            .ok_or_else(|| FactsError::resource(budget, "replay storage overflow"))?;
        charged(budget, budget.charge_work(self.values.len()))?;
        let storage = charged(budget, budget.reserve_bytes(prepaid))?;
        let mut next = Self {
            values: Vec::new(),
            storage: Some(storage),
        };
        next.values
            .try_reserve_exact(requested)
            .map_err(|_| FactsError::resource(budget, "replay buffer allocation failed"))?;
        // Production passes a no-op. Tests fail after the actual allocation,
        // while next still owns its full buffer admission and old is intact.
        after_growth(next.values.capacity())?;
        world.check_nominal_replay_issuer()?;
        if next.values.capacity() > paid_slots {
            return Err(FactsError::resource(
                budget,
                "replay allocator exceeded paid capacity",
            ));
        }
        let actual = Self::buffer_bytes(next.values.capacity())
            .ok_or_else(|| FactsError::resource(budget, "replay storage overflow"))?;
        charged(
            budget,
            next.storage.as_mut().unwrap().shrink(prepaid - actual),
        )?;
        next.values.append(&mut self.values);
        std::mem::swap(self, &mut next);
        drop(next);
        Ok(())
    }
}

/// Realized nominal facts whose world replay already succeeded inside this
/// node's computation, keyed by the retained immutable fact allocation. Owned
/// by the caller's scratch and dropped with it; never in a fact or shared.
/// Failures are never recorded, so a failing replay is always re-reported.
/// Bounded by the distinct nominal facts the node itself read.
#[derive(Default)]
#[allow(clippy::type_complexity)]
pub(in crate::compiler) struct NominalReplays {
    /// Local canonical reconstruction only, never a replacement for payload replay.
    nominal_session: Option<Box<dyn NominalValidationSession>>,
    /// Metadata answers belong to one exact world and original caller context.
    metadata_context: Option<(WorldId, Option<(usize, usize, FactsPhase, DeclId)>)>,
    /// Pin the exact outer environment allocation used by the context key.
    _metadata_env: Option<Arc<GenericEnv>>,
    /// A raw nested computation never borrows caller-authorized records. Its
    /// own exact world/environment arena survives repeated raw replay roots.
    raw_metadata: Option<(WorldId, GenericEnv, Box<NominalReplays>)>,
    // The child never leaves its parent replay computation.
    _raw_metadata_storage: Option<ir::PlanReservation>,
    /// Additional raw environments of this same world. A caller's formation
    /// replay can alternate its own environment with several source binders.
    /// Keep their completed proofs isolated instead of rebuilding every root
    /// after the first environment. Each row owns its scoped admission.
    raw_metadata_others: RawMetadataArenas,
    /// Complete raw formation proofs, distinct from metadata-only answers.
    raw_formations: body_validation::RawFormations,
    /// Full caller-authorized predicate formations never enter the raw memo.
    predicate_formations: TypeWalk,
    // Clones pin each allocation for the memo's lifetime, so pointer
    // identity can never be reused by a different fact.
    facts: MeteredReplayRows<super::nominal_realization::ReadNominal>,
    inputs: MeteredReplayRows<Arc<[Predicate]>>,
    /// Types whose metadata walk already succeeded, with the environment and
    /// height. Clones pin the identities compared by `same_type_allocation`.
    types: MeteredReplayRows<(Ty, GenericEnv, usize)>,
    // Sorted by (hash, insertion ordinal), preserving collision candidate order.
    type_index: MeteredReplayRows<(usize, usize)>,
}
/// Types already fully validated by one local computation (for example one
/// CFG validation pass). Owned by the caller, never published or shared;
/// clones pin the compared allocations. Only successes are recorded.
#[derive(Default)]
pub struct ValidatedTypes {
    types: Vec<Ty>,
    index: std::collections::HashMap<usize, Vec<usize>>,
}
impl ValidatedTypes {
    /// Run `validate` unless an identical allocation already passed.
    pub fn check<E>(
        &mut self,
        ty: &Ty,
        validate: impl FnOnce(&Ty) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), E> {
        let Some(address) = type_allocation(ty) else {
            return validate(ty);
        };
        if let Some(candidates) = self.index.get(&address)
            && candidates
                .iter()
                .any(|&index| same_type_allocation(&self.types[index], ty))
        {
            return Ok(());
        }
        validate(ty)?;
        if self.types.len() < MAX_SCRATCH_OBLIGATIONS && self.types.try_reserve(1).is_ok() {
            self.index
                .entry(address)
                .or_default()
                .push(self.types.len());
            self.types.push(ty.clone());
        }
        Ok(())
    }
}
/// Allocation identity of a type root, or None for leaves not worth caching.
fn type_allocation(ty: &Ty) -> Option<usize> {
    use std::hash::{Hash, Hasher};
    match ty {
        Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Bool | Ty::Unit | Ty::Never | Ty::Str => {
            return None;
        }
        _ => {}
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ty.hash(&mut hasher);
    Some(hasher.finish() as usize)
}
fn same_types(a: &[Ty], b: &[Ty]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_type_allocation(a, b))
}
fn same_trait_call(a: &TraitCall, b: &TraitCall) -> bool {
    a.item == b.item
        && a.trait_ref.declaration == b.trait_ref.declaration
        && same_type_allocation(&a.trait_ref.self_ty, &b.trait_ref.self_ty)
        && same_types(&a.trait_ref.args, &b.trait_ref.args)
        && same_types(&a.method_args, &b.method_args)
}
fn same_callee(a: &Callee, b: &Callee) -> bool {
    match (a, b) {
        (Callee::Direct(a), Callee::Direct(b)) => {
            a.declaration == b.declaration && same_types(&a.type_args, &b.type_args)
        }
        (Callee::TraitItem(a), Callee::TraitItem(b)) => same_trait_call(a, b),
        (Callee::Virtual(a), Callee::Virtual(b)) => same_trait_call(&a.call, &b.call),
        _ => false,
    }
}
/// Equality that is faithful to validation inputs: nominal applications must
/// share the exact retained definition and realization capability (identity
/// alone is not authority); other structure compares recursively. Projections
/// compare their item, trait and exact child authority. Opaque, closure, object
/// and function-item payloads only match the same allocation.
fn same_type_allocation(a: &Ty, b: &Ty) -> bool {
    fn nominal_same<T>(
        x: &T,
        y: &T,
        realization: impl Fn(&T) -> Option<&super::nominal_realization::ReadNominal>,
    ) -> bool {
        match (realization(x), realization(y)) {
            (None, None) => true,
            (Some(x), Some(y)) => x.same_fact(y),
            _ => false,
        }
    }
    fn all(a: &[Ty], b: &[Ty]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_type_allocation(a, b))
    }
    match (a, b) {
        (Ty::Struct(x), Ty::Struct(y)) => {
            x.id() == y.id()
                && std::ptr::eq(x.def(), y.def())
                && nominal_same(x, y, |t| t.realization())
        }
        (Ty::Enum(x), Ty::Enum(y)) => {
            x.id() == y.id()
                && std::ptr::eq(x.def(), y.def())
                && nominal_same(x, y, |t| t.realization())
        }
        (Ty::Tuple(x), Ty::Tuple(y)) => Arc::ptr_eq(x, y) || all(x, y),
        (Ty::Array { element: x, len: a }, Ty::Array { element: y, len: b }) => {
            a == b && (Arc::ptr_eq(x, y) || same_type_allocation(x, y))
        }
        (Ty::Slice { element: x }, Ty::Slice { element: y }) => {
            Arc::ptr_eq(x, y) || same_type_allocation(x, y)
        }
        (
            Ty::Ref {
                referent: x,
                kind: a,
            },
            Ty::Ref {
                referent: y,
                kind: b,
            },
        )
        | (
            Ty::RawPtr {
                pointee: x,
                kind: a,
            },
            Ty::RawPtr {
                pointee: y,
                kind: b,
            },
        ) => a == b && (Arc::ptr_eq(x, y) || same_type_allocation(x, y)),
        (Ty::Param(x), Ty::Param(y)) => x == y,
        (Ty::Int(x), Ty::Int(y)) => x == y,
        (Ty::Float(x), Ty::Float(y)) => x == y,
        (Ty::Char, Ty::Char)
        | (Ty::Bool, Ty::Bool)
        | (Ty::Unit, Ty::Unit)
        | (Ty::Never, Ty::Never)
        | (Ty::Str, Ty::Str) => true,
        (Ty::Opaque(x), Ty::Opaque(y)) => Arc::ptr_eq(x, y),
        (Ty::Closure(x), Ty::Closure(y)) => Arc::ptr_eq(x, y),
        (Ty::PhantomData(x), Ty::PhantomData(y)) => Arc::ptr_eq(x, y),
        (Ty::Cell(x), Ty::Cell(y)) => Arc::ptr_eq(x, y),
        (Ty::ManuallyDrop(x), Ty::ManuallyDrop(y)) => Arc::ptr_eq(x, y),
        (Ty::FnPointer(x), Ty::FnPointer(y)) => Arc::ptr_eq(x, y),
        (Ty::FnItem(x), Ty::FnItem(y)) => Arc::ptr_eq(x, y),
        (Ty::AssociatedType(x), Ty::AssociatedType(y)) => {
            Arc::ptr_eq(x, y)
                || (x.item == y.item
                    && x.trait_ref.declaration == y.trait_ref.declaration
                    && same_type_allocation(&x.trait_ref.self_ty, &y.trait_ref.self_ty)
                    && same_types(&x.trait_ref.args, &y.trait_ref.args))
        }
        (Ty::Dyn(x), Ty::Dyn(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}
impl NominalReplays {
    fn raw_metadata_child(
        &mut self,
        world: &ValidatedTraitWorld,
        env: &GenericEnv,
    ) -> Result<Option<&mut Self>> {
        self.raw_metadata_child_with(world, env, || Ok(()))
    }

    fn raw_metadata_child_with(
        &mut self,
        world: &ValidatedTraitWorld,
        env: &GenericEnv,
        after_row_growth: impl FnOnce() -> Result<()>,
    ) -> Result<Option<&mut Self>> {
        charged(&world.budget, world.budget.charge_work(1))?;
        // Reusing a completed proof never renews a poisoned issuer ledger.
        checked_module(
            &world.budget,
            world
                .catalog
                .modules()
                .budget()
                .check()
                .map_err(ModuleError::from),
        )?;
        if let Some((bound_world, bound_env, _)) = self.raw_metadata.as_ref() {
            if *bound_world != world.id {
                return Ok(None);
            }
            if !world.same_env(bound_env, env)? {
                let mut found = None;
                for (index, row) in self.raw_metadata_others.values.iter().enumerate() {
                    charged(&world.budget, world.budget.charge_work(1))?;
                    if world.same_env(&row.env, env)? {
                        found = Some(index);
                        break;
                    }
                }
                let index = if let Some(index) = found {
                    index
                } else {
                    if self.raw_metadata_others.values.len() >= MAX_SCRATCH_OBLIGATIONS - 1 {
                        // Saturation is a cache miss, never a validation success.
                        return Ok(None);
                    }
                    self.raw_metadata_others
                        .push_with(&world.budget, env, after_row_growth)?
                };
                return Ok(Some(self.raw_metadata_others.values[index].child.as_mut()));
            }
        } else {
            let storage = charged(
                &world.budget,
                world.budget.reserve_bytes(
                    std::mem::size_of::<NominalReplays>() + 2 * std::mem::size_of::<usize>(),
                ),
            )?;
            self.raw_metadata = Some((world.id, env.clone(), Box::default()));
            self._raw_metadata_storage = Some(storage);
        }
        Ok(self
            .raw_metadata
            .as_mut()
            .map(|(_, _, child)| child.as_mut()))
    }
    /// Bind once. A different caller must use a fresh local replay arena,
    /// never retag the arena an enclosing validation is still using.
    fn bind_metadata_caller(&mut self, world: WorldId, facts: Option<&ValidatedFacts>) -> bool {
        let caller = facts.map(|facts| {
            (
                Arc::as_ptr(facts.env()) as usize,
                facts.predicates.index,
                facts.phase(),
                facts.declaration(),
            )
        });
        let context = Some((world, caller));
        if self.metadata_context.is_none() {
            self._metadata_env = facts.map(|facts| facts.env().clone());
            self.metadata_context = context;
        }
        self.metadata_context == context
    }
    #[cfg(test)]
    pub(in crate::compiler) fn replay_counts(&self) -> (usize, usize, usize, usize) {
        (
            self.facts.len(),
            self.inputs.len(),
            self.types.len(),
            self.type_index.len(),
        )
    }
    pub(in crate::compiler) fn contains_fact(
        &self,
        fact: &super::nominal_realization::ReadNominal,
        world: &ValidatedTraitWorld,
    ) -> Result<bool> {
        world.check_nominal_replay_issuer()?;
        for old in self.facts.iter() {
            charged(&world.budget, world.budget.charge_work(1))?;
            if old.same_fact(fact) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(in crate::compiler) fn record_fact(
        &mut self,
        fact: &super::nominal_realization::ReadNominal,
        world: &ValidatedTraitWorld,
    ) -> Result<()> {
        self.admit(world)?;
        self.facts.prepare(world)?;
        self.facts.values.push(fact.clone());
        Ok(())
    }
    fn contains_input(
        &self,
        input: &ClosedNominalInput,
        world: &ValidatedTraitWorld,
    ) -> Result<bool> {
        world.check_nominal_replay_issuer()?;
        for old in self.inputs.iter() {
            charged(&world.budget, world.budget.charge_work(1))?;
            if Arc::ptr_eq(old, input.obligation_identity()) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn record_input(
        &mut self,
        input: &ClosedNominalInput,
        world: &ValidatedTraitWorld,
    ) -> Result<()> {
        self.admit(world)?;
        self.inputs.prepare(world)?;
        self.inputs.values.push(input.obligation_identity().clone());
        Ok(())
    }
    fn type_index_start(&self, address: usize, world: &ValidatedTraitWorld) -> Result<usize> {
        let mut left = 0;
        let mut right = self.type_index.len();
        while left < right {
            charged(&world.budget, world.budget.charge_work(1))?;
            let middle = left + (right - left) / 2;
            if self.type_index[middle].0 < address {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        Ok(left)
    }
    /// Complete source/payload authority and ordered environment still decide
    /// every hit; the sorted hash index only selects candidate ordinals.
    fn validated_type(
        &self,
        world: &ValidatedTraitWorld,
        ty: &Ty,
        env: &GenericEnv,
    ) -> Result<Option<usize>> {
        world.check_nominal_replay_issuer()?;
        let Some(address) = type_allocation(ty) else {
            return Ok(None);
        };
        let start = self.type_index_start(address, world)?;
        for &(hash, index) in &self.type_index[start..] {
            charged(&world.budget, world.budget.charge_work(1))?;
            if hash != address {
                break;
            }
            let (old, old_env, height) = &self.types[index];
            if same_type_allocation(old, ty) && world.same_env(old_env, env)? {
                return Ok(Some(*height));
            }
        }
        Ok(None)
    }
    fn record_type(
        &mut self,
        ty: &Ty,
        env: &GenericEnv,
        height: usize,
        world: &ValidatedTraitWorld,
    ) -> Result<()> {
        self.record_type_with(ty, env, height, world, || Ok(()), || Ok(()))
    }
    #[allow(clippy::too_many_arguments)]
    fn record_type_with(
        &mut self,
        ty: &Ty,
        env: &GenericEnv,
        height: usize,
        world: &ValidatedTraitWorld,
        after_type_growth: impl FnOnce() -> Result<()>,
        after_index_growth: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        world.check_nominal_replay_issuer()?;
        let Some(address) = type_allocation(ty) else {
            return Ok(());
        };
        if self.types.len() >= MAX_SCRATCH_OBLIGATIONS {
            // Preserve best-effort saturation: full validation repeats later.
            return Ok(());
        }
        let budget = &world.budget;
        charged(budget, budget.charge_work(1))?;
        // Equal hashes retain first-completed order, as in the old bucket Vec.
        let mut position = self.type_index_start(address, world)?;
        while position < self.type_index.len() {
            charged(budget, budget.charge_work(1))?;
            if self.type_index[position].0 != address {
                break;
            }
            position += 1;
        }
        charged(budget, budget.charge_work(self.type_index.len() - position))?;
        // All fallible preparations precede BOTH row publications. A failure
        // may leave larger fully paid capacity, never a partial success row.
        self.types.prepare_with(world, |_| after_type_growth())?;
        self.type_index
            .prepare_with(world, |_| after_index_growth())?;
        world.check_nominal_replay_issuer()?;
        let ordinal = self.types.len();
        self.types.values.push((ty.clone(), env.clone(), height));
        self.type_index.values.insert(position, (address, ordinal));
        Ok(())
    }
    fn admit(&self, world: &ValidatedTraitWorld) -> Result<()> {
        world.check_nominal_replay_issuer()?;
        let budget = &world.budget;
        if self.facts.len() + self.inputs.len() >= MAX_SCRATCH_OBLIGATIONS {
            return Err(FactsError::resource(
                budget,
                "nominal replay memo limit for one computation exceeded",
            ));
        }
        charged(budget, budget.charge_work(1))
    }
}
/// Distinct goals one proof computation may memoize. Per scratch (per
/// node), never a shared whole-world ledger.
const MAX_SCRATCH_OBLIGATIONS: usize = 4096;
impl ProofScratch {
    fn charge_obligation(&self, budget: &PlanBudget) -> Result<()> {
        if self.memo.len() + self.projections.len() >= MAX_SCRATCH_OBLIGATIONS {
            return Err(FactsError::resource(
                budget,
                "trait obligation limit for one proof computation exceeded",
            ));
        }
        charged(budget, budget.charge_work(1))
    }
}
/// Caller-owned proof scratch for a sequence of trait queries inside one
/// graph node (for example every call site of one body). It memoizes within
/// that computation only: it is never stored in a fact or shared between
/// nodes, and it resets when used with a different trait world.
#[derive(Default)]
pub struct TraitScratch {
    world: Option<WorldId>,
    proofs: ProofScratch,
    /// Executable type validation (`validate_ty`) successes and Copy
    /// answers for identical type allocations under one facts subject.
    valid: body_validation::Validated,
    /// Signature formation roots use no possibly-unsized Self allowance.
    signature_types: body_validation::Validated,
    copy: Vec<(Ty, bool)>,
    copy_index: std::collections::HashMap<usize, Vec<usize>>,
    /// Successful call-signature validations (subject callee, signature).
    signatures: Vec<(Callee, Signature)>,
    signature_index: std::collections::HashMap<u64, Vec<usize>>,
    /// Successful direct-call discharges under the current facts subject.
    directs: Vec<DirectCallProof>,
    direct_index: std::collections::HashMap<u64, Vec<usize>>,
    /// Canonical signatures produced for exact retained trait-call inputs.
    formed_trait_signatures: Vec<(TraitCall, Signature)>,
    formed_trait_signature_index: std::collections::HashMap<DeclId, Vec<usize>>,
    /// The facts identity these answers belong to (environment + phase).
    facts_key: Option<(usize, FactsPhase, DeclId)>,
}
impl TraitScratch {
    fn for_world(&mut self, world: WorldId) -> &mut ProofScratch {
        if self.world != Some(world) {
            *self = TraitScratch {
                world: Some(world),
                ..TraitScratch::default()
            };
        }
        &mut self.proofs
    }
    fn for_facts(&mut self, facts: &ValidatedFacts) {
        self.for_world(facts.world.id);
        let key = (facts.predicates.index, facts.phase(), facts.declaration());
        if self.facts_key != Some(key) {
            self.facts_key = Some(key);
            self.valid = body_validation::Validated::default();
            self.signature_types = body_validation::Validated::default();
            self.copy.clear();
            self.copy_index.clear();
            self.signatures.clear();
            self.signature_index.clear();
            self.directs.clear();
            self.direct_index.clear();
            self.formed_trait_signatures.clear();
            self.formed_trait_signature_index.clear();
        }
    }
}
impl ProofScratch {
    fn with_caller_arena<T>(
        &mut self,
        world: WorldId,
        caller: Option<&ValidatedFacts>,
        run: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let Some(caller) = caller else {
            return if self.checked_context.is_some() {
                run(&mut Self::default())
            } else {
                run(self)
            };
        };
        let context = (
            world,
            Arc::as_ptr(caller.env()) as usize,
            caller.predicates.index,
            caller.phase(),
            caller.declaration(),
        );
        if self.checked_context == Some(context) {
            return run(self);
        }
        if self.checked_context.is_none() {
            if self.checked.is_none() {
                self._checked_storage = Some(charged(
                    caller.budget(),
                    caller.budget().reserve_bytes(
                        std::mem::size_of::<ProofScratch>() + 2 * std::mem::size_of::<usize>(),
                    ),
                )?);
            }
            let arena = self.checked.get_or_insert_with(|| {
                Box::new(Self {
                    checked_context: Some(context),
                    _checked_env: Some(caller.env().clone()),
                    ..Self::default()
                })
            });
            if arena.checked_context == Some(context) {
                return run(arena);
            }
        }
        // A nested different caller never changes the parent arena or its records.
        run(&mut Self {
            checked_context: Some(context),
            _checked_env: Some(caller.env().clone()),
            ..Self::default()
        })
    }
}
impl std::ops::Deref for ProofScratch {
    type Target = Vec<GoalKey>;
    fn deref(&self) -> &Vec<GoalKey> {
        &self.active
    }
}
impl std::ops::DerefMut for ProofScratch {
    fn deref_mut(&mut self) -> &mut Vec<GoalKey> {
        &mut self.active
    }
}

#[derive(Clone, Copy)]
struct ProofContext<'a> {
    // Only from_facts carries an original caller. Raw predicate contexts never mint one.
    original: Option<&'a ValidatedFacts>,
    predicates: &'a PredicateEnv,
    phase: FactsPhase,
}
impl<'a> ProofContext<'a> {
    fn from_facts(facts: &'a ValidatedFacts) -> Self {
        Self {
            original: (facts.phase() != FactsPhase::Schema).then_some(facts),
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

/// A type-argument invariance obligation, not a mutation/effect capability.
#[derive(Clone, Debug)]
pub(in crate::compiler) struct NativeHeapInvariant {
    pub(in crate::compiler) storage: ir::InputCarrier,
    pub(in crate::compiler) sources: Arc<[ir::InputCarrier]>,
}

/// Structural declaration transport only. Actual loans and occurrence-local
/// region solutions belong to the borrow plan, never this world-owned view.
#[derive(Clone, Debug)]
pub struct CarrierContractView {
    native_heap_invariants: Arc<[NativeHeapInvariant]>,
    nominal: Option<Arc<nominal_carriers::NominalCarrierAuthority>>,
    closure_result: Option<(Arc<ir::ClosureTy>, ir::ClosureKind)>,
    physical: Option<super::opaque_physical::PhysicalCarrierIdentity>,
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
    projected: Option<Arc<projected_methods::ProjectedMethodEvidence>>,
    projected_subject: Option<TraitRef>,
    equality_required: Option<Arc<equality::EqualityRequiredMethodEvidence>>,
    equality_results: Option<Arc<parameter_equality::EqualityResultAuthority>>,
    result_subject: parameter_equality::EqualityResultSubject,
    result_signature: Signature,
    budget: PlanBudget,
}
impl CarrierContractView {
    pub(in crate::compiler) fn native_heap_invariants(&self) -> &[NativeHeapInvariant] {
        &self.native_heap_invariants
    }
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
    /// The exact direct closure-provider call this closure-result view was
    /// replayed for, if any. Its arguments are the provider's own identity
    /// parameters, not the instantiated call target's.
    pub(in crate::compiler) fn closure_result_call(&self) -> Option<&CallTarget> {
        match (&self.closure_result, &self.result_subject) {
            (Some(_), parameter_equality::EqualityResultSubject::Direct(call)) => Some(call),
            _ => None,
        }
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
    /// Live checks for a private borrow-plan receipt of this exact immutable
    /// body view. This does not authenticate a receipt or a different view.
    pub(super) fn check_body_replay_context(&self, facts: &ValidatedFacts) -> Result<()> {
        facts.world.check_facts(facts, true)?;
        facts.world.check_context(ProofContext::from_facts(facts))?;
        facts.world.check_normalization_budget()?;
        checked_module(
            facts.budget(),
            facts
                .world
                .catalog
                .modules()
                .budget()
                .check()
                .map_err(ModuleError::from),
        )?;
        if self.physical.is_some()
            || self.world != facts.world.id
            || self.registry != facts.registry_snapshot()
            || self.environment != facts.predicates.index
            || self.phase != facts.phase()
            || !self.budget.same_authority(facts.budget())
            || !matches!(
                self.result_subject,
                parameter_equality::EqualityResultSubject::Body { .. }
            )
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "body replay receipt has foreign live context",
            ));
        }
        Ok(())
    }
    pub fn validate_for(&self, facts: &ValidatedFacts) -> Result<()> {
        if self.physical.is_some() {
            return Err(FactsError::invalid(
                facts.budget(),
                "physical carrier rows require borrowed normalization replay",
            ));
        }
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
        if let Some(authority) = &self.nominal {
            return authority.validate(self, facts);
        }
        if let Some((closure, mode)) = &self.closure_result {
            return facts
                .world
                .validate_closure_result_carrier(self, closure, *mode, facts);
        }
        if let Some(evidence) = &self.projected {
            facts
                .world
                .validate_projected_carrier(self, evidence, facts)?;
        }
        if let Some(evidence) = &self.equality_required {
            facts
                .world
                .validate_equality_required_carrier(self, evidence, facts)?;
        }
        facts.world.validate_equality_results(self, facts)?;
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
    Closure(usize),
    FnPointer(usize),
    PhantomData(usize),
    ManuallyDrop(usize),
    Cell(usize),
}
#[derive(Default)]
struct TypeWalk {
    nominal_session: Option<Box<dyn NominalValidationSession>>,
    caller_context: Option<Option<(usize, usize, FactsPhase, DeclId)>>,
    _caller_env: Option<Arc<GenericEnv>>,
    caller_world: Option<WorldId>,
    raw_env: Option<GenericEnv>,
    complete: Vec<((TypeNode, TypeUse), usize, Ty)>,
    reservations: Vec<ir::PlanReservation>,
    visits: usize,
    maybe_unsized: Option<ParamId>,
    has_parameters: bool,
    has_opaque: bool,
}
impl TypeWalk {
    fn growth_slots(len: usize, capacity: usize) -> Option<usize> {
        if len < capacity {
            Some(0)
        } else if len == capacity {
            len.checked_add(len.max(1)).map(|_| len.max(1))
        } else {
            None
        }
    }
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
            Ty::Cell(cell) => Some(TypeNode::Cell(Arc::as_ptr(cell) as usize)),
            Ty::PhantomData(marker) => Some(TypeNode::PhantomData(Arc::as_ptr(marker) as usize)),
            Ty::ManuallyDrop(wrapper) => {
                Some(TypeNode::ManuallyDrop(Arc::as_ptr(wrapper) as usize))
            }
            Ty::FnPointer(pointer) => Some(TypeNode::FnPointer(Arc::as_ptr(pointer) as usize)),
            Ty::Closure(closure) => Some(TypeNode::Closure(Arc::as_ptr(closure) as usize)),
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
    fn lookup(
        &self,
        key: (TypeNode, TypeUse),
        ty: &Ty,
        budget: &PlanBudget,
    ) -> Result<Option<usize>> {
        for (old, height, retained) in &self.complete {
            charged(budget, budget.charge_work(1))?;
            if *old == key && schema_instantiation::same_input(retained, ty, budget, 0)? {
                return Ok(Some(*height));
            }
        }
        Ok(None)
    }
    fn insert(
        &mut self,
        key: (TypeNode, TypeUse),
        height: usize,
        ty: &Ty,
        budget: &PlanBudget,
    ) -> Result<()> {
        let reservation = charged(
            budget,
            budget.reserve_bytes(
                2 * std::mem::size_of::<((TypeNode, TypeUse), usize, Ty)>()
                    + 2 * std::mem::size_of::<ir::PlanReservation>()
                    + 32,
            ),
        )?;
        // The per-entry receipt pays two slots in both vectors. Doubling only
        // at capacity therefore remains paid after publication of this entry.
        // During relocation the old allocation is still live, so separately
        // admit that temporary peak before requesting either new allocation.
        let complete_growth = Self::growth_slots(self.complete.len(), self.complete.capacity())
            .ok_or_else(|| FactsError::resource(budget, "type memo capacity overflow"))?;
        let reservation_growth =
            Self::growth_slots(self.reservations.len(), self.reservations.capacity())
                .ok_or_else(|| FactsError::resource(budget, "type memo capacity overflow"))?;
        let grow_complete = complete_growth != 0;
        let grow_reservations = reservation_growth != 0;
        let old_complete = if grow_complete {
            self.complete.len()
        } else {
            0
        };
        let old_reservations = if grow_reservations {
            self.reservations.len()
        } else {
            0
        };
        let relocation = old_complete
            .checked_add(old_reservations)
            .ok_or_else(|| FactsError::resource(budget, "type memo relocation overflow"))?;
        let temporary_bytes = old_complete
            .checked_mul(std::mem::size_of::<((TypeNode, TypeUse), usize, Ty)>())
            .and_then(|bytes| {
                old_reservations
                    .checked_mul(std::mem::size_of::<ir::PlanReservation>())
                    .and_then(|other| bytes.checked_add(other))
            })
            .ok_or_else(|| FactsError::resource(budget, "type memo storage overflow"))?;
        let _relocation_storage = charged(budget, budget.reserve_bytes(temporary_bytes))?;
        charged(budget, budget.charge_work(relocation))?;
        if grow_complete {
            self.complete
                .try_reserve_exact(complete_growth)
                .map_err(|_| FactsError::resource(budget, "type memo allocation failed"))?;
        }
        if grow_reservations {
            self.reservations
                .try_reserve_exact(reservation_growth)
                .map_err(|_| {
                    FactsError::resource(budget, "type memo reservation allocation failed")
                })?;
        }
        let paid_slots = self
            .complete
            .len()
            .checked_add(1)
            .and_then(|len| len.checked_mul(2))
            .ok_or_else(|| FactsError::resource(budget, "type memo capacity overflow"))?;
        if self.complete.capacity() > paid_slots || self.reservations.capacity() > paid_slots {
            return Err(FactsError::resource(
                budget,
                "type memo allocator exceeded paid capacity",
            ));
        }
        // Retain every completed input, including freshly materialized fields.
        // A reused walk must never treat a recycled allocation address as proof.
        self.complete.push((key, height, ty.clone()));
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
    environment_index: DeclIndex,
    empty: Arc<PredicateEnv>,
    requirements: Vec<(DeclId, Arc<[Predicate]>)>,
    requirement_index: DeclIndex,
    origins: Vec<(DeclId, Arc<ir::SignatureOriginSchema>)>,
    origin_index: DeclIndex,
    carriers: Vec<DeclarationCarriers>,
    carrier_index: DeclIndex,
    normalized_callables: Vec<CallableHeader>,
    /// `(declaration, first position)` sorted by declaration, rebuilt with
    /// `normalized_callables`. Replaces a linear scan per header lookup.
    normalized_callable_index: Vec<(DeclId, usize)>,
    owner_normalization_ready: bool,
    owner_normalization_finalized: bool,
    has_associated_bounds: bool,
    has_normal_drop: bool,
    declared_callables: Vec<equality::DeclaredCallableContext>,
    constant_header_index: DeclIndex,
    constant_impl_index: std::collections::HashMap<(DeclId, DeclId), DeclId>,
    declared_callable_index: DeclIndex,
    /// Trait declaration to impl header positions (TRAIT_IMPLS source).
    trait_impl_index: std::collections::HashMap<DeclId, Vec<usize>>,
    /// Owner declaration to its callables, in header order. Built once so
    /// per-impl ownership checks are linear, not impls x callables.
    owned_callables: std::collections::HashMap<DeclId, Vec<DeclId>>,
    /// Source default provider to the trait item positions naming it.
    default_item_index: std::collections::HashMap<DeclId, Vec<usize>>,
    /// First header position of each trait, item, impl and callable
    /// declaration, so header lookups are not linear scans.
    trait_header_index: DeclIndex,
    item_header_index: DeclIndex,
    impl_header_index: DeclIndex,
    callable_header_index: DeclIndex,
    /// Whole-world impl overlap at build time; the graph defers it to
    /// per-impl coherence nodes.
    eager_coherence: bool,
}

impl ValidatedTraitWorld {
    /// Immutable implementation headers retained by this validated world.
    pub(in crate::compiler) fn canonical_catalog(&self) -> &CoreCatalog {
        &self.catalog
    }
    pub fn impl_headers(&self) -> &[ImplHeader] {
        &self.headers.impls
    }

    /// Every immutable header of this validated world, including the core
    /// catalog. Read-only: consumed by the semantic core adapter.
    pub fn all_headers(&self) -> &HeaderSet {
        &self.headers
    }

    pub fn build(
        headers: HeaderSet,
        schema: Arc<dyn SchemaCx + Send + Sync>,
        budget: &PlanBudget,
    ) -> Result<Arc<Self>> {
        let world = charged(
            budget,
            crate::compiler::owner_path::standalone_issued(
                &crate::compiler::expansion_budget::ExpansionBudget::default(),
                crate::compiler::owner_path::OwnerDomain::TraitWorld,
            )
            .map_err(|error| error.to_string()),
        )?;
        Self::build_with(headers, schema, budget, true, world)
    }
    /// `eager_coherence = false` leaves impl overlap to per-impl checks.
    pub fn build_with(
        mut headers: HeaderSet,
        schema: Arc<dyn SchemaCx + Send + Sync>,
        budget: &PlanBudget,
        eager_coherence: bool,
        world: crate::compiler::owner_path::Issued,
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
        // Issued by the producing node (or a standalone root for direct
        // tooling): never a compiler-global counter.
        let id = WorldId(SnapshotId::issued(world));
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
            environment_index: DeclIndex::default(),
            empty,
            requirements: Vec::new(),
            requirement_index: DeclIndex::default(),
            origins: Vec::new(),
            origin_index: DeclIndex::default(),
            carriers: Vec::new(),
            carrier_index: DeclIndex::default(),
            normalized_callables: Vec::new(),
            normalized_callable_index: Vec::new(),
            owner_normalization_ready: false,
            owner_normalization_finalized: false,
            has_associated_bounds: false,
            has_normal_drop: false,
            declared_callables: Vec::new(),
            constant_header_index: DeclIndex::default(),
            constant_impl_index: std::collections::HashMap::new(),
            declared_callable_index: DeclIndex::default(),
            trait_impl_index: std::collections::HashMap::new(),
            owned_callables: std::collections::HashMap::new(),
            default_item_index: std::collections::HashMap::new(),
            trait_header_index: DeclIndex::default(),
            item_header_index: DeclIndex::default(),
            impl_header_index: DeclIndex::default(),
            callable_header_index: DeclIndex::default(),
            eager_coherence,
        };
        for (position, h) in world.headers.traits.iter().enumerate() {
            world
                .trait_header_index
                .note(budget, h.declaration, position)?;
        }
        for (position, h) in world.headers.items.iter().enumerate() {
            world
                .item_header_index
                .note(budget, h.declaration, position)?;
        }
        for (position, h) in world.headers.impls.iter().enumerate() {
            world
                .impl_header_index
                .note(budget, h.declaration, position)?;
        }
        for (position, h) in world.headers.callables.iter().enumerate() {
            world
                .callable_header_index
                .note(budget, h.declaration, position)?;
        }
        for (position, h) in world.headers.impls.iter().enumerate() {
            charged(budget, budget.charge_work(1))?;
            world
                .trait_impl_index
                .entry(h.trait_ref.declaration)
                .or_default()
                .push(position);
        }
        for (position, item) in world.headers.items.iter().enumerate() {
            if let Some(default) = item.default {
                charged(budget, budget.charge_work(1))?;
                world
                    .default_item_index
                    .entry(default)
                    .or_default()
                    .push(position);
            }
        }
        for callable in world.headers.callables.iter() {
            if let Some(owner) = world.info(callable.declaration)?.owner {
                world
                    .owned_callables
                    .entry(owner)
                    .or_default()
                    .push(callable.declaration);
            }
        }
        world.build_constant_index()?;
        world.validate_header_type_metadata()?;
        world.validate_headers()?;
        world.validate_constant_headers()?;
        world.validate_associated_headers()?;
        world.validate_supertrait_graph()?;
        world.build_declared_environments()?;
        world.build_declared_callable_contexts()?;
        world.validate_projected_owner_metadata()?;
        world.owner_normalization_ready = true;
        world.stage_associated_types()?;
        world.build_origin_schemas()?;
        for h in world.headers.impls.iter() {
            world.validate_impl(h)?;
        }
        for h in world.headers.impls.iter() {
            let env = world.predicate_env(h.declaration)?;
            let context = ProofContext {
                original: None,
                predicates: &env,
                phase: FactsPhase::Universal,
            };
            world.require_impl_wf(h, &h.trait_ref, context, &mut ProofScratch::default())?;
        }
        world.build_environments()?;
        world.build_constant_environments()?;
        world.validate_final_declared_contexts()?;
        world.validate_default_body_signatures()?;
        for h in world.headers.impls.iter() {
            if h.trait_ref.declaration != world.catalog.id(CatalogItem::Drop) {
                continue;
            }
            world.has_normal_drop = true;
            let owner_args = match &h.trait_ref.self_ty {
                Ty::Struct(owner) => owner.def().type_args.clone(),
                Ty::Enum(owner) => owner.def().type_args.clone(),
                _ => {
                    return Err(FactsError::invalid(
                        budget,
                        "E0120: the Drop trait may only be implemented for local structs, enums, and unions",
                    ));
                }
            };
            // Drop must cover the complete nominal parameter space, never a
            // specialization such as Owner<i32> or Owner<(T,T)>.
            if h.schema_env.total_arity() != owner_args.len() {
                return Err(FactsError::invalid(
                    budget,
                    "canonical Drop cannot specialize owner arguments",
                ));
            }
            for (index, argument) in owner_args.iter().enumerate() {
                charged(budget, budget.charge_work(index + 1))?;
                if !matches!(argument, Ty::Param(param) if h.schema_env.contains(*param))
                    || owner_args[..index].contains(argument)
                {
                    return Err(FactsError::invalid(
                        budget,
                        "canonical Drop requires distinct owner parameters",
                    ));
                }
            }
            for predicate in h.predicates.iter() {
                charged(budget, budget.charge_work(1))?;
                if !matches!(predicate,
                    Predicate::Implements(trait_ref)
                        if trait_ref.declaration == world.catalog.id(CatalogItem::Sized)
                            && trait_ref.args.is_empty()
                            && matches!(trait_ref.self_ty, Ty::Param(param) if h.schema_env.contains(param)))
                {
                    return Err(FactsError::unsupported(
                        budget,
                        "unsupported: generic Drop with additional owner predicates",
                    ));
                }
            }
        }
        world.owner_normalization_finalized = true;
        let world = Arc::new(world);
        world.validate_constant_implementations()?;
        Ok(world)
    }
    fn check_nominal_replay_issuer(&self) -> Result<()> {
        self.check_normalization_budget()?;
        charged(&self.budget, self.budget.charge_work(0))?;
        checked_module(
            &self.budget,
            self.catalog
                .modules()
                .budget()
                .check()
                .map_err(ModuleError::from),
        )
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
    /// Callables whose registry owner is `owner`, in header order.
    pub(super) fn owned_callables(&self, owner: DeclId) -> &[DeclId] {
        self.owned_callables.get(&owner).map_or(&[], Vec::as_slice)
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
        checked_module(
            &self.budget,
            info.visibility
                .validate_for(self.catalog.modules(), info.module),
        )?;
        if let Some(canonical) = self.catalog.declaration(id) {
            let c = &canonical.info;
            if info.kind != c.kind
                || info.owner != c.owner
                || info.receiver != c.receiver
                || info.visibility != c.visibility
                || info.module != c.module
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
        } else if !id.is_source() {
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
        if let Some(position) = self.trait_header_index.get(&self.budget, id)? {
            return Ok(&self.headers.traits[position]);
        }
        Err(FactsError::invalid(
            &self.budget,
            "trait declaration is not in the validated world",
        ))
    }
    fn item_header(&self, id: DeclId) -> Result<&TraitItemHeader> {
        if let Some(position) = self.item_header_index.get(&self.budget, id)? {
            return Ok(&self.headers.items[position]);
        }
        Err(FactsError::invalid(
            &self.budget,
            "required trait item is not in the validated world",
        ))
    }
    fn callable_header(&self, id: DeclId) -> Result<&CallableHeader> {
        charged(&self.budget, self.budget.charge_work(1))?;
        let first = self
            .normalized_callable_index
            .partition_point(|(declaration, _)| *declaration < id);
        if let Some((declaration, position)) = self.normalized_callable_index.get(first)
            && *declaration == id
        {
            return Ok(&self.normalized_callables[*position]);
        }
        if let Some(position) = self.callable_header_index.get(&self.budget, id)? {
            return Ok(&self.headers.callables[position]);
        }
        Err(FactsError::invalid(
            &self.budget,
            "declaration has no executable callable schema",
        ))
    }
    fn predicate_env(&self, id: DeclId) -> Result<Arc<PredicateEnv>> {
        if let Some(position) = self.environment_index.get(&self.budget, id)? {
            return Ok(self.environments[position].clone());
        }
        Err(FactsError::invalid(
            &self.budget,
            "declaration has no authoritative predicate environment",
        ))
    }
    fn trait_arguments(&self, goal: &TraitRef) -> Result<Vec<Ty>> {
        let header = self.trait_header(goal.declaration)?;
        if goal.args.len().checked_add(1) != Some(header.env.total_arity()) {
            return Err(FactsError::invalid(
                &self.budget,
                "trait argument arity mismatch",
            ));
        }
        let mut arguments = Vec::new();
        push(&self.budget, &mut arguments, goal.self_ty.clone())?;
        for argument in goal.args.iter() {
            push(&self.budget, &mut arguments, argument.clone())?;
        }
        Ok(arguments)
    }
    fn symbolic_trait_ref(&self, declaration: DeclId) -> Result<TraitRef> {
        let header = self.trait_header(declaration)?;
        let mut args = Vec::new();
        for parameter in header.env.ordered_params().skip(1) {
            push(&self.budget, &mut args, Ty::Param(parameter))?;
        }
        Ok(TraitRef {
            declaration,
            self_ty: Ty::Param(header.self_param),
            args: args.into(),
        })
    }
    fn validate_ref_in(
        &self,
        goal: &TraitRef,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
    ) -> Result<()> {
        self.validate_ref_with(goal, env, maybe_unsized, &mut NominalReplays::default())
    }
    fn validate_ref_with(
        &self,
        goal: &TraitRef,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
        replays: &mut NominalReplays,
    ) -> Result<()> {
        self.validate_ref_with_facts(goal, env, maybe_unsized, replays, None)
    }
    fn validate_ref_with_facts(
        &self,
        goal: &TraitRef,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
        replays: &mut NominalReplays,
        caller: Option<&ValidatedFacts>,
    ) -> Result<()> {
        self.validate_ty_metadata_with_facts(&goal.self_ty, env, 0, replays, caller)?;
        for argument in goal.args.iter() {
            self.validate_ty_metadata_with_facts(argument, env, 0, replays, caller)?;
        }
        let declaration = self.info(goal.declaration)?;
        if declaration.kind != DeclKind::Trait {
            return Err(FactsError::invalid(
                &self.budget,
                "trait reference does not name a trait",
            ));
        }
        let header = self.trait_header(goal.declaration)?;
        self.validate_predicate_formation(
            &goal.self_ty,
            env,
            TypeUse::Root,
            maybe_unsized,
            replays,
            caller,
        )?;
        for argument in goal.args.iter() {
            if matches!(argument, Ty::Str) {
                // Preserve the raw actual metadata allowance and its charge.
                charged(&self.budget, self.budget.charge_work(1))?;
            } else {
                self.validate_predicate_formation(
                    argument,
                    env,
                    TypeUse::Actual,
                    None,
                    replays,
                    caller,
                )?;
            }
            charged(&self.budget, ir::reject_references(argument, &self.budget))?;
        }
        if goal.args.len().checked_add(1) != Some(header.env.total_arity()) {
            return Err(FactsError::invalid(
                &self.budget,
                "trait argument arity mismatch",
            ));
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn validate_predicate_formation(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        usage: TypeUse,
        maybe_unsized: Option<ParamId>,
        replays: &mut NominalReplays,
        caller: Option<&ValidatedFacts>,
    ) -> Result<()> {
        if caller.is_none() {
            return replays
                .raw_formations
                .check(self, ty, env, usage, maybe_unsized, 0);
        }
        if replays.predicate_formations.maybe_unsized != maybe_unsized {
            // Different formation policy cannot reuse previous successes.
            let mut local = TypeWalk {
                maybe_unsized,
                ..TypeWalk::default()
            };
            return self
                .validate_ty_inner_with_facts(ty, env, 0, usage, &mut local, caller)
                .map(|_| ());
        }
        self.validate_ty_inner_with_facts(
            ty,
            env,
            0,
            usage,
            &mut replays.predicate_formations,
            caller,
        )
        .map(|_| ())
    }
    fn validate_predicates(
        &self,
        predicates: &[Predicate],
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
    ) -> Result<()> {
        self.validate_predicates_with(
            predicates,
            env,
            maybe_unsized,
            &mut NominalReplays::default(),
        )
    }
    fn validate_predicates_with(
        &self,
        predicates: &[Predicate],
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
        replays: &mut NominalReplays,
    ) -> Result<()> {
        self.validate_predicates_with_facts(predicates, env, maybe_unsized, replays, None)
    }
    fn validate_predicates_with_facts(
        &self,
        predicates: &[Predicate],
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
        replays: &mut NominalReplays,
        caller: Option<&ValidatedFacts>,
    ) -> Result<()> {
        self.validate_predicates_policy(
            predicates,
            env,
            maybe_unsized,
            replays,
            caller,
            |projection, ty, replays| {
                self.validate_equality_domain_with_facts(projection, ty, env, replays, caller)
            },
        )
    }
    fn validate_predicates_with_active(
        &self,
        predicates: &[Predicate],
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
        active: &mut ProofScratch,
        caller: Option<&ValidatedFacts>,
    ) -> Result<()> {
        let ProofScratch {
            replays,
            closedness,
            closedness_storage,
            ..
        } = active;
        self.validate_predicates_policy(
            predicates,
            env,
            maybe_unsized,
            replays,
            caller,
            |projection, ty, replays| {
                self.validate_equality_domain_with_storage(
                    projection,
                    ty,
                    env,
                    replays,
                    caller,
                    closedness,
                    closedness_storage,
                )
            },
        )
    }
    fn validate_predicates_policy(
        &self,
        predicates: &[Predicate],
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
        replays: &mut NominalReplays,
        caller: Option<&ValidatedFacts>,
        mut equality_domain: impl FnMut(
            &ir::AssociatedTypeProjection,
            &Ty,
            &mut NominalReplays,
        ) -> Result<()>,
    ) -> Result<()> {
        if let Some(caller) = caller {
            self.check_facts(caller, false)?;
            if !caller.budget().same_authority(&self.budget) || !self.same_env(env, caller.env())? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "predicate caller changed environment or budget authority",
                ));
            }
        }
        let mut canonical = Vec::new();
        for predicate in predicates {
            self.validate_predicate_metadata_with_facts(predicate, env, replays, caller)?;
        }
        for predicate in predicates {
            match predicate {
                Predicate::Implements(goal) => self.validate_predicate_ref_with_facts(
                    goal,
                    env,
                    maybe_unsized,
                    replays,
                    caller,
                )?,
                Predicate::ProjectionEq { projection, ty } => {
                    equality_domain(projection, ty, replays)?;
                }
            }
            self.append_predicate(&mut canonical, predicate)?;
        }
        self.validate_equality_conflicts(predicates)?;
        Ok(())
    }
    /// Inspect canonical contextual bounds without invoking type validation or
    /// the goal solver. Merely being a trait Self does not establish Sized.
    fn predicates_prove_sized(&self, parameter: ParamId, predicates: &[Predicate]) -> Result<bool> {
        for predicate in predicates {
            let Predicate::Implements(goal) = predicate else {
                continue;
            };
            charged(&self.budget, self.budget.charge_work(1))?;
            if goal.declaration == self.catalog.id(CatalogItem::Sized)
                && goal.args.is_empty()
                && matches!(&goal.self_ty, Ty::Param(p) if *p == parameter)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Headers precede authoritative environments. Close only declared bounds,
    /// including transitive owner supertraits, using the bounded charged walker.
    /// Never recurse through validation/proving or manufacture a Sized premise.
    fn schema_maybe_unsized(
        &self,
        parameter: ParamId,
        owner: DeclId,
        predicates: &[Predicate],
        caller: &GenericEnv,
    ) -> Result<Option<ParamId>> {
        let mut declared = Vec::new();
        self.append_predicate(
            &mut declared,
            &Predicate::Implements(self.symbolic_trait_ref(owner)?),
        )?;
        for predicate in predicates {
            self.append_predicate(&mut declared, predicate)?;
        }
        self.close_predicates(&mut declared, caller)?;
        Ok((!self.predicates_prove_sized(parameter, &declared)?).then_some(parameter))
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
    fn validate_phantom_sized_argument(
        &self,
        ty: &Ty,
        caller: Option<&ValidatedFacts>,
        depth: usize,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if let Some(caller) = caller {
            retain::<Ty>(&self.budget, 0)?;
            let outcome = self.prove_in(
                &Predicate::Implements(TraitRef {
                    declaration: self.catalog.id(CatalogItem::Sized),
                    self_ty: ty.clone(),
                    args: Arc::from([]),
                }),
                ProofContext::from_facts(caller),
                &mut ProofScratch::default(),
            )?;
            return match outcome {
                ProofOutcome::Proved(_) => Ok(()),
                ProofOutcome::NotProven => Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: PhantomData argument requires an authoritative Sized proof",
                )),
            };
        }
        let child = |ty: &Ty| self.validate_phantom_sized_argument(ty, None, depth + 1);
        let sized = match ty {
            Ty::Param(parameter) => {
                if let Some(declared) = self.catalog.declared_sized_bound(*parameter)? {
                    return if declared {
                        Ok(())
                    } else {
                        Err(FactsError::unsupported(
                            &self.budget,
                            "unsupported: canonical parameter has no declared Sized bound",
                        ))
                    };
                }
                let owner = parameter.owner.declaration;
                let info = self.info(owner)?;
                match info.kind {
                    DeclKind::Struct | DeclKind::Enum => {
                        let schema = self.schema.nominal_declaration_schema(owner)?;
                        self.predicates_prove_sized(*parameter, &schema.predicates)?
                    }
                    DeclKind::Function | DeclKind::AssociatedFunction | DeclKind::TraitItem => {
                        let schema = self.schema.declaration_schema(owner)?;
                        self.predicates_prove_sized(*parameter, &schema.predicates)?
                    }
                    DeclKind::TraitImpl => {
                        let position = self
                            .impl_header_index
                            .get(&self.budget, owner)?
                            .ok_or_else(|| {
                                FactsError::invalid(
                                    &self.budget,
                                    "implementation Sized parameter has no retained header",
                                )
                            })?;
                        let header = &self.headers.impls[position];
                        if header.declaration != owner
                            || info.own_env.binder() != parameter.owner
                            || !info.own_env.contains(*parameter)
                            || !header.schema_env.contains(*parameter)
                            || !self.same_env(&info.env, &header.schema_env)?
                        {
                            return Err(FactsError::invalid(
                                &self.budget,
                                "implementation Sized parameter has foreign header or binder authority",
                            ));
                        }
                        // Source header construction retains its exact implicit
                        // Sized predicates before world validation starts. Read
                        // those declarations, never infer a default from flags.
                        self.predicates_prove_sized(*parameter, &header.predicates)?
                    }
                    DeclKind::Trait => {
                        let header = self.trait_header(owner)?;
                        let mut sized = false;
                        for bound in header.supertraits.iter() {
                            charged(&self.budget, self.budget.charge_work(1))?;
                            if bound.declaration == self.catalog.id(CatalogItem::Sized)
                                && bound.args.is_empty()
                                && matches!(bound.self_ty, Ty::Param(p) if p == *parameter)
                            {
                                sized = true;
                            }
                        }
                        sized
                    }
                    _ => false,
                }
            }
            Ty::Tuple(parts) => {
                for ty in parts.iter() {
                    child(ty)?;
                }
                true
            }
            Ty::Array { element, .. } => {
                child(element)?;
                true
            }
            Ty::Struct(nominal) => {
                for field in nominal
                    .def()
                    .fields
                    .read(&self.budget)
                    .map_err(|e| FactsError::from_message(&self.budget, &e))?
                    .iter()
                {
                    child(&field.ty)?;
                }
                true
            }
            Ty::Enum(nominal) => {
                for variant in nominal.def().variants.iter() {
                    for field in variant
                        .fields
                        .read(&self.budget)
                        .map_err(|e| FactsError::from_message(&self.budget, &e))?
                        .iter()
                    {
                        child(&field.ty)?;
                    }
                }
                true
            }
            Ty::Cell(_)
            | Ty::ManuallyDrop(_)
            | Ty::Ref { .. }
            | Ty::Closure(_)
            | Ty::PhantomData(_)
            | Ty::FnPointer(_)
            | Ty::RawPtr { .. }
            | Ty::Int(_)
            | Ty::Float(_)
            | Ty::Char
            | Ty::Bool
            | Ty::Unit
            | Ty::Never => true,
            _ => false,
        };
        if sized {
            Ok(())
        } else {
            Err(FactsError::unsupported(
                &self.budget,
                "unsupported: PhantomData argument requires an authoritative Sized proof",
            ))
        }
    }
    fn validate_actual(&self, ty: &Ty, env: &GenericEnv) -> Result<()> {
        // A bare `str` root actual is well-formed metadata. The binder's
        // implicit `Sized` row rejects it unless the parameter is `?Sized`.
        if matches!(ty, Ty::Str) {
            charged(&self.budget, self.budget.charge_work(1))?;
            return Ok(());
        }
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
        self.validate_ty_inner_with_facts(ty, env, depth, usage, walk, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn validate_ty_inner_with_facts(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
        usage: TypeUse,
        walk: &mut TypeWalk,
        caller: Option<&ValidatedFacts>,
    ) -> Result<usize> {
        if let Some(caller) = caller {
            self.check_facts(caller, false)?;
            if !self.same_env(env, caller.env())? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "checked type caller environment changed",
                ));
            }
        }
        let context = caller.map(|f| {
            (
                Arc::as_ptr(f.env()) as usize,
                f.predicates.index,
                f.phase(),
                f.declaration(),
            )
        });
        if let Some(bound) = walk.caller_context {
            let original_env = walk._caller_env.as_deref().or(walk.raw_env.as_ref());
            let same_env = match original_env {
                Some(original) => self.same_env(original, env)?,
                None => false,
            };
            if bound != context || walk.caller_world != Some(self.id) || !same_env {
                let mut nested = TypeWalk {
                    maybe_unsized: walk.maybe_unsized,
                    ..TypeWalk::default()
                };
                return self.validate_ty_inner_with_facts(
                    ty,
                    env,
                    depth,
                    usage,
                    &mut nested,
                    caller,
                );
            }
        } else {
            walk.caller_context = Some(context);
            walk._caller_env = caller.map(|f| f.env().clone());
            walk.caller_world = Some(self.id);
            if caller.is_none() {
                // GenericEnv clones only its Arc-backed scope list, not scopes.
                charged(&self.budget, self.budget.charge_work(1))?;
                retain::<GenericEnv>(&self.budget, 1)?;
                walk.raw_env = Some(env.clone());
            }
        }
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        if let Some(height) = self.closed_component_type_height(ty)? {
            charged(
                &self.budget,
                self.budget.check_depth(depth.saturating_add(height)),
            )?;
            return Ok(height);
        }
        if ty.is_nominal_back_edge() {
            return Ok(0);
        }
        walk.visits = walk
            .visits
            .checked_add(1)
            .ok_or_else(|| FactsError::resource(&self.budget, "type walk visit count overflow"))?;
        let key = TypeWalk::key(ty).map(|node| (node, usage));
        if let Some(key) = key
            && let Some(height) = walk.lookup(key, ty, &self.budget)?
        {
            charged(
                &self.budget,
                self.budget.check_depth(depth.saturating_add(height)),
            )?;
            return Ok(height);
        }
        let nominal = match ty {
            Ty::Struct(t) => Some((t.def().id.snapshot, t.def().source_decl.snapshot)),
            Ty::Enum(t) => Some((t.def().id.snapshot, t.def().source_decl.snapshot)),
            _ => None,
        };
        if let Some((snapshot, declaration_snapshot)) = nominal {
            if snapshot != self.registry || declaration_snapshot != self.registry {
                return Err(FactsError::invalid(
                    &self.budget,
                    "foreign nominal identity",
                ));
            }
            validate_nominal_modules(ty, self.catalog.modules(), &self.budget)?;
            if walk.nominal_session.is_none() {
                walk.nominal_session = self.schema.nominal_validation_session()?;
            }
            if let Some(session) = &mut walk.nominal_session {
                session.validate_node(ty, env, caller, &self.budget)?;
            } else if let Some(caller) = caller {
                self.schema
                    .validate_nominal_node_with_facts(ty, caller, &self.budget)?;
            } else {
                self.schema.validate_nominal_node(ty, env, &self.budget)?;
            }
        }
        let maybe_unsized = walk.maybe_unsized;
        let mut height = 0usize;
        let mut child = |t: &Ty, next_use: TypeUse| -> Result<()> {
            height = height.max(
                self.validate_ty_inner_with_facts(t, env, depth + 1, next_use, walk, caller)?
                    .saturating_add(1),
            );
            Ok(())
        };
        match ty {
            Ty::Opaque(opaque) => {
                self.validate_opaque_metadata(opaque, env)?;
                for ty in opaque.types() {
                    child(ty, TypeUse::Root)?;
                }
                walk.has_opaque = true;
            }
            Ty::Closure(closure) => {
                self.validate_closure_metadata(closure, env)?;
                for ty in closure.types() {
                    child(ty, TypeUse::Root)?;
                }
            }
            Ty::Cell(cell) => {
                cell.validate(&self.catalog, &self.budget)
                    .map_err(|error| FactsError::from_message(&self.budget, &error))?;
                if !cell.is_concrete() {
                    self.catalog.validate_cell_schema_environment(
                        env,
                        caller.map(|facts| facts.declaration()),
                    )?;
                }
                child(cell.argument(), TypeUse::Actual)?;
            }
            Ty::ManuallyDrop(wrapper) => {
                wrapper
                    .validate(&self.catalog, &self.budget)
                    .map_err(|error| FactsError::from_message(&self.budget, &error))?;
                child(wrapper.argument(), TypeUse::Actual)?;
                self.validate_phantom_sized_argument(wrapper.argument(), caller, depth + 1)?;
            }
            Ty::PhantomData(marker) => {
                marker
                    .validate(&self.catalog, &self.budget)
                    .map_err(|e| FactsError::from_message(&self.budget, &e))?;
                child(marker.argument(), TypeUse::Actual)?;
                self.validate_phantom_sized_argument(marker.argument(), caller, depth + 1)?;
            }
            Ty::FnPointer(pointer) => {
                pointer
                    .validate(&self.budget)
                    .map_err(|e| FactsError::from_message(&self.budget, &e))?;
                if pointer.inputs.len() > 32 {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "function pointer parameter limit exceeded",
                    ));
                }
                for ty in pointer.inputs.iter() {
                    child(ty, TypeUse::Aggregate)?;
                }
                child(
                    &pointer.output,
                    if matches!(pointer.output, Ty::Never) {
                        TypeUse::Root
                    } else {
                        TypeUse::Aggregate
                    },
                )?;
            }
            Ty::FnItem(item) => {
                self.info(item.declaration)?;
                for ty in item.type_args.iter() {
                    child(ty, TypeUse::Root)?;
                }
            }
            Ty::AssociatedType(projection) => {
                self.validate_universal_projection(projection, env)?;
                walk.has_parameters = true;
            }
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
                if *len > ir::MAX_ARRAY_TYPE_LEN {
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
            Ty::Slice { element } => {
                // Unsized: only meaningful directly behind a reference, as a
                // root trait Self type (`impl Tr for [T]`), or as an actual for
                // a `?Sized` parameter (`Deref<Target = [T]>`). A parameter
                // without `?Sized` still rejects it through its implicit Sized row.
                if !matches!(
                    usage,
                    TypeUse::Referent { .. } | TypeUse::Root | TypeUse::Actual
                ) {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: slice type outside a reference",
                    ));
                }
                child(element, TypeUse::Aggregate)?;
                // Symbolic parameters remain checked in their exact environment.
                // A concrete instantiation is checked again, including references
                // hidden in nominal fields, before it can carry slice elements.
                charged(&self.budget, ir::reject_references(element, &self.budget))?;
            }
            Ty::Ref { kind, referent } => {
                let depth = match usage {
                    TypeUse::Root | TypeUse::Aggregate => 1,
                    TypeUse::Referent { depth, .. } => depth.checked_add(1).ok_or_else(|| {
                        FactsError::resource(&self.budget, "reference value depth overflow")
                    })?,
                    TypeUse::Actual => 1,
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
            // A raw pointer is a Sized, Copy, loan-free scalar. Its pointee is
            // validated as metadata only and may be unsized.
            Ty::RawPtr { pointee, .. } => {
                child(pointee, TypeUse::Root)?;
            }
            Ty::Struct(t) => {
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
                for field in nominal_fields(&t.def().fields, &self.budget)?.iter() {
                    child(&field.ty, TypeUse::Aggregate)?;
                }
            }
            Ty::Enum(t) => {
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
                    for field in nominal_fields(&variant.fields, &self.budget)?.iter() {
                        child(&field.ty, TypeUse::Aggregate)?;
                    }
                }
            }
            Ty::Str => {
                // Unsized: only meaningful directly behind a shared reference.
                if !matches!(
                    usage,
                    TypeUse::Referent { shared: true, .. } | TypeUse::Root | TypeUse::Actual
                ) {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: str type outside a shared reference",
                    ));
                }
            }
            Ty::Dyn(object) => {
                // Unsized: only meaningful directly behind a reference, as the
                // root Self of a trait goal (`dyn T: T` dispatch schema), or as a
                // nominal actual. An actual for a parameter without `?Sized`
                // is still rejected by that parameter's implicit Sized row.
                if !matches!(
                    usage,
                    TypeUse::Referent { .. } | TypeUse::Root | TypeUse::Actual
                ) {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: trait object type outside a reference",
                    ));
                }
                let header = self.trait_header(object.principal)?;
                if header.declaration != object.principal {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "foreign trait object principal",
                    ));
                }
                self.validate_ty_metadata_in(ty, env, depth)?;
                for value in object.types() {
                    child(value, TypeUse::Root)?;
                }
            }
            Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Bool | Ty::Unit | Ty::Never => {}
        }
        if let Some(key) = key {
            walk.insert(key, height, ty, &self.budget)?;
        }
        Ok(height)
    }
    fn validate_signature_shape(
        &self,
        signature: &Signature,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
    ) -> Result<()> {
        self.validate_signature_shape_with_facts(signature, env, maybe_unsized, None)
    }
    fn validate_signature_shape_with_facts(
        &self,
        signature: &Signature,
        env: &GenericEnv,
        maybe_unsized: Option<ParamId>,
        caller: Option<&ValidatedFacts>,
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
            self.validate_ty_inner_with_facts(t, env, 0, TypeUse::Root, &mut walk, caller)?;
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
                &format!(
                    "declaration return origins are not the complete rigid canonical contract for {declaration:?}: signature={signature:?}, derived={derived:?}"
                ),
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
        // Raw declaration schemas over a projection (`&Self::Target`) keep
        // unresolved return origins until the exact application normalizes.
        let projected = charged(
            &self.budget,
            ir::contains_associated_types(&signature.result, &self.budget),
        )?;
        if row != contract.input_regions.len() || results != contract.results.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "reference contract has missing or extra positions",
            ));
        }
        for (depth, result) in contract.results.iter().enumerate() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if usize::from(result.position.depth) != depth
                || (!projected && matches!(result.returns, ir::ReturnContract::NoReference))
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
    // Required results whose region was freshly introduced by substituting an
    // erased reference type argument (bound to no input, origin-free) carry
    // no claim the provider can contradict. Every other result must refine
    // exactly, input regions must agree, and the provider may only add the
    // outlives facts implied by its own input nesting.
    fn erased_result_regions_refine(
        &self,
        required: &ir::RefContract,
        provided: &ir::RefContract,
    ) -> Result<bool> {
        if required.input_regions != provided.input_regions
            || required.results.len() != provided.results.len()
            || required.static_regions != provided.static_regions
        {
            return Ok(false);
        }
        charged(
            &self.budget,
            self.budget
                .charge_work(required.results.len() * (required.input_regions.len() + 1) + 1),
        )?;
        let mut erased_any = false;
        for (want, have) in required.results.iter().zip(provided.results.iter()) {
            if want.position != have.position {
                return Ok(false);
            }
            let fresh = !required
                .input_regions
                .iter()
                .any(|row| row.region == want.region);
            let empty = matches!(
                &want.returns,
                ir::ReturnContract::SharedInputEnvelopes(inputs) if inputs.is_empty()
            );
            if fresh && empty {
                if !matches!(
                    have.returns,
                    ir::ReturnContract::SharedInputEnvelopes(_)
                        | ir::ReturnContract::SharedInputPaths(_)
                ) {
                    return Ok(false);
                }
                erased_any = true;
                continue;
            }
            if want.region != have.region
                || !self.return_origin_refines(&want.returns, &have.returns, provided)?
            {
                return Ok(false);
            }
        }
        Ok(erased_any)
    }
    fn return_refines(
        &self,
        required: &ir::RefContract,
        provided: &ir::RefContract,
    ) -> Result<bool> {
        if !self.same_regions(required, provided)? {
            return Ok(false);
        }
        let provided_contract = provided;
        for (required, provided) in required.results.iter().zip(provided.results.iter()) {
            if !self.return_origin_refines(
                &required.returns,
                &provided.returns,
                provided_contract,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    // A method may accept independently quantified input lifetimes where the
    // trait requires one shared lifetime. This is contravariance, not erased
    // lifetime identity. Restrict this proof to reference-free return values.
    fn reference_free_implementation_refines(
        &self,
        required: &Signature,
        provided: &Signature,
        env: &GenericEnv,
        original: Option<&ValidatedFacts>,
    ) -> Result<bool> {
        if self.return_refines(&required.ref_contract, &provided.ref_contract)? {
            return Ok(true);
        }
        let a = &required.ref_contract;
        let b = &provided.ref_contract;
        if !a.results.is_empty()
            || !b.results.is_empty()
            || a.input_regions.len() != b.input_regions.len()
        {
            return Ok(false);
        }
        for result in [&required.result, &provided.result] {
            if !self.authenticated_result_carriers_absent(result, env, original)? {
                return Ok(false);
            }
        }
        // Map each provided binder to the required binder at the same typed
        // input position. A provided equality cannot identify distinct required
        // lifetimes. Separate provided binders may instantiate to the same one.
        let mut map = Vec::new();
        for (required, provided) in a.input_regions.iter().zip(b.input_regions.iter()) {
            charged(&self.budget, self.budget.charge_work(map.len() + 1))?;
            if required.input != provided.input {
                return Ok(false);
            }
            if let Some((_, mapped)) = map.iter().find(|(old, _)| *old == provided.region) {
                if *mapped != required.region {
                    return Ok(false);
                }
            } else {
                push(&self.budget, &mut map, (provided.region, required.region))?;
            }
        }
        if map.len() != usize::from(b.region_count) {
            return Ok(false);
        }
        // Both declarations have canonical transitively closed outlives rows.
        // Every condition imposed by the implementation must follow from the
        // required contract after substitution, including reflexive equalities.
        for (long, short) in b.outlives.iter() {
            charged(
                &self.budget,
                self.budget
                    .charge_work(map.len() * 2 + a.outlives.len() + 1),
            )?;
            let Some((_, long)) = map.iter().find(|(old, _)| old == long) else {
                return Ok(false);
            };
            let Some((_, short)) = map.iter().find(|(old, _)| old == short) else {
                return Ok(false);
            };
            if long != short && !a.outlives.contains(&(*long, *short)) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn authenticated_result_carriers_absent(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        original: Option<&ValidatedFacts>,
    ) -> Result<bool> {
        // Replay exact source/graph authority before inspecting field payloads.
        self.validate_ty_metadata_with_facts(ty, env, 0, &mut NominalReplays::default(), original)?;
        let mut visits = 0;
        let (opaque, _type_height) = charged(
            &self.budget,
            ir::reference_type_summary(ty, false, &self.budget, 0, &mut visits),
        )?;
        if opaque {
            return Ok(false);
        }
        self.result_carriers_absent(ty)
    }
    #[cfg(test)]
    pub(in crate::compiler) fn result_carriers_absent_for_tests(
        &self,
        ty: &Ty,
        facts: &ValidatedFacts,
    ) -> Result<bool> {
        self.check_facts(facts, false)?;
        self.authenticated_result_carriers_absent(ty, facts.env(), Some(facts))
    }
    // Finite inventory, not a recursive success cache. Deduplication only
    // avoids scheduling an application twice. No pending application proves
    // absence: final success requires exhausting every queued child.
    fn result_carriers_absent(&self, ty: &Ty) -> Result<bool> {
        let mut queue = Vec::new();
        let mut inventory = Vec::new();
        self.enqueue_result_carrier(ty, 0, &mut queue, &mut inventory)?;
        let mut cursor = 0;
        while cursor < queue.len() {
            let (current, depth) = queue[cursor].clone();
            cursor += 1;
            charged(&self.budget, self.budget.charge_work(1))?;
            let mut enqueue = |child: &Ty| {
                self.enqueue_result_carrier(child, depth + 1, &mut queue, &mut inventory)
            };
            match &current {
                Ty::Ref { .. }
                | Ty::Param(_)
                | Ty::AssociatedType(_)
                | Ty::Opaque(_)
                | Ty::Str
                | Ty::Slice { .. }
                | Ty::Dyn(_)
                | Ty::RawPtr { .. } => return Ok(false),
                Ty::Tuple(fields) => {
                    for field in fields.iter() {
                        enqueue(field)?;
                    }
                }
                Ty::Array { element, .. } => enqueue(element)?,
                Ty::Struct(value) => {
                    for actual in value.def().type_args.iter() {
                        enqueue(actual)?;
                    }
                    let fields = nominal_fields(&value.def().fields, &self.budget)?;
                    for field in fields.iter() {
                        enqueue(&field.ty)?;
                    }
                }
                Ty::Enum(value) => {
                    for actual in value.def().type_args.iter() {
                        enqueue(actual)?;
                    }
                    for variant in value.def().variants.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        let fields = nominal_fields(&variant.fields, &self.budget)?;
                        for field in fields.iter() {
                            enqueue(&field.ty)?;
                        }
                    }
                }
                Ty::Cell(cell) => enqueue(cell.argument())?,
                Ty::ManuallyDrop(wrapper) => enqueue(wrapper.argument())?,
                Ty::FnPointer(_) | Ty::PhantomData(_) => {}
                Ty::FnItem(item) => {
                    for actual in item.type_args.iter() {
                        enqueue(actual)?;
                    }
                }
                Ty::Closure(closure) => {
                    for capture in closure.captures() {
                        enqueue(capture)?;
                    }
                }
                Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Bool | Ty::Unit | Ty::Never => {}
            }
        }
        Ok(true)
    }
    fn enqueue_result_carrier(
        &self,
        ty: &Ty,
        depth: usize,
        queue: &mut Vec<(Ty, usize)>,
        inventory: &mut Vec<ir::NominalAppId>,
    ) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        let nominal = match ty {
            Ty::Struct(value) => Some(value.id()),
            Ty::Enum(value) => Some(value.id()),
            _ => None,
        };
        if let Some(id) = nominal {
            charged(&self.budget, self.budget.charge_work(inventory.len() + 1))?;
            if inventory.contains(&id) {
                // Scheduling deduplication, not a success proof.
                return Ok(());
            }
            push(&self.budget, inventory, id)?;
        }
        push(&self.budget, queue, (ty.clone(), depth))
    }
    fn return_origin_refines(
        &self,
        required: &ir::ReturnContract,
        provided: &ir::ReturnContract,
        contract: &ir::RefContract,
    ) -> Result<bool> {
        use ir::ReturnContract::*;
        self.charge_returns(required)?;
        self.charge_returns(provided)?;
        let (allowed, shared) = match required {
            SharedInputEnvelopes(inputs)
                if matches!(provided, SharedInputPaths(_) | SharedInputEnvelopes(_)) =>
            {
                (inputs, true)
            }
            MutInputEnvelopes(inputs)
                if matches!(provided, MutInputPaths(_) | MutInputEnvelopes(_)) =>
            {
                (inputs, false)
            }
            _ => return Ok(required == provided),
        };
        let region_of = |reference: ir::InputReference| {
            contract
                .input_regions
                .iter()
                .find(|row| row.input == reference)
                .map(|row| row.region)
        };
        // A shared result may come from a reference whose declared region
        // outlives the region of an allowed input: `&'0 &'1 T` with `'1: '0`
        // returning the `'1` referent is a valid `'0` borrow.
        let contains = |reference: ir::InputReference| -> Result<bool> {
            for input in allowed.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                if input.input == reference {
                    return Ok(true);
                }
            }
            if shared && let Some(region) = region_of(reference) {
                for input in allowed.iter() {
                    charged(
                        &self.budget,
                        self.budget.charge_work(contract.outlives.len() + 1),
                    )?;
                    if let Some(target) = region_of(input.input)
                        && contract.outlives.contains(&(region, target))
                    {
                        return Ok(true);
                    }
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
            (Ty::AssociatedType(a), Ty::AssociatedType(b)) => Ok(a.item == b.item
                && a.trait_ref.declaration == b.trait_ref.declaration
                && self.eq_ty(&a.trait_ref.self_ty, &b.trait_ref.self_ty, depth + 1)?
                && self.eq_types(&a.trait_ref.args, &b.trait_ref.args, depth + 1)?),
            (Ty::Opaque(a), Ty::Opaque(b)) => a.try_eq(b, |a, b| self.eq_ty(a, b, depth + 1)),
            (Ty::PhantomData(a), Ty::PhantomData(b)) => {
                a.try_eq(b, |a, b| self.eq_ty(a, b, depth + 1))
            }
            (Ty::ManuallyDrop(a), Ty::ManuallyDrop(b)) => {
                a.try_eq(b, |a, b| self.eq_ty(a, b, depth + 1))
            }
            (Ty::Cell(a), Ty::Cell(b)) => a.try_eq(b, |a, b| self.eq_ty(a, b, depth + 1)),
            (Ty::FnPointer(a), Ty::FnPointer(b)) => a.try_eq(b, |a, b| self.eq_ty(a, b, depth + 1)),
            (Ty::Closure(a), Ty::Closure(b)) => a.try_eq(b, |a, b| self.eq_ty(a, b, depth + 1)),
            (Ty::FnItem(a), Ty::FnItem(b)) => Ok(a.declaration == b.declaration
                && self.eq_types(&a.type_args, &b.type_args, depth + 1)?),
            (Ty::Param(a), Ty::Param(b)) => Ok(a == b),
            (Ty::Int(a), Ty::Int(b)) => Ok(a == b),
            (Ty::Float(a), Ty::Float(b)) => Ok(a == b),
            (Ty::Char, Ty::Char)
            | (Ty::Str, Ty::Str)
            | (Ty::Bool, Ty::Bool)
            | (Ty::Unit, Ty::Unit)
            | (Ty::Never, Ty::Never) => Ok(true),
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
            (Ty::Slice { element: a }, Ty::Slice { element: b }) => self.eq_ty(a, b, depth + 1),
            (Ty::Dyn(a), Ty::Dyn(b)) => {
                if a.principal != b.principal
                    || a.associated_types.len() != b.associated_types.len()
                    || !self.eq_types(&a.args, &b.args, depth + 1)?
                {
                    return Ok(false);
                }
                for ((ai, av), (bi, bv)) in a.associated_types.iter().zip(b.associated_types.iter())
                {
                    if ai != bi || !self.eq_ty(av, bv, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
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
            ) => Ok(a == b && self.eq_ty(at, bt, depth + 1)?),
            (
                Ty::RawPtr {
                    kind: a,
                    pointee: at,
                },
                Ty::RawPtr {
                    kind: b,
                    pointee: bt,
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
        match (a, b) {
            (
                Predicate::ProjectionEq {
                    projection: a,
                    ty: at,
                },
                Predicate::ProjectionEq {
                    projection: b,
                    ty: bt,
                },
            ) => {
                return Ok(self.same_projection(a, b)? && self.eq_ty(at, bt, 0)?);
            }
            (Predicate::Implements(_), Predicate::ProjectionEq { .. })
            | (Predicate::ProjectionEq { .. }, Predicate::Implements(_)) => return Ok(false),
            _ => {}
        }
        let (Predicate::Implements(a), Predicate::Implements(b)) = (a, b) else {
            unreachable!()
        };
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
        let mut seen = std::collections::HashSet::new();
        for id in self
            .headers
            .traits
            .iter()
            .map(|h| h.declaration)
            .chain(self.headers.items.iter().map(|h| h.declaration))
            .chain(self.headers.associated_types.iter().map(|h| h.declaration))
            .chain(self.headers.impls.iter().map(|h| h.declaration))
            .chain(self.headers.callables.iter().map(|h| h.declaration))
        {
            charged(&self.budget, self.budget.charge_work(1))?;
            if !seen.insert(id) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "duplicate trait-world header declaration",
                ));
            }
        }
        for h in self.headers.traits.iter() {
            let info = self.info(h.declaration)?;
            if info.kind != DeclKind::Trait
                || !self.same_env(&info.env, &h.env)?
                || !h.env.contains(h.self_param)
                || h.self_param.owner.declaration != h.declaration
                || h.self_param.index != 0
                || h.env.total_arity() == 0
                || h.env.total_arity() > 32
                || h.env.total_arity() != info.own_env.arity()
                || h.env.scopes().len() != 1
                || h.env.ordered_params().next() != Some(h.self_param)
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
                || !self.method_environment_matches(
                    &item.schema.env,
                    &trait_header.env,
                    &info.own_env,
                )?
                || !self.same_env(&info.env, &item.schema.env)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "required trait item schema/owner mismatch",
                ));
            }
            self.validate_declaration_schema(item.declaration, &item.schema)?;
            // Equality rows are immutable required obligations. Their domain is
            // checked below and again after the exact caller substitution.
            let projected = self.schema_contains_projection(&item.schema)?;
            if projected {
                self.validate_projected_required_schema(item)?;
            }
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
                && self.catalog.declaration(item.declaration).is_none()
            {
                self.validate_source_default(item, default)?;
            }
            let maybe_unsized = self.schema_maybe_unsized(
                trait_header.self_param,
                item.trait_decl,
                &item.schema.predicates,
                &item.schema.env,
            )?;
            if !projected {
                self.validate_schema_signature(
                    item.declaration,
                    &item.schema.signature,
                    &item.schema.env,
                    maybe_unsized,
                )?;
            }
            self.validate_predicates(&item.schema.predicates, &item.schema.env, maybe_unsized)?;
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
            let default_self = match self.source_default_item(callable.declaration)? {
                Some(item) => self.schema_maybe_unsized(
                    self.trait_header(item.trait_decl)?.self_param,
                    item.trait_decl,
                    &callable.schema.predicates,
                    &callable.schema.env,
                )?,
                None => None,
            };
            if !self.signature_contains_projection(&callable.schema.signature)? {
                self.validate_schema_signature(
                    callable.declaration,
                    &callable.schema.signature,
                    &callable.schema.env,
                    default_self,
                )?;
            } else {
                for ty in callable
                    .schema
                    .signature
                    .params
                    .iter()
                    .chain(std::iter::once(&callable.schema.signature.result))
                {
                    self.validate_raw_associated_ty(ty, &callable.schema.env, 0)?;
                }
            }
            if let ReceiverMode::Borrowed { kind } = info.receiver
                && !matches!(callable.schema.signature.params.first(),
                    Some(Ty::Ref { kind: actual, .. }) if *actual == kind)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "borrowed callable receiver disagrees with canonical parameter zero",
                ));
            }
            self.validate_predicates(
                &callable.schema.predicates,
                &callable.schema.env,
                default_self,
            )?;
            if self.catalog.declaration(callable.declaration).is_none()
                && info
                    .owner
                    .is_some_and(|owner| self.headers.traits.iter().any(|h| h.declaration == owner))
                && self.source_default_item(callable.declaration)?.is_none()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait-owned callable is not a registered source default provider",
                ));
            }
        }
        if self.eager_coherence {
            for (index, a) in self.headers.impls.iter().enumerate() {
                for b in self.headers.impls[..index].iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if a.trait_ref.declaration == b.trait_ref.declaration {
                        self.check_impl_pair(a, b)?;
                    }
                }
            }
        }
        Ok(())
    }
    /// Every implementation header of `trait_decl`, in header order. This is
    /// the explicit completeness fact behind lookups that need all impls.
    pub fn trait_impls(&self, trait_decl: DeclId) -> Vec<DeclId> {
        self.trait_impl_index
            .get(&trait_decl)
            .map(|positions| {
                positions
                    .iter()
                    .map(|&p| self.headers.impls[p].declaration)
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Trait of one implementation header, if it is a source impl.
    pub fn impl_trait(&self, implementation: DeclId) -> Option<DeclId> {
        self.headers
            .impls
            .iter()
            .find(|h| h.declaration == implementation)
            .map(|h| h.trait_ref.declaration)
    }
    /// Coherence of one impl against every other impl of the same trait.
    pub fn check_impl_coherence(&self, implementation: DeclId, peers: &[DeclId]) -> Result<()> {
        let find = |id: DeclId| {
            self.headers
                .impls
                .iter()
                .find(|h| h.declaration == id)
                .ok_or_else(|| FactsError::invalid(&self.budget, "unknown impl header"))
        };
        let a = find(implementation)?;
        for &peer in peers {
            charged(&self.budget, self.budget.charge_work(1))?;
            if peer == implementation {
                continue;
            }
            let b = find(peer)?;
            if a.trait_ref.declaration == b.trait_ref.declaration {
                self.check_impl_pair(a, b)?;
            }
        }
        Ok(())
    }
    fn check_impl_pair(&self, a: &ImplHeader, b: &ImplHeader) -> Result<()> {
        let mut bindings = Vec::new();
        let mut overlaps = self.unify(
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
        )?;
        if a.trait_ref.args.len() != b.trait_ref.args.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "trait impl head arity mismatch",
            ));
        }
        for (left, right) in a.trait_ref.args.iter().zip(b.trait_ref.args.iter()) {
            if !overlaps {
                break;
            }
            overlaps = self.unify(
                Term { side: 0, ty: left },
                Term { side: 1, ty: right },
                &mut bindings,
                0,
            )?;
        }
        if overlaps {
            return Err(FactsError::ambiguous(
                &self.budget,
                "overlapping trait impl heads (predicate absence cannot establish disjointness)",
            ));
        }
        Ok(())
    }
    /// A source default is a distinct executable declaration, not executable
    /// permission attached to the signature-only trait item.
    fn source_default_item(&self, declaration: DeclId) -> Result<Option<&TraitItemHeader>> {
        let mut found = None;
        let positions = self
            .default_item_index
            .get(&declaration)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for item in positions.iter().map(|&p| &self.headers.items[p]) {
            charged(&self.budget, self.budget.charge_work(1))?;
            if item.default == Some(declaration)
                && self.catalog.declaration(item.declaration).is_none()
            {
                if found.is_some() {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "source default provider is shared by multiple trait items",
                    ));
                }
                found = Some(item);
            }
        }
        Ok(found)
    }
    fn validate_source_default(&self, item: &TraitItemHeader, default: DeclId) -> Result<()> {
        let info = self.info(default)?;
        let callable = self.callable_header(default)?;
        let required_schema = if info.own_env.arity() == 0 {
            item.schema.clone()
        } else {
            let owner = self.trait_header(item.trait_decl)?;
            if self.info(item.declaration)?.own_env.arity() != info.own_env.arity()
                || !self.method_environment_matches(
                    &callable.schema.env,
                    &owner.env,
                    &info.own_env,
                )?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "generic default provider binder mismatch",
                ));
            }
            let substituted = self.alpha_rename_method_schema(
                item,
                &self.symbolic_trait_ref(item.trait_decl)?,
                &self.method_arguments(default)?,
                &callable.schema.env,
            )?;
            GenericSchema {
                env: callable.schema.env.clone(),
                signature: substituted.signature,
                predicates: substituted.predicates,
            }
        };
        if self.catalog.declaration(default).is_some()
            || info.kind != DeclKind::AssociatedFunction
            || info.owner != Some(item.trait_decl)
            || info.receiver != item.receiver
            || !self.same_parameters(&callable.schema.env, &required_schema.env)?
            || !self.eq_signature(&callable.schema.signature, &required_schema.signature)?
            || self.source_default_item(default)?.map(|h| h.declaration) != Some(item.declaration)
        {
            return Err(FactsError::invalid(
                &self.budget,
                "source default provider owner, binders or signature disagree with trait item",
            ));
        }
        // Both schemas may explicitly spell implicit bounds. Compare their
        // closed obligation sets, adding only the owning trait and Sized rules.
        let mut required = Vec::new();
        for predicate in required_schema.predicates.iter() {
            self.append_predicate(&mut required, predicate)?;
        }
        self.implicit_sized(&required_schema.env, &mut required)?;
        self.append_predicate(
            &mut required,
            &Predicate::Implements(self.symbolic_trait_ref(item.trait_decl)?),
        )?;
        self.close_predicates(&mut required, &required_schema.env)?;
        let mut provided = Vec::new();
        for predicate in callable.schema.predicates.iter() {
            self.append_predicate(&mut provided, predicate)?;
        }
        self.implicit_sized(&callable.schema.env, &mut provided)?;
        self.append_predicate(
            &mut provided,
            &Predicate::Implements(self.symbolic_trait_ref(item.trait_decl)?),
        )?;
        self.close_predicates(&mut provided, &callable.schema.env)?;
        for (left, right) in [(&required, &provided), (&provided, &required)] {
            for predicate in left.iter() {
                let mut present = false;
                for candidate in right.iter() {
                    if self.eq_predicate(predicate, candidate)? {
                        present = true;
                        break;
                    }
                }
                if !present {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "source default provider changes trait item obligations",
                    ));
                }
            }
        }
        Ok(())
    }
    /// A declaration may describe `fn f(self)` for a possibly-unsized Self,
    /// but an executable default cannot receive or return that Self by value.
    /// Only declared method bounds or owning-trait supertraits can discharge
    /// this requirement, never the existence of a default body itself.
    fn validate_default_body_signatures(&self) -> Result<()> {
        for item in self.headers.items.iter() {
            let Some(default) = item.default else {
                continue;
            };
            if self.catalog.declaration(item.declaration).is_some() {
                continue;
            }
            let self_param = self.trait_header(item.trait_decl)?.self_param;
            for ty in item
                .schema
                .signature
                .params
                .iter()
                .chain(std::iter::once(&item.schema.signature.result))
            {
                charged(&self.budget, self.budget.charge_work(1))?;
                if matches!(ty, Ty::Param(parameter) if *parameter == self_param) {
                    let env = self.predicate_env(default)?;
                    if matches!(
                        self.prove_in(
                            &Predicate::Implements(TraitRef {
                                declaration: self.catalog.id(CatalogItem::Sized),
                                self_ty: ty.clone(),
                                args: Arc::from([]),
                            }),
                            ProofContext {
                                original: None,
                                predicates: &env,
                                phase: FactsPhase::Universal,
                            },
                            &mut ProofScratch::default(),
                        )?,
                        ProofOutcome::NotProven
                    ) {
                        return Err(FactsError::unproved(
                            &self.budget,
                            "trait default body takes or returns Self by value without a proven Sized bound",
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
        // Local source traits may be implemented for any type (orphan rule:
        // the trait is local). Canonical foreign traits need a local nominal.
        if !matches!(h.trait_ref.self_ty, Ty::Struct(_) | Ty::Enum(_))
            && self.catalog.declaration(h.trait_ref.declaration).is_some()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "canonical foreign trait implementation requires local nominal Self",
            ));
        }
        if h.trait_ref.declaration == self.catalog.id(CatalogItem::Sized) {
            return Err(FactsError::invalid(
                &self.budget,
                "explicit canonical Sized implementations are invalid",
            ));
        }
        self.validate_ref_in(&h.trait_ref, &h.schema_env, None)?;
        self.validate_predicates(&h.predicates, &h.schema_env, None)?;
        self.validate_impl_parameter_constraints(h)?;
        if h.trait_ref.declaration == self.catalog.id(CatalogItem::Sync) {
            let arguments = match &h.trait_ref.self_ty {
                Ty::Struct(owner) => &owner.def().type_args,
                Ty::Enum(owner) => &owner.def().type_args,
                _ => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "canonical Sync implementation requires local nominal Self",
                    ));
                }
            };
            if h.schema_env.total_arity() != arguments.len() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "E0366: canonical Sync implementations cannot specialize nominal arguments",
                ));
            }
            for (index, argument) in arguments.iter().enumerate() {
                charged(&self.budget, self.budget.charge_work(index + 1))?;
                if !matches!(argument, Ty::Param(parameter) if h.schema_env.contains(*parameter))
                    || arguments[..index].contains(argument)
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "E0366: canonical Sync implementations require distinct nominal parameters",
                    ));
                }
            }
        }
        let trait_header = self.trait_header(h.trait_ref.declaration)?;
        let (implementation_unsafe, source_trait_unsafe) = self
            .schema
            .impl_trait_safety(h.declaration, h.trait_ref.declaration)?;
        let trait_unsafe = if self.catalog.declaration(h.trait_ref.declaration).is_some() {
            self.catalog
                .trait_requires_unsafe_impl(h.trait_ref.declaration)?
        } else {
            source_trait_unsafe
        };
        if trait_unsafe && !implementation_unsafe {
            return Err(FactsError::invalid(
                &self.budget,
                "E0200: the trait requires an unsafe impl declaration",
            ));
        }
        if implementation_unsafe && !trait_unsafe {
            return Err(FactsError::invalid(
                &self.budget,
                "E0199: implementing this trait is not unsafe",
            ));
        }
        for id in trait_header.items.iter() {
            let item = self.item_header(*id)?;
            if item.default.is_none() && !h.items.iter().any(|(mapped, _)| mapped == id) {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait impl has missing required items",
                ));
            }
        }
        for owned in self.owned_callables(h.declaration) {
            {
                charged(&self.budget, self.budget.charge_work(h.items.len()))?;
                if !h.items.iter().any(|(_, target)| target == owned) {
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
            let owner_info = self.info(h.declaration)?;
            let private_visibility = match &target_info.visibility {
                super::ir::Visibility::Public => false,
                super::ir::Visibility::Restricted(scope) => {
                    checked_module(&self.budget, self.catalog.modules().check(*scope))?;
                    *scope == owner_info.module
                }
            };
            if item.trait_decl != h.trait_ref.declaration
                || target_info.owner != Some(h.declaration)
                || target_info.receiver != item.receiver
                || !private_visibility
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait implementation item owner/receiver/visibility mismatch",
                ));
            }
            let method_arguments = self.method_arguments(*callable_id)?;
            if self.info(item.declaration)?.own_env.arity() != method_arguments.len()
                || !self.method_environment_matches(
                    &target.schema.env,
                    &h.schema_env,
                    &target_info.own_env,
                )?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait implementation method binder mismatch",
                ));
            }
            let env = self.predicate_env(h.declaration)?;
            let context = ProofContext {
                original: None,
                predicates: &env,
                phase: FactsPhase::Universal,
            };
            let mut expected = if !method_arguments.is_empty() {
                self.alpha_rename_method_schema(
                    item,
                    &h.trait_ref,
                    &method_arguments,
                    &target.schema.env,
                )?
            } else if self.schema_contains_projection(&item.schema)? {
                self.instantiate_owner_item(item, h, context)?
            } else {
                self.instantiate_item(item, &h.trait_ref, context)?.0
            };
            if !method_arguments.is_empty()
                && self.signature_contains_projection(&expected.signature)?
            {
                self.normalize_required_owner_method_signature(
                    *callable_id,
                    &mut expected.signature,
                )?;
            }
            expected.predicates =
                self.normalize_required_owner_predicates(*callable_id, &expected.predicates)?;
            self.validate_signature_shape(&expected.signature, &target.schema.env, None)?;
            self.validate_predicates(&expected.predicates, &target.schema.env, None)?;
            if !self.eq_types(
                &expected.signature.params,
                &target.schema.signature.params,
                0,
            )? || !self.eq_ty(
                &expected.signature.result,
                &target.schema.signature.result,
                0,
            )? || !self.reference_free_implementation_refines(
                &expected.signature,
                &target.schema.signature,
                &target.schema.env,
                None,
            )? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait implementation signature does not match required declaration",
                ));
            }
            if self.schema_contains_projection(&item.schema)? {
                self.validate_projected_carrier_refinement(item, &expected.signature, target)?;
            }
            let owner_membership = Predicate::Implements(h.trait_ref.clone());
            for predicate in target.schema.predicates.iter() {
                // The checked item owner is this exact implementation, so its
                // full application is known inside the implementation body.
                // Another application with the same Self is not that proof.
                let mut allowed = self.eq_predicate(predicate, &owner_membership)?;
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
    fn supers_for(&self, goal: &TraitRef, caller: &GenericEnv) -> Result<Vec<Predicate>> {
        let mut result = self.supertrait_predicates_for(goal, caller)?;
        for associated in self.trait_header(goal.declaration)?.associated_types.iter() {
            for predicate in self
                .associated_bounds_for(*associated, goal, caller)?
                .iter()
            {
                self.append_predicate(&mut result, predicate)?;
            }
        }
        Ok(result)
    }
    fn supertrait_predicates_for(
        &self,
        goal: &TraitRef,
        caller: &GenericEnv,
    ) -> Result<Vec<Predicate>> {
        self.substitute_supertrait_predicates(goal, caller, true)
    }
    fn substitute_supertrait_predicates(
        &self,
        goal: &TraitRef,
        caller: &GenericEnv,
        validate_types: bool,
    ) -> Result<Vec<Predicate>> {
        let header = self.trait_header(goal.declaration)?;
        if goal.args.len().checked_add(1) != Some(header.env.total_arity()) {
            return Err(FactsError::invalid(
                &self.budget,
                "trait argument arity mismatch",
            ));
        }
        let mut declared = Vec::new();
        for requirement in header.supertraits.iter() {
            push(
                &self.budget,
                &mut declared,
                Predicate::Implements(requirement.clone()),
            )?;
        }
        if declared.is_empty() {
            return Ok(declared);
        }
        // Preserve the old zero-argument path and its reference-Self behavior.
        if header.env.total_arity() == 1
            && header.supertraits.iter().all(|requirement| {
                matches!(requirement.self_ty, Ty::Param(p) if p == header.self_param)
                    && requirement.args.is_empty()
            })
        {
            for predicate in &mut declared {
                if let Predicate::Implements(requirement) = predicate {
                    requirement.self_ty = goal.self_ty.clone();
                }
            }
            return Ok(declared);
        }
        let arguments = self.trait_arguments(goal)?;
        retain::<ir::RefContract>(&self.budget, 1)?;
        let schema = GenericSchema {
            env: header.env.clone(),
            signature: Signature {
                ref_contract: Arc::new(ir::RefContract::default()),
                params: Vec::new(),
                result: Ty::Unit,
            },
            predicates: declared.into(),
        };
        let instantiated =
            self.schema
                .instantiate_schema(&schema, &arguments, caller, &self.budget)?;
        if validate_types {
            self.validate_predicates(&instantiated.predicates, caller, None)?;
        } else {
            for predicate in instantiated.predicates.iter() {
                self.validate_predicate_metadata(predicate, caller)?;
            }
        }
        let mut result = Vec::new();
        for predicate in instantiated.predicates.iter() {
            push(&self.budget, &mut result, predicate.clone())?;
        }
        Ok(result)
    }
    fn close_predicates(&self, predicates: &mut Vec<Predicate>, caller: &GenericEnv) -> Result<()> {
        let mut index = 0;
        while index < predicates.len() {
            let Predicate::Implements(goal) = &predicates[index] else {
                index += 1;
                continue;
            };
            let supers = self.supers_for(goal, caller)?;
            for p in supers {
                self.append_predicate(predicates, &p)?;
            }
            index += 1;
        }
        Ok(())
    }
    fn implicit_sized(&self, env: &GenericEnv, predicates: &mut Vec<Predicate>) -> Result<()> {
        for p in env.ordered_params() {
            if (self.info(p.owner.declaration)?.kind == DeclKind::Trait && p.index == 0)
                || env.relaxed_sized(p)
            {
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
            self.close_predicates(&mut predicates, &h.schema_env)?;
            self.validate_declared_equality_domains(&predicates, &h.schema_env)?;
            retain::<PredicateEnv>(&self.budget, 1)?;
            let index = self.environments.len();
            push_indexed_environment(
                &self.budget,
                &mut self.environments,
                &mut self.environment_index,
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
        if let Some(view) = self.closure_result_carrier_contract(subject, facts)? {
            return Ok(view);
        }
        if let SignatureSubject::Call(
            Callee::TraitItem(call)
            | Callee::Virtual(ir::VirtualCall { call })
            | Callee::Untupled(call),
        ) = subject
            && self.schema_contains_projection(&self.item_header(call.item)?.schema)?
        {
            if !self.trait_call_is_closed(call, facts.env())?
                && !matches!(&call.trait_ref.self_ty, Ty::Dyn(_))
            {
                return self.universal_required_carrier_contract(call, facts);
            }
            return self.projected_carrier_contract(call, facts);
        }
        let result_subject = self.equality_result_subject(subject, facts)?;
        let (declaration, arguments, signature) = match subject {
            SignatureSubject::Body if facts.closure.is_some() => {
                let closure = facts.closure.as_ref().expect("guarded closure facts");
                (
                    closure.provider(),
                    self.closure_body_arguments(facts)?
                        .expect("closure arguments"),
                    self.closure_body_signature(facts)?,
                )
            }
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
                FactsSubject::SchemaItem(_) | FactsSubject::AssociatedConstant(_) => {
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
            SignatureSubject::Call(
                Callee::TraitItem(call)
                | Callee::Virtual(ir::VirtualCall { call })
                | Callee::Untupled(call),
            ) => {
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
        let raw = self.normalized_declaration_schema(declaration)?;
        if raw.env.total_arity() != arguments.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "carrier subject argument arity mismatch",
            ));
        }
        let retained = self.declaration_carriers(declaration)?;
        let origin = self
            .origin_index
            .get(&self.budget, declaration)?
            .map(|position| self.origins[position].1.clone())
            .ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "carrier subject has no retained origin schema",
                )
            })?;
        self.transport_carrier_contract(
            declaration,
            arguments,
            signature,
            facts,
            raw,
            retained,
            origin,
            None,
            result_subject,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn transport_carrier_contract(
        &self,
        declaration: DeclId,
        arguments: Arc<[Ty]>,
        signature: Signature,
        facts: &ValidatedFacts,
        raw: GenericSchema,
        retained: &DeclarationCarriers,
        origin: Arc<ir::SignatureOriginSchema>,
        projected: Option<Arc<projected_methods::ProjectedMethodEvidence>>,
        result_subject: parameter_equality::EqualityResultSubject,
    ) -> Result<Arc<CarrierContractView>> {
        let mut view = self.transport_carrier_parts(
            declaration,
            arguments,
            signature,
            facts,
            raw,
            retained,
            origin,
            projected,
            result_subject,
        )?;
        self.seal_equality_results(&mut view, facts)?;
        view.validate_for(facts)?;
        Ok(Arc::new(view))
    }

    /// Private structural replay only. A view cannot be published from this
    /// helper: the publishing wrapper always seals and validates all authority.
    #[allow(clippy::too_many_arguments)]
    fn transport_carrier_parts(
        &self,
        declaration: DeclId,
        arguments: Arc<[Ty]>,
        signature: Signature,
        facts: &ValidatedFacts,
        raw: GenericSchema,
        retained: &DeclarationCarriers,
        origin: Arc<ir::SignatureOriginSchema>,
        projected: Option<Arc<projected_methods::ProjectedMethodEvidence>>,
        result_subject: parameter_equality::EqualityResultSubject,
    ) -> Result<CarrierContractView> {
        if arguments.len() != raw.env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "carrier transport argument arity mismatch",
            ));
        }
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
        let mut families = Vec::new();
        for parameter in raw.env.ordered_params() {
            push(
                &self.budget,
                &mut families,
                ir::CarrierFamily::Parameter(parameter),
            )?;
        }
        for location in retained
            .contract
            .inputs
            .iter()
            .map(|input| &input.location)
            .chain(
                retained
                    .contract
                    .results
                    .iter()
                    .map(|result| &result.location),
            )
            .chain(
                retained
                    .contract
                    .writes
                    .iter()
                    .map(|write| &write.destination.location),
            )
        {
            if let Some(
                family @ (ir::CarrierFamily::Projection(_) | ir::CarrierFamily::Opaque(_)),
            ) = location.channel.family()
            {
                charged(&self.budget, self.budget.charge_work(families.len()))?;
                if !families.contains(&family) {
                    push(&self.budget, &mut families, family)?;
                }
            }
        }
        for family in families {
            let actual = match &family {
                ir::CarrierFamily::Parameter(parameter) => {
                    let index = raw
                        .env
                        .ordered_params()
                        .position(|candidate| candidate == *parameter)
                        .ok_or_else(|| {
                            FactsError::invalid(
                                &self.budget,
                                "carrier parameter disappeared from original environment",
                            )
                        })?;
                    arguments[index].clone()
                }
                ir::CarrierFamily::Opaque(opaque) => {
                    self.realize_opaque_family(opaque, &raw.env, &arguments, facts)?
                }
                ir::CarrierFamily::Projection(projection) => {
                    self.realize_projection_family(projection, &raw.env, &arguments, facts)?
                }
            };
            for relative in
                charged(&self.budget, ir::carrier_locations(&actual, &self.budget))?.iter()
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
                        original_family: family.clone(),
                        original: ir::CarrierLocation {
                            path: relative.path.clone(),
                            channel: match &family {
                                ir::CarrierFamily::Parameter(parameter) => {
                                    ir::CarrierChannel::ParamTail {
                                        parameter: *parameter,
                                        prefix: relative.channel.prefix(),
                                    }
                                }
                                ir::CarrierFamily::Opaque(opaque) => {
                                    ir::CarrierChannel::OpaqueTail {
                                        opaque: opaque.clone(),
                                        prefix: relative.channel.prefix(),
                                    }
                                }
                                ir::CarrierFamily::Projection(projection) => {
                                    ir::CarrierChannel::ProjectionTail {
                                        projection: projection.clone(),
                                        prefix: relative.channel.prefix(),
                                    }
                                }
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
            for expanded in
                self.expand_carrier(&input.location, &raw.env, &arguments, &nominals, facts)?
            {
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
            for output in
                self.expand_carrier(&result.location, &raw.env, &arguments, &nominals, facts)?
            {
                let sources = self.expand_result_sources(
                    result,
                    &output,
                    &retained.contract.inputs,
                    &raw.env,
                    &arguments,
                    &nominals,
                    facts,
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
            for destination in self.expand_carrier(
                &write.destination.location,
                &raw.env,
                &arguments,
                &nominals,
                facts,
            )? {
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
                    &write.destination.location.channel,
                    &write.sources,
                    destination.component.as_ref(),
                    &raw.env,
                    &arguments,
                    &nominals,
                    facts,
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
        let mut carrier_regions = Vec::new();
        for row in retained.source_regions.carriers.iter() {
            let region = match row.region {
                ir::CarrierRegionTerm::Fixed(old) => {
                    ir::CarrierRegionTerm::Fixed(self.transport_fixed_region(
                        old,
                        &raw.signature.ref_contract,
                        &signature.ref_contract,
                    )?)
                }
                ref other => other.clone(),
            };
            for expanded in
                self.expand_carrier(&row.location, &raw.env, &arguments, &nominals, facts)?
            {
                push(
                    &self.budget,
                    &mut carrier_regions,
                    ir::SourceCarrierRegion {
                        parameter: row.parameter,
                        location: expanded.location,
                        region: region.clone(),
                    },
                )?;
            }
        }
        let source_regions = Arc::new(ir::SourceRegionEnv {
            declaration,
            region_count: retained.source_regions.region_count,
            outlives: retained.source_regions.outlives.clone(),
            normalized: normalized.into(),
            carriers: carrier_regions.into(),
        });
        retain::<CarrierContractView>(&self.budget, 1)?;
        let projected_subject = projected.as_ref().map(|proof| proof.goal().clone());
        let fixed = signature.ref_contract.clone();
        let native_heap_invariants =
            self.native_heap_invariants(&raw.signature, &retained.contract.inputs)?;
        let view = CarrierContractView {
            native_heap_invariants,
            nominal: None,
            physical: None,
            closure_result: None,
            result_signature: signature,
            result_subject,
            equality_results: None,
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
            fixed,
            regions: ir::CarrierRegionSchema {
                constraints: constraints.into(),
            },
            source_regions,
            components: components.into(),
            nominals: nominals.into(),
            projected,
            projected_subject,
            equality_required: None,
            budget: self.budget.clone(),
        };
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
    /// Sources of one expanded result carrier. A type-parameter family result
    /// with no tracked source (values of `T` obtained through trait calls such
    /// as `Iterator::next`) may still contain any lifetime the caller's actual
    /// for `T` shares with an input, so its substituted reference carriers
    /// conservatively depend on every substituted input carrier reached
    /// through a type-family position: more loans, never fewer. A declared
    /// fixed reference layer (`&mut self`) carries a lifetime chosen
    /// independently of `T`, so a `T` value cannot borrow from it.
    #[allow(clippy::too_many_arguments)]
    fn expand_result_sources(
        &self,
        result: &ir::CarrierReturn,
        output: &ExpandedCarrier,
        declared_inputs: &[ir::InputCarrier],
        env: &GenericEnv,
        args: &[Ty],
        nominals: &[ir::NominalCarrierMapping],
        facts: &ValidatedFacts,
    ) -> Result<Arc<[ir::InputCarrier]>> {
        if result.sources.is_empty()
            && matches!(
                result.location.channel,
                ir::CarrierChannel::ParamTail { .. }
            )
            && matches!(output.location.channel, ir::CarrierChannel::Fixed(_))
        {
            let mut sources = Vec::new();
            for input in declared_inputs {
                charged(&self.budget, self.budget.charge_work(1))?;
                if matches!(input.location.channel, ir::CarrierChannel::Fixed(_)) {
                    continue;
                }
                for expanded in self.expand_carrier(&input.location, env, args, nominals, facts)? {
                    if sources.len() >= ir::MAX_LOAN_SITES {
                        return Err(FactsError::resource(
                            &self.budget,
                            "carrier input origin limit exceeded",
                        ));
                    }
                    push(
                        &self.budget,
                        &mut sources,
                        ir::InputCarrier {
                            param: input.param,
                            location: expanded.location,
                        },
                    )?;
                }
            }
            return Ok(Arc::from(sources));
        }
        self.expand_carrier_sources(
            &result.location.channel,
            &result.sources,
            output.component.as_ref(),
            env,
            args,
            nominals,
            facts,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn expand_carrier_sources(
        &self,
        destination: &ir::CarrierChannel,
        sources: &[ir::InputCarrier],
        component: Option<&ir::CarrierLocation>,
        env: &GenericEnv,
        args: &[Ty],
        nominals: &[ir::NominalCarrierMapping],
        facts: &ValidatedFacts,
    ) -> Result<Arc<[ir::InputCarrier]>> {
        let mut output = Vec::new();
        for source in sources {
            for expanded in self.expand_carrier(&source.location, env, args, nominals, facts)? {
                let matches = match (component, expanded.component.as_ref()) {
                    (
                        Some(ir::CarrierLocation {
                            path: destination_path,
                            channel: ir::CarrierChannel::OpaqueTail { opaque, prefix: 0 },
                        }),
                        Some(ir::CarrierLocation {
                            path: source_path,
                            channel:
                                ir::CarrierChannel::ParamTail {
                                    parameter,
                                    prefix: 0,
                                },
                        }),
                    ) if destination_path.0.is_empty() && source_path.0.is_empty() => {
                        // A public opaque envelope deliberately changes the
                        // family identity. Its retained source edge is valid
                        // only through exact declaration capture substitution.
                        self.opaque_captures_parameter(opaque, *parameter)?
                    }
                    // A projection's realized carriers are produced by the
                    // selected impl from data of the parameters it mentions;
                    // which concrete component maps where is impl-internal,
                    // so every carrier of such a parameter is an eligible
                    // source (conservative: more loans, never fewer).
                    _ if matches!(
                        (destination, &source.location.channel),
                        (
                            ir::CarrierChannel::ProjectionTail { projection, .. },
                            ir::CarrierChannel::ParamTail { parameter, .. },
                        ) if self.projection_mentions_parameter(projection, *parameter)?
                    ) =>
                    {
                        true
                    }
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
    fn projection_mentions_parameter(
        &self,
        projection: &ir::AssociatedTypeProjection,
        parameter: ParamId,
    ) -> Result<bool> {
        fn mentions(
            ty: &Ty,
            parameter: ParamId,
            budget: &PlanBudget,
            depth: usize,
        ) -> Result<bool> {
            charged(budget, budget.check_depth(depth))?;
            charged(budget, budget.charge_work(1))?;
            let all = |types: &[Ty]| -> Result<bool> {
                for ty in types {
                    if mentions(ty, parameter, budget, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            };
            Ok(match ty {
                Ty::Param(p) => *p == parameter,
                Ty::AssociatedType(projection) => {
                    mentions(&projection.trait_ref.self_ty, parameter, budget, depth + 1)?
                        || all(&projection.trait_ref.args)?
                }
                Ty::Cell(cell) => mentions(cell.argument(), parameter, budget, depth + 1)?,
                Ty::ManuallyDrop(wrapper) => {
                    mentions(wrapper.argument(), parameter, budget, depth + 1)?
                }
                Ty::Tuple(parts) => all(parts)?,
                Ty::Array { element, .. } | Ty::Slice { element } => {
                    mentions(element, parameter, budget, depth + 1)?
                }
                Ty::Ref { referent, .. }
                | Ty::RawPtr {
                    pointee: referent, ..
                } => mentions(referent, parameter, budget, depth + 1)?,
                Ty::Struct(n) => all(&n.def().type_args)?,
                Ty::Enum(n) => all(&n.def().type_args)?,
                // Opaque, closure, object and fn-item payloads: conservative.
                Ty::Opaque(_)
                | Ty::Closure(_)
                | Ty::Dyn(_)
                | Ty::PhantomData(_)
                | Ty::FnPointer(_)
                | Ty::FnItem(_) => true,
                _ => false,
            })
        }
        if mentions(&projection.trait_ref.self_ty, parameter, &self.budget, 0)? {
            return Ok(true);
        }
        for argument in projection.trait_ref.args.iter() {
            if mentions(argument, parameter, &self.budget, 0)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn expand_carrier(
        &self,
        location: &ir::CarrierLocation,
        env: &GenericEnv,
        args: &[Ty],
        nominals: &[ir::NominalCarrierMapping],
        facts: &ValidatedFacts,
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
        match &location.channel {
            ir::CarrierChannel::Fixed(position) => {
                push(
                    &self.budget,
                    &mut output,
                    ExpandedCarrier {
                        location: ir::CarrierLocation {
                            path: ir::CarrierPath(path.into()),
                            channel: ir::CarrierChannel::Fixed(*position),
                        },
                        component: None,
                    },
                )?;
            }
            ir::CarrierChannel::ParamTail { .. }
            | ir::CarrierChannel::ProjectionTail { .. }
            | ir::CarrierChannel::OpaqueTail { .. } => {
                let prefix = location.channel.prefix();
                let actual = match &location.channel {
                    ir::CarrierChannel::OpaqueTail { opaque, .. } => {
                        self.realize_opaque_family(opaque, env, args, facts)?
                    }
                    ir::CarrierChannel::ProjectionTail { projection, .. } => {
                        self.realize_projection_family(projection, env, args, facts)?
                    }
                    ir::CarrierChannel::ParamTail { parameter, .. } => {
                        let mut actual = None;
                        for (index, p) in env.ordered_params().enumerate() {
                            charged(&self.budget, self.budget.charge_work(1))?;
                            if p == *parameter {
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
                        actual.clone()
                    }
                    _ => unreachable!(),
                };
                for relative in
                    charged(&self.budget, ir::carrier_locations(&actual, &self.budget))?.iter()
                {
                    let mut full = Vec::new();
                    for step in &path {
                        push(&self.budget, &mut full, step.clone())?;
                    }
                    let channel = if relative.path.0.is_empty() {
                        match &relative.channel {
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
                                parameter: *parameter,
                                prefix: prefix.checked_add(*p).ok_or_else(|| {
                                    FactsError::resource(
                                        &self.budget,
                                        "carrier symbolic prefix overflow",
                                    )
                                })?,
                            },
                            ir::CarrierChannel::OpaqueTail { opaque, prefix: p } => {
                                ir::CarrierChannel::OpaqueTail {
                                    opaque: opaque.clone(),
                                    prefix: prefix.checked_add(*p).ok_or_else(|| {
                                        FactsError::resource(
                                            &self.budget,
                                            "opaque carrier prefix overflow",
                                        )
                                    })?,
                                }
                            }
                            ir::CarrierChannel::ProjectionTail {
                                projection,
                                prefix: p,
                            } => ir::CarrierChannel::ProjectionTail {
                                projection: projection.clone(),
                                prefix: prefix.checked_add(*p).ok_or_else(|| {
                                    FactsError::resource(
                                        &self.budget,
                                        "projection carrier prefix overflow",
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
            (Ty::Cell(a), Ty::Cell(b)) => {
                if !a.try_eq(b, |a, b| {
                    self.collect_nominal_mappings(a, b, output, depth + 1)?;
                    Ok::<bool, FactsError>(true)
                })? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "Cell declaration changed during transport",
                    ));
                }
            }
            (Ty::ManuallyDrop(a), Ty::ManuallyDrop(b)) => {
                if !a.try_eq(b, |a, b| {
                    self.collect_nominal_mappings(a, b, output, depth + 1)?;
                    Ok::<bool, FactsError>(true)
                })? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "ManuallyDrop declaration changed during transport",
                    ));
                }
            }
            (Ty::PhantomData(a), Ty::PhantomData(b)) => {
                if !a.try_eq(b, |a, b| {
                    self.collect_nominal_mappings(a, b, output, depth + 1)?;
                    Ok::<bool, FactsError>(true)
                })? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "PhantomData declaration changed during transport",
                    ));
                }
            }
            (Ty::FnPointer(a), Ty::FnPointer(b)) => {
                if !a.try_eq(b, |a, b| {
                    self.collect_nominal_mappings(a, b, output, depth + 1)?;
                    Ok::<bool, FactsError>(true)
                })? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "function pointer signature arity changed during transport",
                    ));
                }
            }
            (Ty::Param(_) | Ty::AssociatedType(_), _) => {}
            (Ty::Opaque(original), Ty::Opaque(instantiated)) => {
                // Opaque identities retain their defining owner, return site,
                // bound topology and captured region family. Only their ordered
                // type payloads follow the already authorized substitution.
                if !original.try_eq(instantiated, |before, after| {
                    self.collect_nominal_mappings(before, after, output, depth + 1)?;
                    Ok::<bool, FactsError>(true)
                })? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "opaque carrier transport changed its canonical declaration identity",
                    ));
                }
            }
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
            (
                Ty::RawPtr {
                    kind: a,
                    pointee: a_ty,
                },
                Ty::RawPtr {
                    kind: b,
                    pointee: b_ty,
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
            (Ty::Slice { element: a }, Ty::Slice { element: b }) => {
                self.collect_nominal_mappings(a, b, output, depth + 1)?;
            }
            (Ty::Dyn(a), Ty::Dyn(b))
                if a.principal == b.principal
                    && a.args.len() == b.args.len()
                    && a.associated_types.len() == b.associated_types.len() =>
            {
                for (a, b) in a.args.iter().zip(b.args.iter()) {
                    self.collect_nominal_mappings(a, b, output, depth + 1)?;
                }
                for ((a_item, a), (b_item, b)) in
                    a.associated_types.iter().zip(b.associated_types.iter())
                {
                    if a_item != b_item {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "dynamic carrier changed associated identity",
                        ));
                    }
                    self.collect_nominal_mappings(a, b, output, depth + 1)?;
                }
            }
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
                for (a, b) in nominal_fields(&a.def().fields, &self.budget)?
                    .iter()
                    .zip(nominal_fields(&b.def().fields, &self.budget)?.iter())
                {
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
                    for (a, b) in nominal_fields(&a.fields, &self.budget)?
                        .iter()
                        .zip(nominal_fields(&b.fields, &self.budget)?.iter())
                    {
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
        if let Some(position) = self.carrier_index.get(&self.budget, declaration)?
            && self.carriers[position].declaration == declaration
        {
            return Ok(&self.carriers[position]);
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
        for (i, reachable) in closure.iter().take(count).enumerate() {
            for j in 0..count {
                if reachable & (1u64 << j) == 0 {
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
        let original = self.schema.declaration_schema(declaration)?;
        let carriers = self.checked_source_carriers(&regions, &original.signature, signature)?;
        retain::<ir::SourceRegionEnv>(&self.budget, 1)?;
        Ok(Arc::new(ir::SourceRegionEnv {
            carriers,
            ..regions.as_ref().clone()
        }))
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
        if a.path != b.path {
            return Ok(false);
        }
        match (&a.channel, &b.channel) {
            (
                ir::CarrierChannel::OpaqueTail {
                    opaque: a,
                    prefix: ap,
                },
                ir::CarrierChannel::OpaqueTail {
                    opaque: b,
                    prefix: bp,
                },
            ) => Ok(ap == bp && self.eq_ty(&Ty::Opaque(a.clone()), &Ty::Opaque(b.clone()), 0)?),
            (
                ir::CarrierChannel::ProjectionTail {
                    projection: a,
                    prefix: ap,
                },
                ir::CarrierChannel::ProjectionTail {
                    projection: b,
                    prefix: bp,
                },
            ) => Ok(ap == bp
                && self.eq_ty(
                    &Ty::AssociatedType(a.clone()),
                    &Ty::AssociatedType(b.clone()),
                    0,
                )?),
            _ => Ok(a.channel == b.channel),
        }
    }
    /// A mutable native owner fixes its element type invariantly. Every value
    /// argument using that same original family must remain valid for the
    /// storage lifetime. This is not permission to write a shared summary.
    fn native_heap_invariants(
        &self,
        signature: &Signature,
        inputs: &[ir::InputCarrier],
    ) -> Result<Arc<[NativeHeapInvariant]>> {
        let mut rows = Vec::new();
        for storage in inputs {
            charged(&self.budget, self.budget.charge_work(1))?;
            let Some(Ty::Ref {
                kind: ir::BorrowKind::Mut,
                ..
            }) = signature.params.get(storage.param)
            else {
                continue;
            };
            // `&mut A` makes every type parameter owned by `A` invariant:
            // a same-family input may be stored anywhere in `A`'s owned
            // structure (`Vec<T>` elements, `HashMap<K, V>` field vectors).
            // Only owned paths below the single mutable referent qualify;
            // a further reference step would be covariant storage.
            let ir::CarrierChannel::ParamTail { prefix: 0, .. } = storage.location.channel else {
                continue;
            };
            let path = storage.location.path.0.as_ref();
            if path.first() != Some(&ir::CarrierStep::Referent)
                || path[1..]
                    .iter()
                    .any(|step| matches!(step, ir::CarrierStep::Referent))
            {
                continue;
            }
            let mut sources = Vec::new();
            for input in inputs {
                charged(&self.budget, self.budget.charge_work(1))?;
                if input.param != storage.param
                    && input.location.channel == storage.location.channel
                {
                    push(&self.budget, &mut sources, input.clone())?;
                }
            }
            if !sources.is_empty() {
                // The view visitor does not traverse these proof pointees.
                // Prepay the immutable transfer independently of Vec capacity.
                charged(&self.budget, self.budget.charge_nodes(1))?;
                retain::<ir::InputCarrier>(&self.budget, sources.len())?;
                push(
                    &self.budget,
                    &mut rows,
                    NativeHeapInvariant {
                        storage: storage.clone(),
                        sources: sources.into(),
                    },
                )?;
            }
        }
        charged(&self.budget, self.budget.charge_nodes(1))?;
        retain::<NativeHeapInvariant>(&self.budget, rows.len())?;
        Ok(rows.into())
    }

    #[cfg(test)]
    pub(in crate::compiler) fn carrier_writable_for_tests(
        &self,
        ty: &Ty,
        location: &ir::CarrierLocation,
    ) -> Result<bool> {
        self.carrier_writable(ty, location)
    }

    fn carrier_writable(&self, ty: &Ty, location: &ir::CarrierLocation) -> Result<bool> {
        let mut ty = ty.clone();
        let mut saw_deref = false;
        let mut all_mutable = true;
        for step in location.path.0.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            ty = match (step, &ty) {
                (ir::CarrierStep::Referent, Ty::Ref { kind, referent }) => {
                    saw_deref = true;
                    all_mutable &= *kind == ir::BorrowKind::Mut;
                    referent.as_ref().clone()
                }
                (ir::CarrierStep::ManuallyDropPayload, Ty::ManuallyDrop(wrapper)) => {
                    charged(&self.budget, wrapper.validate(&self.catalog, &self.budget))?;
                    wrapper.argument().clone()
                }
                (ir::CarrierStep::Field(_), Ty::ManuallyDrop(_)) => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "ManuallyDrop carrier effect requires canonical payload step",
                    ));
                }
                (ir::CarrierStep::Field(index), _) => {
                    charged(&self.budget, ty.field_ty(*index, &self.budget))?.ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "carrier effect field is outside original type",
                        )
                    })?
                }
                (ir::CarrierStep::ConstantIndex(index), Ty::Array { element, len })
                    if index < len =>
                {
                    element.as_ref().clone()
                }
                (ir::CarrierStep::SliceElement, _) if ty.owned_sequence_element().is_some() => {
                    // A heap summary, like a slice summary, never grants write
                    // authority to its potentially shared element family.
                    all_mutable = false;
                    ty.owned_sequence_element().unwrap().clone()
                }
                (ir::CarrierStep::SliceElement, Ty::Slice { element }) => {
                    // Slice elements are writable exactly when every enclosing
                    // referent is mutable (`&mut [&'a T]`); shared slices
                    // already cleared authority at their referent step.
                    element.as_ref().clone()
                }
                (ir::CarrierStep::VariantField { variant, field }, Ty::Enum(value))
                    if variant.owner == value.def().id =>
                {
                    let variant = value.def().variants.get(variant.index).ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "carrier variant effect is outside original type",
                        )
                    })?;
                    nominal_fields(&variant.fields, &self.budget)?
                        .get(*field)
                        .ok_or_else(|| {
                            FactsError::invalid(
                                &self.budget,
                                "carrier variant effect is outside original type",
                            )
                        })?
                        .ty
                        .clone()
                }
                _ => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "carrier effect lens is not canonical",
                    ));
                }
            };
        }
        let prefix = location.channel.prefix();
        for _ in 0..prefix {
            charged(&self.budget, self.budget.charge_work(1))?;
            let Ty::Ref { kind, referent } = &ty else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "carrier effect prefix is outside original type",
                ));
            };
            saw_deref = true;
            all_mutable &= *kind == ir::BorrowKind::Mut;
            ty = referent.as_ref().clone();
        }
        // A parameter family can hide mutable reference cells even when its
        // declaration has no visible dereference. Retain that deferred envelope;
        // instantiation expands it and rechecks every concrete destination with
        // this same predicate. A shared prefix still forbids mutation.
        let deferred_parameter = matches!(
            (&location.channel, &ty),
            (ir::CarrierChannel::ParamTail { parameter, .. }, Ty::Param(actual))
                if parameter == actual
        );
        Ok(all_mutable && (saw_deref || deferred_parameter))
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
                let eligible = if let Some(eligible) =
                    self.source_carrier_eligible(source, signature, location, input)?
                {
                    eligible
                } else {
                    self.carrier_source_eligible(
                        signature,
                        location,
                        &signature.result,
                        input,
                        None,
                    )?
                };
                if eligible {
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
                let eligible = if let Some(eligible) =
                    self.source_write_eligible(source, signature, destination, input)?
                {
                    eligible
                } else {
                    self.carrier_source_eligible(
                        signature,
                        &destination.location,
                        &signature.params[destination.param],
                        input,
                        Some(destination.param),
                    )?
                };
                if eligible {
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
        // A reference stored behind a shared slice borrow must outlive that
        // borrow. Keep the independent source regions, recording implication
        // rather than identifying the inner lifetime with the outer one.
        for child in source.carriers.iter().filter(|row| {
            row.parameter.is_some() && row.location.path.0.contains(&ir::CarrierStep::SliceElement)
        }) {
            for parent in source
                .carriers
                .iter()
                .filter(|row| row.parameter == child.parameter)
            {
                charged(
                    &self.budget,
                    self.budget.charge_work(
                        child.location.path.0.len() + parent.location.path.0.len() + 1,
                    ),
                )?;
                if carrier_regions::enclosing(&parent.location, &child.location) {
                    push(
                        &self.budget,
                        &mut constraints,
                        ir::CarrierRegionConstraint {
                            longer: child.region.clone(),
                            shorter: parent.region.clone(),
                        },
                    )?;
                }
            }
        }
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
            if let Some(family) = input.location.channel.family()
                && input.location.path.0.is_empty()
            {
                let prefix = input.location.channel.prefix();
                for row in signature.ref_contract.input_regions.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if row.input.param == input.param && row.input.position.depth < prefix {
                        push(
                            &self.budget,
                            &mut constraints,
                            ir::CarrierRegionConstraint {
                                longer: match &family {
                                    ir::CarrierFamily::Parameter(parameter) => {
                                        ir::CarrierRegionTerm::Family {
                                            parameter: *parameter,
                                        }
                                    }
                                    ir::CarrierFamily::Opaque(opaque) => {
                                        ir::CarrierRegionTerm::OpaqueFamily {
                                            opaque: opaque.clone(),
                                        }
                                    }
                                    ir::CarrierFamily::Projection(projection) => {
                                        ir::CarrierRegionTerm::ProjectionFamily {
                                            projection: projection.clone(),
                                        }
                                    }
                                },
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
                ir::CarrierChannel::OpaqueTail { opaque, prefix: 0 },
                ir::CarrierChannel::ParamTail {
                    parameter,
                    prefix: 0,
                },
            ) if write.is_none() => {
                // The public envelope derives solely from the defining schema.
                // Actual hidden-value transport needs a separate pack witness.
                if !matches!(a, Ty::Opaque(actual) if actual.as_ref() == opaque.as_ref())
                    || b != Ty::Param(*parameter)
                {
                    return Ok(false);
                }
                self.opaque_captures_parameter(opaque, *parameter)
            }
            (
                ir::CarrierChannel::ProjectionTail { projection: a, .. },
                ir::CarrierChannel::ProjectionTail { projection: b, .. },
            ) => self.eq_ty(
                &Ty::AssociatedType(a.clone()),
                &Ty::AssociatedType(b.clone()),
                0,
            ),
            (
                ir::CarrierChannel::ParamTail { parameter: a, .. },
                ir::CarrierChannel::ParamTail { parameter: b, .. },
            ) => Ok(a == b),
            // A projection is determined by its trait arguments, so its value
            // carriers can only originate in data whose type mentions one of
            // those parameters (`self.it: I` returning `I::Item`).
            (
                ir::CarrierChannel::ProjectionTail { projection, .. },
                ir::CarrierChannel::ParamTail { parameter, .. },
            ) if write.is_none() => self.projection_mentions_parameter(projection, *parameter),
            (ir::CarrierChannel::Fixed(position), ir::CarrierChannel::Fixed(source_position)) => {
                if let Some(destination_param) = write {
                    if !self.eq_ty(&a, &b, 0)? {
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
                    // Reference carriers nested in aggregates have no
                    // per-position region rows yet. Conservatively let the
                    // result borrow from every input carrier: the caller then
                    // keeps every input loan alive for the result's lifetime.
                    return Ok(true);
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
        let mut seen_declarations = std::collections::HashSet::new();
        for declaration in self
            .headers
            .items
            .iter()
            .map(|h| h.declaration)
            .chain(self.headers.callables.iter().map(|h| h.declaration))
        {
            charged(&self.budget, self.budget.charge_work(1))?;
            seen_declarations
                .try_reserve(1)
                .map_err(|_| FactsError::resource(&self.budget, "declaration set allocation"))?;
            if seen_declarations.insert(declaration) {
                push(&self.budget, &mut declarations, declaration)?;
            }
        }
        for declaration in declarations {
            let raw = self.normalized_declaration_schema(declaration)?;
            if self.signature_contains_projection(&raw.signature)? {
                for ty in raw
                    .signature
                    .params
                    .iter()
                    .chain(std::iter::once(&raw.signature.result))
                {
                    self.validate_universal_projection_type(ty, &raw.env, 0)?;
                }
                self.validate_source_regions(declaration, &raw.signature)?;
            }
            let schema = charged(
                &self.budget,
                ir::derive_signature_origin_schema(&raw.signature, &raw.env, &self.budget),
            )?;
            let source_regions = self.validate_source_regions(declaration, &raw.signature)?;
            let (contract, regions) = self.derive_carriers(&raw.signature, &source_regions)?;
            self.origin_index
                .note(&self.budget, declaration, self.origins.len())?;
            push(&self.budget, &mut self.origins, (declaration, schema))?;
            self.carrier_index
                .note(&self.budget, declaration, self.carriers.len())?;
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
        goal: &TraitRef,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        if goal.declaration != h.trait_ref.declaration {
            return Err(FactsError::invalid(
                &self.budget,
                "implementation well-formedness changed trait identity",
            ));
        }
        let actual = &goal.self_ty;
        for p in self.supertrait_predicates_for(goal, context.env())? {
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
        // A field that is not Copy is stock's E0204, distinct from the E0277
        // reported for an unproven Clone supertrait above.
        let mut field = |ty: &Ty| -> Result<()> {
            let predicate = Predicate::Implements(TraitRef {
                declaration: self.catalog.id(CatalogItem::Copy),
                self_ty: ty.clone(),
                args: Arc::from([]),
            });
            match self.prove_in(&predicate, context, active)? {
                ProofOutcome::Proved(_) => Ok(()),
                ProofOutcome::NotProven => Err(FactsError::unproved(
                    &self.budget,
                    "E0204: the trait `Copy` cannot be implemented for this type because a field does not implement `Copy`",
                )),
            }
        };
        match actual {
            Ty::Struct(t) => {
                for f in nominal_fields(&t.def().fields, &self.budget)?.iter() {
                    field(&f.ty)?;
                }
            }
            Ty::Enum(t) => {
                for v in t.def().variants.iter() {
                    for f in nominal_fields(&v.fields, &self.budget)?.iter() {
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
                &Predicate::Implements(self.symbolic_trait_ref(item.trait_decl)?),
            )?;
            self.close_predicates(&mut predicates, &item.schema.env)?;
            self.validate_declared_equality_domains(&predicates, &item.schema.env)?;
            retain::<PredicateEnv>(&self.budget, 1)?;
            let env = Arc::new(PredicateEnv {
                world: self.id,
                index: self.environments.len(),
                declaration: Some(item.declaration),
                env: item.schema.env.clone(),
                predicates: predicates.into(),
            });
            push_indexed_environment(
                &self.budget,
                &mut self.environments,
                &mut self.environment_index,
                env,
            )?;
        }
        for callable in self.headers.callables.iter() {
            let mut requirements = Vec::new();
            for p in callable.schema.predicates.iter() {
                self.append_predicate(&mut requirements, p)?;
            }
            self.implicit_sized(&callable.schema.env, &mut requirements)?;
            if let Some(item) = self.source_default_item(callable.declaration)? {
                self.append_predicate(
                    &mut requirements,
                    &Predicate::Implements(self.symbolic_trait_ref(item.trait_decl)?),
                )?;
            }
            let owner = self.info(callable.declaration)?.owner;
            let mut implementation = None;
            if let Some(owner) = owner
                && let Some(position) = self.impl_header_index.get(&self.budget, owner)?
            {
                implementation = Some(&self.headers.impls[position]);
            }
            if let Some(h) = implementation {
                for p in h.predicates.iter() {
                    self.append_predicate(&mut requirements, p)?;
                }
            }
            self.close_predicates(&mut requirements, &callable.schema.env)?;
            retain::<Predicate>(&self.budget, requirements.len())?;
            let declared: Arc<[Predicate]> = requirements.clone().into();
            self.requirement_index.note(
                &self.budget,
                callable.declaration,
                self.requirements.len(),
            )?;
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
            self.close_predicates(&mut requirements, &callable.schema.env)?;
            retain::<PredicateEnv>(&self.budget, 1)?;
            let env = Arc::new(PredicateEnv {
                world: self.id,
                index: self.environments.len(),
                declaration: Some(callable.declaration),
                env: callable.schema.env.clone(),
                predicates: requirements.into(),
            });
            push_indexed_environment(
                &self.budget,
                &mut self.environments,
                &mut self.environment_index,
                env,
            )?;
        }
        Ok(())
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
        if let Ty::Cell(cell) = term.ty
            && self.occurs(
                side,
                parameter,
                Term {
                    side: term.side,
                    ty: cell.argument(),
                },
                bindings,
                depth + 1,
            )?
        {
            return Ok(true);
        }
        if let Ty::ManuallyDrop(wrapper) = term.ty
            && self.occurs(
                side,
                parameter,
                Term {
                    side: term.side,
                    ty: wrapper.argument(),
                },
                bindings,
                depth + 1,
            )?
        {
            return Ok(true);
        }
        if let Ty::PhantomData(marker) = term.ty
            && self.occurs(
                side,
                parameter,
                Term {
                    side: term.side,
                    ty: marker.argument(),
                },
                bindings,
                depth + 1,
            )?
        {
            return Ok(true);
        }
        if let Ty::FnPointer(pointer) = term.ty {
            for child in pointer.types() {
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
        | Ty::Slice { element }
        | Ty::RawPtr {
            pointee: element, ..
        }
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
            (Ty::Cell(a), Ty::Cell(b)) => {
                return a.try_eq(b, |a, b| {
                    self.unify(
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
                    )
                });
            }
            (Ty::ManuallyDrop(a), Ty::ManuallyDrop(b)) => {
                return a.try_eq(b, |a, b| {
                    self.unify(
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
                    )
                });
            }
            (Ty::PhantomData(a), Ty::PhantomData(b)) => {
                return a.try_eq(b, |a, b| {
                    self.unify(
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
                    )
                });
            }
            (Ty::FnPointer(a), Ty::FnPointer(b)) => {
                return a.try_eq(b, |a, b| {
                    self.unify(
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
                    )
                });
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
            (
                Ty::RawPtr {
                    kind: a,
                    pointee: at,
                },
                Ty::RawPtr {
                    kind: b,
                    pointee: bt,
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
            (Ty::Slice { element: at }, Ty::Slice { element: bt }) => {
                return self.unify(
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
                );
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
    fn check_original_context(&self, facts: &ValidatedFacts) -> Result<()> {
        self.check_facts(facts, true)?;
        self.check_context(ProofContext::from_facts(facts))?;
        self.info(facts.declaration())?;
        self.callable_header(facts.declaration())?;
        match &facts.subject {
            FactsSubject::UniversalCallable(declaration) => {
                let expected = self.predicate_env(*declaration)?;
                if !Arc::ptr_eq(&expected, &facts.predicates) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "original header read changed its universal caller environment",
                    ));
                }
            }
            FactsSubject::ClosedInstance(target) => {
                if !Arc::ptr_eq(&self.empty, &facts.predicates)
                    || target.type_args.len()
                        != self
                            .callable_header(target.declaration)?
                            .schema
                            .env
                            .total_arity()
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "original header read changed its closed caller subject or empty context",
                    ));
                }
                for ty in target.type_args.iter() {
                    self.validate_ty(ty, self.empty.env(), None, true, 0)?;
                }
            }
            FactsSubject::SchemaItem(_) | FactsSubject::AssociatedConstant(_) => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "schema-only facts cannot read executable original call headers",
                ));
            }
        }
        Ok(())
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
            closure: None,
            closure_mode: None,
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
            closure: None,
            closure_mode: None,
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
            closure: None,
            closure_mode: None,
        };
        let proof = self.discharge_direct(target, &facts)?;
        facts.requirements = proof.evidence;
        retain::<ValidatedFacts>(&self.budget, 1)?;
        Ok(Arc::new(facts))
    }
    fn callable_schema(&self, id: DeclId) -> Result<GenericSchema> {
        let raw = &self.callable_header(id)?.schema;
        let predicates = self
            .requirement_index
            .get(&self.budget, id)?
            .map(|position| self.requirements[position].1.clone());
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
        self.instantiate_scratch_with_facts(
            schema,
            args,
            facts,
            declaration,
            &mut ProofScratch::default(),
            facts.original,
        )
    }
    fn instantiate_scratch(
        &self,
        schema: &GenericSchema,
        args: &[Ty],
        facts: ProofContext<'_>,
        declaration: Option<DeclId>,
        active: &mut ProofScratch,
    ) -> Result<InstantiatedSchema> {
        // This entry is explicitly raw. Genuine callers use the checked entry.
        let facts = ProofContext {
            original: None,
            ..facts
        };
        self.instantiate_scratch_with_facts(schema, args, facts, declaration, active, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn instantiate_scratch_with_facts(
        &self,
        schema: &GenericSchema,
        args: &[Ty],
        facts: ProofContext<'_>,
        declaration: Option<DeclId>,
        active: &mut ProofScratch,
        caller: Option<&ValidatedFacts>,
    ) -> Result<InstantiatedSchema> {
        self.check_proof_context_caller(facts)?;
        if let Some(caller) = caller {
            self.check_facts(caller, true)?;
            if !std::ptr::eq(facts.predicates, caller.predicates.as_ref())
                || facts.phase != caller.phase()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "checked signature caller context changed",
                ));
            }
        }
        active.with_caller_arena(self.id, caller, |arena| {
            self.instantiate_scratch_in_caller_arena(
                schema,
                args,
                facts,
                declaration,
                arena,
                caller,
            )
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn instantiate_scratch_in_caller_arena(
        &self,
        schema: &GenericSchema,
        args: &[Ty],
        facts: ProofContext<'_>,
        declaration: Option<DeclId>,
        active: &mut ProofScratch,
        caller: Option<&ValidatedFacts>,
    ) -> Result<InstantiatedSchema> {
        for argument in args {
            self.validate_ty_metadata_with_facts(
                argument,
                facts.env(),
                0,
                &mut active.replays,
                caller,
            )?;
        }
        if args.len() != schema.env.total_arity() {
            return Err(FactsError::invalid(
                &self.budget,
                "generic argument arity mismatch",
            ));
        }
        for arg in args {
            self.validate_executable_actual_with_facts(arg, facts.env(), caller)?;
        }
        let mut result = self.instantiate_raw_cached(
            schema,
            args,
            facts,
            declaration,
            &mut active.instantiations,
        )?;
        if !self.eq_contract(
            &schema.signature.ref_contract,
            &result.signature.ref_contract,
        )? {
            return Err(FactsError::invalid(
                &self.budget,
                "instantiation changed declaration-derived return authority",
            ));
        }
        self.normalize_instantiated_projection_signature(&mut result.signature, facts, active)?;
        result.signature.ref_contract =
            self.realize_ref_contract(schema, args, &result.signature, declaration)?;
        if result.predicates.len() != schema.predicates.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "schema substitution changed predicate row count",
            ));
        }
        for (raw, actual) in schema.predicates.iter().zip(result.predicates.iter()) {
            self.validate_predicate_substitution(raw, actual, &schema.env, args)?;
        }
        result.predicates =
            self.normalize_instantiated_projection_predicates(&result.predicates, facts, active)?;
        self.validate_signature_shape_with_facts(&result.signature, facts.env(), None, caller)?;
        self.validate_predicates_with_active(
            &result.predicates,
            facts.env(),
            None,
            active,
            caller,
        )?;
        // The complete predicate batch, including ordinary membership rows,
        // already passed metadata validation above. Only equality discharge is
        // new work here; replaying metadata repeats deep actual-type walks.
        self.replay_validated_equality_rows(&result.predicates, facts, active)?;
        Ok(result)
    }
    fn validate_executable_actual(&self, actual: &Ty, env: &GenericEnv) -> Result<()> {
        self.validate_executable_actual_with_facts(actual, env, None)
    }
    fn validate_executable_actual_with_facts(
        &self,
        actual: &Ty,
        env: &GenericEnv,
        caller: Option<&ValidatedFacts>,
    ) -> Result<()> {
        charged(
            &self.budget,
            ir::validate_root_reference_actual(actual, &self.budget),
        )?;
        if matches!(actual, Ty::Str) {
            charged(&self.budget, self.budget.charge_work(1))?;
            return Ok(());
        }
        let usage = if matches!(actual, Ty::Ref { .. }) {
            TypeUse::Root
        } else {
            TypeUse::Actual
        };
        self.validate_ty_inner_with_facts(actual, env, 0, usage, &mut TypeWalk::default(), caller)
            .map(|_| ())
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
            static_regions: 0,
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
    fn check_proof_context_caller(&self, context: ProofContext<'_>) -> Result<()> {
        if let Some(caller) = context.original {
            self.check_facts(caller, true)?;
            if !std::ptr::eq(context.predicates, caller.predicates.as_ref())
                || !Arc::ptr_eq(context.env(), caller.env())
                || context.phase != caller.phase()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait proof original caller context changed",
                ));
            }
        }
        Ok(())
    }
    fn check_context(&self, context: ProofContext<'_>) -> Result<()> {
        self.check_proof_context_caller(context)?;
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
        goal: &TraitRef,
        context: ProofContext<'_>,
    ) -> Result<(InstantiatedSchema, Option<Arc<SignatureTopologyProof>>)> {
        self.instantiate_item_with_scratch(item, goal, context, &mut ProofScratch::default())
    }
    fn instantiate_item_with_scratch(
        &self,
        item: &TraitItemHeader,
        goal: &TraitRef,
        context: ProofContext<'_>,
        scratch: &mut ProofScratch,
    ) -> Result<(InstantiatedSchema, Option<Arc<SignatureTopologyProof>>)> {
        self.check_context(context)?;
        if goal.declaration != item.trait_decl {
            return Err(FactsError::invalid(
                &self.budget,
                "trait item application owner mismatch",
            ));
        }
        self.validate_ref_with(goal, context.env(), None, &mut scratch.replays)?;
        let actual = &goal.self_ty;
        let arguments = self.trait_arguments(goal)?;
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
        self.validate_ty_inner_with_facts(
            actual,
            context.env(),
            0,
            TypeUse::Root,
            &mut TypeWalk::default(),
            context.original,
        )?;
        // Non-Self arguments can also carry references (for example
        // PartialEq<&str>). Derive their positions from every original
        // parameter tail, just as for generic method instantiation.
        let mut argument_has_references = false;
        for argument in goal.args.iter() {
            if charged(&self.budget, ir::root_reference_layers(argument))? != 0 {
                argument_has_references = true;
            }
        }
        if argument_has_references && !self.schema_contains_projection(&item.schema)? {
            let instantiated = self.instantiate_scratch_with_facts(
                &item.schema,
                &arguments,
                context,
                Some(item.declaration),
                scratch,
                context.original,
            )?;
            return Ok((instantiated, None));
        }
        if self.schema_contains_projection(&item.schema)? {
            let application =
                self.instantiate_projected_item_with_scratch(item, goal, &[], context, scratch)?;
            return Ok((application.instantiated, None));
        }
        let origin = self
            .origin_index
            .get(&self.budget, item.declaration)?
            .map(|position| self.origins[position].1.clone())
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
                if !owner.env.contains(parameter) {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "latent carrier has foreign trait parameter",
                    ));
                }
                // Non-Self trait actuals are reference-free, so their latent
                // parameter tails introduce no runtime reference positions.
                continue;
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
            &arguments,
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
            self.validate_signature_shape_with_facts(
                &instantiated.signature,
                context.env(),
                None,
                context.original,
            )?;
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
            static_regions: 0,
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
        self.validate_signature_shape_with_facts(
            &instantiated.signature,
            context.env(),
            None,
            context.original,
        )?;
        self.validate_position_coverage(&instantiated.signature)?;
        retain::<Ty>(&self.budget, arguments.len())?;
        retain::<SignatureTopologyProof>(&self.budget, 1)?;
        let proof = Arc::new(SignatureTopologyProof {
            world: self.id,
            registry: self.registry,
            environment: context.predicates.index,
            phase: context.phase,
            declaration: item.declaration,
            arguments: arguments.into(),
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
        let proof_origin = self
            .origin_index
            .get(&self.budget, proof.declaration)?
            .map(|position| &self.origins[position].1);
        if proof.world != self.id
            || proof.registry != self.registry
            || proof.environment != context.predicates.index
            || proof.phase != context.phase
            || !proof.budget.same_authority(&self.budget)
            || proof.arguments.len()
                != self
                    .trait_header(self.item_header(proof.declaration)?.trait_decl)?
                    .env
                    .total_arity()
            || proof.fixed_regions.len() != usize::from(proof.schema.fixed.region_count)
            || !proof_origin.is_some_and(|origin| Arc::ptr_eq(origin, &proof.schema))
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
        active: &mut ProofScratch,
    ) -> Result<ProofEvidence> {
        match self.prove_in(predicate, facts, active)? {
            ProofOutcome::Proved(p) => Ok(p),
            ProofOutcome::NotProven if matches!(predicate, Predicate::ProjectionEq { .. }) => {
                Err(FactsError::invalid(
                    &self.budget,
                    "associated equality obligation does not match the caller or selected implementation",
                ))
            }
            ProofOutcome::NotProven => Err(FactsError::unproved(
                &self.budget,
                "E0277: trait obligation is not proven by caller bounds or a conditional implementation",
            )),
        }
    }
    fn same_key(&self, a: &GoalKey, b: &GoalKey) -> Result<bool> {
        Ok(a.environment == b.environment
            && a.phase == b.phase
            && a.finalized == b.finalized
            && self.eq_predicate(&a.goal, &b.goal)?)
    }
    fn prove(
        &self,
        predicate: &Predicate,
        facts: &ValidatedFacts,
        active: &mut ProofScratch,
    ) -> Result<ProofOutcome> {
        self.prove_in(predicate, ProofContext::from_facts(facts), active)
    }
    fn prove_in(
        &self,
        predicate: &Predicate,
        facts: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<ProofOutcome> {
        self.check_proof_context_caller(facts)?;
        active.with_caller_arena(self.id, facts.original, |arena| {
            self.prove_in_caller_arena(predicate, facts, arena)
        })
    }
    fn prove_in_caller_arena(
        &self,
        predicate: &Predicate,
        facts: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<ProofOutcome> {
        charged(&self.budget, self.budget.check_depth(active.len()))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        self.check_normalization_budget()?;
        match predicate {
            Predicate::Implements(goal) => self.validate_predicate_ref_with_facts(
                goal,
                facts.env(),
                None,
                &mut active.replays,
                facts.original,
            )?,
            Predicate::ProjectionEq { projection, ty } => self
                .validate_equality_domain_with_active(
                    projection,
                    ty,
                    facts.env(),
                    active,
                    facts.original,
                )?,
        }
        let key = GoalKey {
            environment: facts.predicates.index,
            phase: facts.phase,
            finalized: self.owner_normalization_finalized,
            goal: predicate.clone(),
        };
        for old in active.active.iter() {
            if self.same_key(old, &key)? {
                active.cycles += 1;
                if matches!(predicate, Predicate::Implements(goal) if goal.declaration == self.catalog.id(CatalogItem::Sync))
                {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: recursive Sync proof requires a coinductive protocol",
                    ));
                }
                return Err(FactsError::cycle(
                    &self.budget,
                    "ordinary trait obligation cycle is not self-justifying evidence",
                ));
            }
        }
        let bucket = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            key.environment.hash(&mut hasher);
            key.phase.hash(&mut hasher);
            key.finalized.hash(&mut hasher);
            key.goal.hash(&mut hasher);
            hasher.finish()
        };
        if let Some(candidates) = active.memo_index.get(&bucket) {
            for &index in candidates {
                let entry = &active.memo[index];
                if self.same_key(&entry.key, &key)? {
                    return entry.result.clone();
                }
            }
        }
        active.charge_obligation(&self.budget)?;
        // Transient DFS ancestry, popped below and bounded by check_depth.
        active.active.push(key.clone());
        let cycles_before = active.cycles;
        let result = self.prove_uncached(predicate, facts, active);
        active.active.pop();
        if active.cycles != cycles_before {
            return result;
        }
        // Cycle and resource outcomes are provisional: they depend on the
        // ancestry or remaining budget at this point, so never reuse them.
        if let Err(error) = &result
            && matches!(
                error.kind(),
                FactsErrorKind::Cycle | FactsErrorKind::Resource
            )
        {
            return result;
        }
        // Transient node-local scratch, bounded by MAX_SCRATCH_OBLIGATIONS
        // and dropped with the computation, so it retains no ledger bytes.
        active
            .memo_index
            .entry(bucket)
            .or_default()
            .push(active.memo.len());
        active.memo.push(MemoEntry {
            key,
            result: result.clone(),
        });
        result
    }
    fn evidence(
        &self,
        predicate: &Predicate,
        facts: ProofContext<'_>,
        origin: ProofOrigin,
    ) -> Result<ProofOutcome> {
        self.validate_projected_subject_evidence(predicate, facts, &origin)?;
        if let ProofOrigin::ReflexiveEquality { membership } = &origin {
            let Predicate::ProjectionEq { projection, ty } = predicate else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "reflexive equality evidence changed predicate kind",
                ));
            };
            if membership.0.world != self.id
                || membership.0.environment != facts.predicates.index
                || membership.0.phase != facts.phase
                || !self.eq_predicate(
                    &membership.0.goal,
                    &Predicate::Implements(projection.trait_ref.clone()),
                )?
                || !matches!(ty, Ty::AssociatedType(rhs) if self.same_projection(projection, rhs)?)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "reflexive equality lost exact membership or projection identity",
                ));
            }
        }
        if let ProofOrigin::ImplementationEquality { membership } = &origin {
            let Predicate::ProjectionEq { projection, ty } = predicate else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "implementation equality evidence cannot prove membership",
                ));
            };
            let normal = self.implementation_equality_rhs(projection, membership, facts)?;
            if !self.eq_ty(&normal, ty, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "implementation equality evidence changed RHS",
                ));
            }
        }
        if let ProofOrigin::ClosedEquality { normalization } = &origin {
            self.validate_projected_dependency(normalization, facts)?;
            let Predicate::ProjectionEq { ty, .. } = predicate else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closed equality evidence cannot prove membership",
                ));
            };
            if !self.eq_ty(normalization.ty(), ty, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closed equality evidence changed RHS",
                ));
            }
        }
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
        active: &mut ProofScratch,
    ) -> Result<ProofOutcome> {
        if let Some(outcome) = self.prove_opaque(predicate, facts)? {
            return Ok(outcome);
        }
        if let Predicate::ProjectionEq { projection, ty } = predicate {
            return self.prove_projection_equality(projection, ty, facts, active);
        }
        for assumption in facts.predicates.predicates.iter() {
            if self.eq_predicate(assumption, predicate)? {
                return self.evidence(predicate, facts, ProofOrigin::Assumption);
            }
        }
        if let Predicate::Implements(goal) = predicate
            && goal.declaration == self.catalog.id(CatalogItem::Sync)
            && let Ty::AssociatedType(projection) = &goal.self_ty
            && !self.goal_is_closed(&projection.trait_ref, facts.env())?
            && self
                .opaque_associated_binding(projection, facts.env())?
                .is_none()
        {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: open associated Sync proof requires an authentic exported bound",
            ));
        }
        if let Some(outcome) = self.prove_projected_subject(predicate, facts, active)? {
            return Ok(outcome);
        }
        if let Predicate::Implements(goal) = predicate
            && goal.declaration == self.catalog.id(CatalogItem::Sync)
        {
            self.catalog.trait_requires_unsafe_impl(goal.declaration)?;
            if let Ty::Dyn(object) = &goal.self_ty {
                for application in self.object_applications(object, &goal.self_ty, facts.env())? {
                    if self.eq_predicate(&Predicate::Implements(application), predicate)? {
                        return self.evidence(predicate, facts, ProofOrigin::TraitObject);
                    }
                }
            }
            // Explicit unsafe implementations carry exactly their declared
            // predicates. In particular their fields need not themselves Sync.
            let explicit = self.prove_implementation_in(predicate, facts, active)?;
            if matches!(explicit, ProofOutcome::Proved(_)) {
                return Ok(explicit);
            }
            if self.sync_has_explicit_head(goal)? {
                return Ok(ProofOutcome::NotProven);
            }
        }
        if let Some(outcome) = self.prove_builtin(predicate, facts, active)? {
            return Ok(outcome);
        }
        if let Predicate::Implements(goal) = predicate
            && let Ty::Dyn(object) = &goal.self_ty
        {
            // Trait objects implement exactly their principal and its
            // supertraits (never through a user impl: E0371 overlap aside).
            for application in self.object_applications(object, &goal.self_ty, facts.env())? {
                if self.eq_predicate(&Predicate::Implements(application), predicate)? {
                    return self.evidence(predicate, facts, ProofOrigin::TraitObject);
                }
            }
        }
        self.prove_implementation_in(predicate, facts, active)
    }
    /// Whether `principal` is `goal` or transitively has it as a supertrait.
    fn trait_object_implements(
        &self,
        principal: DeclId,
        goal: DeclId,
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        if principal == goal {
            return Ok(true);
        }
        let header = self.trait_header(principal)?;
        for parent in header.supertraits.iter() {
            if self.trait_object_implements(parent.declaration, goal, depth + 1)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn prove_implementation_in(
        &self,
        predicate: &Predicate,
        facts: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<ProofOutcome> {
        let Predicate::Implements(goal) = predicate else {
            return Err(FactsError::invalid(
                &self.budget,
                "equality is not implementation membership",
            ));
        };
        let mut selected = None;
        let candidates = self
            .trait_impl_index
            .get(&goal.declaration)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for implementation in candidates.iter().map(|&p| &self.headers.impls[p]) {
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
            if implementation.trait_ref.args.len() != goal.args.len() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "trait implementation application arity mismatch",
                ));
            }
            let mut matches = true;
            for (head, actual) in implementation.trait_ref.args.iter().zip(goal.args.iter()) {
                if !self.match_head(head, actual, &implementation.schema_env, &mut slots, 0)? {
                    matches = false;
                    break;
                }
            }
            if !matches {
                continue;
            }
            if slots.iter().any(|slot| slot.is_none())
                && !self.infer_impl_equality_arguments(implementation, &mut slots, facts, active)?
            {
                continue;
            }
            // Generic impl parameters bind unsized actuals only when declared
            // `?Sized`, and only `str` among unsized actuals so far. An
            // implicitly-Sized parameter cannot bind `str`/`[T]`/`dyn`, so
            // such a candidate does not apply rather than erroring.
            if slots
                .iter()
                .zip(implementation.schema_env.ordered_params())
                .any(|(actual, parameter)| match actual {
                    Some(Ty::Str | Ty::Dyn(_)) => {
                        !implementation.schema_env.relaxed_sized(parameter)
                    }
                    Some(Ty::Slice { .. }) => true,
                    _ => false,
                })
            {
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
            let mut head_arguments = Vec::new();
            for argument in implementation.trait_ref.args.iter() {
                // Only an original root reference is a false callable input.
                // Preserve every other head argument, including bare str and
                // parameters that may specialize to str, in its original use.
                let argument = if matches!(argument, Ty::Ref { .. }) {
                    retain::<Ty>(&self.budget, 1)?;
                    Ty::Tuple(Arc::from([argument.clone()]))
                } else {
                    argument.clone()
                };
                push(&self.budget, &mut head_arguments, argument)?;
            }
            let schema = GenericSchema {
                env: implementation.schema_env.clone(),
                // Self is carried as a one-tuple so a reference Self type
                // (`impl Tr for &T`) is not a returned reference.
                signature: Signature {
                    ref_contract: Arc::new(ir::RefContract::default()),
                    params: head_arguments,
                    result: match &implementation.trait_ref.self_ty {
                        reference @ Ty::Ref { .. } => Ty::Tuple(Arc::from([reference.clone()])),
                        other => other.clone(),
                    },
                },
                predicates: implementation.predicates.clone(),
            };
            let instantiated = self.instantiate_scratch_with_facts(
                &schema,
                &args,
                facts,
                None,
                active,
                facts.original,
            )?;
            let head_self = match (
                &implementation.trait_ref.self_ty,
                &instantiated.signature.result,
            ) {
                (Ty::Ref { .. }, Ty::Tuple(wrapped)) => &wrapped[0],
                (Ty::Ref { .. }, _) => {
                    return Err(FactsError::invalid(&self.budget, "impl head Self lost"));
                }
                (_, result) => result,
            };
            let mut arguments_match = instantiated.signature.params.len() == goal.args.len()
                && instantiated.signature.params.len() == implementation.trait_ref.args.len();
            for ((original, instantiated), expected) in implementation
                .trait_ref
                .args
                .iter()
                .zip(instantiated.signature.params.iter())
                .zip(goal.args.iter())
            {
                // The original head position, not the specialized type, owns
                // the wrapper policy. A genuine tuple stays a genuine tuple.
                let actual = if matches!(original, Ty::Ref { .. }) {
                    let Ty::Tuple(fields) = instantiated else {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "impl reference head argument wrapper lost",
                        ));
                    };
                    if fields.len() != 1 {
                        return Err(FactsError::invalid(
                            &self.budget,
                            "impl reference head argument wrapper arity changed",
                        ));
                    }
                    &fields[0]
                } else {
                    instantiated
                };
                arguments_match &= self.eq_ty(actual, expected, 0)?;
            }
            if !self.eq_ty(head_self, &goal.self_ty, 0)? || !arguments_match {
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
            self.require_impl_wf(implementation, goal, facts, active)?;
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
    /// A conditional explicit impl suppresses automatic derivation even when
    /// its predicates are not proved. Use the same canonical head unifier as
    /// selection, never a display name or an all-program scan.
    fn sync_has_explicit_head(&self, goal: &TraitRef) -> Result<bool> {
        let candidates = self
            .trait_impl_index
            .get(&goal.declaration)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for &index in candidates {
            charged(&self.budget, self.budget.charge_work(1))?;
            let implementation = &self.headers.impls[index];
            let mut slots = Vec::new();
            for _ in implementation.schema_env.ordered_params() {
                push(&self.budget, &mut slots, None)?;
            }
            if self.match_head(
                &implementation.trait_ref.self_ty,
                &goal.self_ty,
                &implementation.schema_env,
                &mut slots,
                0,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn prove_builtin(
        &self,
        predicate: &Predicate,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<ProofOutcome>> {
        let Predicate::Implements(goal) = predicate else {
            return Ok(None);
        };
        if matches!(goal.self_ty, Ty::FnPointer(_))
            && self.closure_trait_kind(goal.declaration)?.is_some()
        {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: function pointer callable traits require a native indirect Rust ABI implementation",
            ));
        }
        if let Some(proof) = self.prove_closure(predicate, context, active)? {
            return Ok(Some(proof));
        }
        if let Some(proof) = self.prove_fn_item(predicate, context)? {
            return Ok(Some(proof));
        }
        let item = if goal.declaration == self.catalog.id(CatalogItem::Sized) {
            CatalogItem::Sized
        } else if goal.declaration == self.catalog.id(CatalogItem::Clone) {
            CatalogItem::Clone
        } else if goal.declaration == self.catalog.id(CatalogItem::Sync) {
            CatalogItem::Sync
        } else if goal.declaration == self.catalog.id(CatalogItem::Copy) {
            CatalogItem::Copy
        } else {
            return Ok(None);
        };
        let mut children: Vec<Ty> = Vec::new();
        let known = match &goal.self_ty {
            Ty::Opaque(_) if item == CatalogItem::Sync => {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: opaque Sync proof requires an authentic exported bound",
                ));
            }
            Ty::Opaque(_) => return Ok(Some(ProofOutcome::NotProven)),
            Ty::Closure(closure) => {
                for ty in closure.captures() {
                    push(&self.budget, &mut children, ty.clone())?;
                }
                true
            }
            Ty::Cell(_) => {
                if item == CatalogItem::Sync {
                    return Ok(Some(ProofOutcome::NotProven));
                }
                if item == CatalogItem::Copy {
                    return Ok(Some(ProofOutcome::NotProven));
                }
                if item == CatalogItem::Clone {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: Cell Clone contract is not implemented",
                    ));
                }
                true
            }
            Ty::ManuallyDrop(wrapper) => {
                push(&self.budget, &mut children, wrapper.argument().clone())?;
                true
            }
            Ty::PhantomData(marker) if item == CatalogItem::Sync => {
                push(&self.budget, &mut children, marker.argument().clone())?;
                true
            }
            Ty::PhantomData(_) | Ty::FnPointer(_) | Ty::FnItem(_) => true,
            Ty::Param(_) => return Ok(Some(ProofOutcome::NotProven)),
            Ty::Struct(t) if matches!(item, CatalogItem::Sized | CatalogItem::Sync) => {
                for f in nominal_fields(&t.def().fields, &self.budget)?.iter() {
                    push(&self.budget, &mut children, f.ty.clone())?;
                }
                true
            }
            Ty::Enum(t) if matches!(item, CatalogItem::Sized | CatalogItem::Sync) => {
                for v in t.def().variants.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    for f in nominal_fields(&v.fields, &self.budget)?.iter() {
                        push(&self.budget, &mut children, f.ty.clone())?;
                    }
                }
                true
            }
            Ty::Struct(_) | Ty::Enum(_) => return Ok(None),
            Ty::Tuple(parts) => {
                for t in parts.iter() {
                    push(&self.budget, &mut children, t.clone())?;
                }
                true
            }
            Ty::Array { element, .. } => {
                push(&self.budget, &mut children, element.as_ref().clone())?;
                true
            }
            Ty::Ref { referent, .. } if item == CatalogItem::Sync => {
                push(&self.budget, &mut children, referent.as_ref().clone())?;
                true
            }
            Ty::Ref { kind, .. } => item == CatalogItem::Sized || *kind == ir::BorrowKind::Shared,
            // `[T]`, `str` and `dyn Trait` are neither Sized nor Copy/Clone.
            Ty::Dyn(_) | Ty::AssociatedType(_) if item == CatalogItem::Sync => {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: Sync for dynamic or associated types requires an authentic exported bound",
                ));
            }
            Ty::Str if item == CatalogItem::Sync => true,
            Ty::Slice { element } if item == CatalogItem::Sync => {
                push(&self.budget, &mut children, element.as_ref().clone())?;
                true
            }
            Ty::Slice { .. } | Ty::Str | Ty::Dyn(_) => false,
            Ty::RawPtr { .. } => item != CatalogItem::Sync,
            Ty::Int(_) | Ty::Float(_) | Ty::Char | Ty::Bool | Ty::Unit | Ty::Never => true,
            Ty::AssociatedType(projection) => {
                self.validate_universal_projection(projection, context.env())?;
                item == CatalogItem::Sized
                    && !self.associated_header(projection.item)?.relaxed_sized
            }
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
        for supertrait in self.supers_for(goal, context.env())? {
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
    fn instantiate_trait_signature_uncached(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Signature> {
        if facts.phase() == FactsPhase::Universal
            && self.schema_contains_projection(&self.item_header(call.item)?.schema)?
            && !self.trait_call_is_closed(call, facts.env())?
            && !matches!(&call.trait_ref.self_ty, Ty::Dyn(_))
            && !matches!(&call.trait_ref.self_ty, Ty::Ref { referent, .. } if matches!(referent.as_ref(), Ty::Struct(_) | Ty::Enum(_)))
        {
            return Ok(self
                .instantiate_universal_required_item(call, facts)?
                .0
                .signature);
        }
        self.trait_signature_scratch(
            call,
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
        )
        .map(|(signature, _)| signature)
    }

    fn trait_signature_with_topology(
        &self,
        call: &TraitCall,
        context: ProofContext<'_>,
    ) -> Result<(Signature, Option<Arc<SignatureTopologyProof>>)> {
        self.trait_signature_scratch(call, context, &mut ProofScratch::default())
    }
    fn trait_signature_scratch(
        &self,
        call: &TraitCall,
        context: ProofContext<'_>,
        scratch: &mut ProofScratch,
    ) -> Result<(Signature, Option<Arc<SignatureTopologyProof>>)> {
        self.validate_ty_metadata_with(
            &call.trait_ref.self_ty,
            context.env(),
            0,
            &mut scratch.replays,
        )?;
        for argument in call.trait_ref.args.iter().chain(call.method_args.iter()) {
            self.validate_ty_metadata_with(argument, context.env(), 0, &mut scratch.replays)?;
        }
        self.validate_ref_with(&call.trait_ref, context.env(), None, &mut scratch.replays)?;
        let item = self.item_header(call.item)?;
        if item.trait_decl != call.trait_ref.declaration {
            return Err(FactsError::invalid(
                &self.budget,
                "trait call item belongs to another trait",
            ));
        }
        let method_arity = self.info(item.declaration)?.own_env.arity();
        if call.method_args.len() != method_arity {
            return Err(FactsError::invalid(
                &self.budget,
                "trait method argument arity mismatch",
            ));
        }
        self.require_in(
            &Predicate::Implements(call.trait_ref.clone()),
            context,
            scratch,
        )?;
        let (instantiated, topology) = if method_arity == 0 {
            self.instantiate_item_with_scratch(item, &call.trait_ref, context, scratch)?
        } else {
            (
                self.instantiate_generic_method_with_scratch(
                    item,
                    &call.trait_ref,
                    &call.method_args,
                    context,
                    scratch,
                )?,
                None,
            )
        };
        for predicate in instantiated.predicates.iter() {
            self.require_in(predicate, context, scratch)?;
        }
        Ok((instantiated.signature, topology))
    }
    fn discharge_in(
        &self,
        target: &CallTarget,
        context: ProofContext<'_>,
        scratch: &mut ProofScratch,
    ) -> Result<DirectCallProof> {
        self.discharge_in_with_facts(target, context, scratch, None)
    }
    fn discharge_in_with_facts(
        &self,
        target: &CallTarget,
        context: ProofContext<'_>,
        scratch: &mut ProofScratch,
        caller: Option<&ValidatedFacts>,
    ) -> Result<DirectCallProof> {
        if let Some(caller) = caller {
            self.check_facts(caller, true)?;
            if !std::ptr::eq(context.predicates, caller.predicates.as_ref())
                || context.phase != caller.phase()
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "checked call caller context changed",
                ));
            }
        }
        scratch.with_caller_arena(self.id, caller, |arena| {
            self.discharge_in_with_caller(target, context, arena, caller)
        })
    }
    fn discharge_in_with_caller(
        &self,
        target: &CallTarget,
        context: ProofContext<'_>,
        scratch: &mut ProofScratch,
        caller: Option<&ValidatedFacts>,
    ) -> Result<DirectCallProof> {
        self.ensure_provider(target.declaration)?;
        self.validate_closure_direct(target, context)?;
        let schema = self.callable_schema(target.declaration)?;
        let instantiated = self.instantiate_scratch_with_facts(
            &schema,
            &target.type_args,
            context,
            Some(target.declaration),
            scratch,
            caller,
        )?;
        self.validate_owner_callable_instantiation(
            target,
            &instantiated.signature,
            context,
            scratch,
        )?;
        let mut evidence = Vec::new();
        for predicate in instantiated.predicates.iter() {
            push(
                &self.budget,
                &mut evidence,
                self.require_in(predicate, context, scratch)?,
            )?;
        }
        Ok(DirectCallProof {
            target: target.clone(),
            signature: instantiated.signature,
            evidence: evidence.into(),
        })
    }
    fn goal_is_closed(&self, goal: &TraitRef, env: &GenericEnv) -> Result<bool> {
        self.goal_is_closed_with(goal, env, &mut ProofScratch::default())
    }
    fn closedness_of_root(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        usage: TypeUse,
        depth: usize,
        scratch: &mut ProofScratch,
    ) -> Result<Closedness> {
        self.closedness_of_root_with_storage(
            ty,
            env,
            usage,
            depth,
            &mut scratch.closedness,
            &mut scratch.closedness_storage,
        )
    }
    fn closedness_of_root_with_storage(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        usage: TypeUse,
        depth: usize,
        rows: &mut Vec<ClosednessValidation>,
        storage: &mut Vec<ir::PlanReservation>,
    ) -> Result<Closedness> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        for entry in rows.iter() {
            charged(&self.budget, self.budget.charge_work(1))?;
            if entry.world == self.id
                && entry.usage == usage
                && env.total_arity() == entry.env.total_arity()
                && std::ptr::eq(env.scopes(), entry.env.scopes())
                && same_closedness_input(ty, &entry.ty)
            {
                charged(&self.budget, self.budget.charge_nodes(1))?;
                charged(
                    &self.budget,
                    self.budget
                        .check_depth(depth.saturating_add(entry.result.height)),
                )?;
                checked_module(
                    &self.budget,
                    self.catalog
                        .modules()
                        .budget()
                        .check()
                        .map_err(ModuleError::from),
                )?;
                return Ok(entry.result);
            }
        }
        // A fresh walk is essential: sticky flags from a prior root cannot
        // describe this root, and memo hits must carry their own flags.
        let mut walk = TypeWalk::default();
        let height = self.validate_ty_inner(ty, env, depth, usage, &mut walk)?;
        let result = Closedness {
            height,
            has_parameters: walk.has_parameters,
            has_opaque: walk.has_opaque,
        };
        if !ty.is_nominal_back_edge() && rows.len() < MAX_SCRATCH_OBLIGATIONS {
            push_scratch_row(
                &self.budget,
                rows,
                storage,
                ClosednessValidation {
                    world: self.id,
                    ty: ty.clone(),
                    env: env.clone(),
                    usage,
                    result,
                },
            )?;
        }
        Ok(result)
    }
    fn goal_is_closed_with(
        &self,
        goal: &TraitRef,
        env: &GenericEnv,
        scratch: &mut ProofScratch,
    ) -> Result<bool> {
        self.goal_is_closed_with_storage(
            goal,
            env,
            &mut scratch.closedness,
            &mut scratch.closedness_storage,
        )
    }
    fn goal_is_closed_with_storage(
        &self,
        goal: &TraitRef,
        env: &GenericEnv,
        rows: &mut Vec<ClosednessValidation>,
        storage: &mut Vec<ir::PlanReservation>,
    ) -> Result<bool> {
        let root = self.closedness_of_root_with_storage(
            &goal.self_ty,
            env,
            TypeUse::Root,
            0,
            rows,
            storage,
        )?;
        let mut closed = !root.has_parameters && !root.has_opaque;
        for ty in goal.args.iter() {
            let actual =
                self.closedness_of_root_with_storage(ty, env, TypeUse::Actual, 0, rows, storage)?;
            closed &= !actual.has_parameters && !actual.has_opaque;
        }
        Ok(closed)
    }
    fn trait_call_is_closed(&self, call: &TraitCall, env: &GenericEnv) -> Result<bool> {
        let mut scratch = ProofScratch::default();
        Ok(
            self.goal_is_closed_with(&call.trait_ref, env, &mut scratch)?
                && self.method_args_are_closed_with(call, env, &mut scratch)?,
        )
    }

    /// Same validation walk as `goal_is_closed`, over a call's ordered method
    /// actuals. Open (parameter or opaque) actuals keep the call universal.
    fn method_args_are_closed_with(
        &self,
        call: &TraitCall,
        env: &GenericEnv,
        scratch: &mut ProofScratch,
    ) -> Result<bool> {
        let mut closed = true;
        for ty in call.method_args.iter() {
            let actual = self.closedness_of_root(ty, env, TypeUse::Actual, 0, scratch)?;
            closed &= !actual.has_parameters && !actual.has_opaque;
        }
        Ok(closed)
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
            (Ty::Cell(a), Ty::Cell(b)) => {
                return a.try_eq(b, |a, b| self.match_head(a, b, env, slots, depth + 1));
            }
            (Ty::ManuallyDrop(a), Ty::ManuallyDrop(b)) => {
                return a.try_eq(b, |a, b| self.match_head(a, b, env, slots, depth + 1));
            }
            (Ty::PhantomData(a), Ty::PhantomData(b)) => {
                return a.try_eq(b, |a, b| self.match_head(a, b, env, slots, depth + 1));
            }
            (Ty::FnPointer(a), Ty::FnPointer(b)) => {
                return a.try_eq(b, |a, b| self.match_head(a, b, env, slots, depth + 1));
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
            (
                Ty::RawPtr {
                    kind: a,
                    pointee: at,
                },
                Ty::RawPtr {
                    kind: b,
                    pointee: bt,
                },
            ) => return Ok(a == b && self.match_head(at, bt, env, slots, depth + 1)?),
            (Ty::Slice { element: a }, Ty::Slice { element: b }) => {
                return self.match_head(a, b, env, slots, depth + 1);
            }
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

impl ValidatedTraitWorld {
    /// `validate_signature` sharing the caller's node-local proof scratch.
    /// A call subject already validated with a faithfully identical
    /// signature under the same facts subject is not re-derived.
    pub(in crate::compiler) fn validate_signature_with(
        &self,
        subject: SignatureSubject<'_>,
        signature: &Signature,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        self.check_facts(facts, true)?;
        let SignatureSubject::Call(callee) = subject else {
            return self.validate_signature_uncached(subject, signature, facts, scratch);
        };
        scratch.for_facts(facts);
        let bucket = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            callee.hash(&mut hasher);
            signature.params.hash(&mut hasher);
            signature.result.hash(&mut hasher);
            hasher.finish()
        };
        if let Some(candidates) = scratch.signature_index.get(&bucket) {
            for &index in candidates {
                let (old_callee, old) = &scratch.signatures[index];
                charged(&self.budget, self.budget.charge_work(1))?;
                if same_callee(old_callee, callee)
                    && Arc::ptr_eq(&old.ref_contract, &signature.ref_contract)
                    && same_types(&old.params, &signature.params)
                    && same_type_allocation(&old.result, &signature.result)
                {
                    return Ok(());
                }
            }
        }
        self.validate_signature_uncached(subject, signature, facts, scratch)?;
        if scratch.signatures.len() < MAX_SCRATCH_OBLIGATIONS
            && scratch.signatures.try_reserve(1).is_ok()
        {
            scratch
                .signature_index
                .entry(bucket)
                .or_default()
                .push(scratch.signatures.len());
            scratch.signatures.push((callee.clone(), signature.clone()));
        }
        Ok(())
    }
    fn validate_signature_uncached(
        &self,
        subject: SignatureSubject<'_>,
        signature: &Signature,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        let expected = match subject {
            SignatureSubject::Body if facts.closure.is_some() => {
                self.closure_body_signature(facts)?
            }
            SignatureSubject::Body => match &facts.subject {
                FactsSubject::UniversalCallable(declaration) => {
                    let signature = &self.callable_header(*declaration)?.schema.signature;
                    retain::<Ty>(&self.budget, signature.params.len())?;
                    signature.clone()
                }
                FactsSubject::ClosedInstance(target) => {
                    self.discharge_direct_with(target, facts, scratch)?
                        .signature
                }
                FactsSubject::SchemaItem(_) | FactsSubject::AssociatedConstant(_) => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "schema facts cannot validate an executable body",
                    ));
                }
            },
            SignatureSubject::Call(Callee::Direct(target)) => {
                self.discharge_direct_with(target, facts, scratch)?
                    .signature
            }
            SignatureSubject::Call(
                Callee::TraitItem(call) | Callee::Virtual(ir::VirtualCall { call }),
            ) if !(self.schema_contains_projection(&self.item_header(call.item)?.schema)?
                && !self.goal_is_closed_with(
                    &call.trait_ref,
                    facts.env(),
                    scratch.for_world(self.id),
                )?
                && !matches!(&call.trait_ref.self_ty, Ty::Dyn(_))) =>
            {
                self.trait_signature_scratch(
                    call,
                    ProofContext::from_facts(facts),
                    scratch.for_world(self.id),
                )?
                .0
            }
            _ => return TraitCx::validate_signature(self, subject, signature, facts),
        };
        if !self.eq_signature(&expected, signature)? {
            return Err(FactsError::invalid(
                &self.budget,
                "signature differs from its exact canonical world subject",
            ));
        }
        scratch.for_facts(facts);
        for ty in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            scratch.signature_types.check_signature_root(facts, ty)?;
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
    /// Metadata validation sharing the caller's node-local replay memo.
    pub(in crate::compiler) fn validate_ty_metadata_scratch(
        &self,
        ty: &Ty,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        self.check_facts(facts, false)?;
        scratch.for_facts(facts);
        let proofs = scratch.for_world(self.id);
        self.validate_ty_metadata_with_facts(ty, facts.env(), 0, &mut proofs.replays, Some(facts))
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
            SignatureSubject::Body if facts.closure.is_some() => {
                self.closure_body_signature(facts)?
            }
            SignatureSubject::Body => match &facts.subject {
                FactsSubject::UniversalCallable(declaration) => {
                    let signature = &self.callable_header(*declaration)?.schema.signature;
                    retain::<Ty>(&self.budget, signature.params.len())?;
                    signature.clone()
                }
                FactsSubject::ClosedInstance(target) => {
                    self.discharge_direct(target, facts)?.signature
                }
                FactsSubject::SchemaItem(_) | FactsSubject::AssociatedConstant(_) => {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "schema facts cannot validate an executable body",
                    ));
                }
            },
            SignatureSubject::Call(Callee::Direct(target)) => {
                self.discharge_direct(target, facts)?.signature
            }
            SignatureSubject::Call(
                Callee::TraitItem(call)
                | Callee::Virtual(ir::VirtualCall { call })
                | Callee::Untupled(call),
            ) => self.instantiate_trait_signature(call, facts)?,
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
                if let Some(receiver) = self.closure_dispatch_receiver(target)? {
                    return Ok(receiver);
                }
                Ok(self.info(target.declaration)?.receiver)
            }
            Callee::TraitItem(call)
            | Callee::Virtual(ir::VirtualCall { call })
            | Callee::Untupled(call) => {
                self.instantiate_trait_signature(call, facts)?;
                Ok(self.item_header(call.item)?.receiver)
            }
        }
    }
    fn validate_trait_ref(&self, goal: &TraitRef, facts: &ValidatedFacts) -> Result<()> {
        self.check_facts(facts, true)?;
        self.validate_ref_in(goal, facts.env(), None)
    }
    fn validate_untupled(&self, call: &TraitCall, facts: &ValidatedFacts) -> Result<Signature> {
        self.check_facts(facts, true)?;
        self.validated_untupled(call, facts)
            .map(|(signature, _)| signature)
    }
    fn untupled_target(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<ir::DirectCallProofView> {
        self.check_facts(facts, true)?;
        self.validated_untupled(call, facts).map(|(_, view)| view)
    }
    fn instantiate_trait_signature(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<Signature> {
        self.instantiate_trait_signature_with(call, facts, &mut TraitScratch::default())
    }
    fn instantiate_trait_signature_with(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<Signature> {
        self.check_facts(facts, true)?;
        // This memo stores only this service's completed canonical outputs.
        // Keep the exact facts subject and the retained actual payloads, not
        // merely their declaration IDs or semantic type equality. In
        // particular a copied nominal with a forged definition/capability is
        // not the same input. Caller-supplied signatures are still validated
        // separately by validate_signature_with.
        scratch.for_facts(facts);
        if let Some(rows) = scratch.formed_trait_signature_index.get(&call.item) {
            for &row in rows {
                charged(&self.budget, self.budget.charge_work(1))?;
                let (original, signature) = &scratch.formed_trait_signatures[row];
                if same_trait_call(original, call) {
                    retain::<Ty>(&self.budget, signature.params.len())?;
                    return Ok(signature.clone());
                }
            }
        }
        let signature = self.instantiate_trait_signature_uncached(call, facts, scratch)?;
        if scratch.formed_trait_signatures.len() < MAX_SCRATCH_OBLIGATIONS {
            retain::<(TraitCall, Signature)>(&self.budget, 2)?;
            retain::<Ty>(&self.budget, signature.params.len())?;
            retain::<usize>(&self.budget, 2)?;
            scratch
                .formed_trait_signatures
                .try_reserve(1)
                .map_err(|_| {
                    FactsError::resource(&self.budget, "trait signature memo allocation")
                })?;
            scratch
                .formed_trait_signature_index
                .try_reserve(1)
                .map_err(|_| {
                    FactsError::resource(&self.budget, "trait signature index allocation")
                })?;
            let rows = scratch
                .formed_trait_signature_index
                .entry(call.item)
                .or_default();
            rows.try_reserve(1).map_err(|_| {
                FactsError::resource(&self.budget, "trait signature row allocation")
            })?;
            rows.push(scratch.formed_trait_signatures.len());
            scratch
                .formed_trait_signatures
                .push((call.clone(), signature.clone()));
        }
        Ok(signature)
    }
    fn prove_goal(&self, goal: &Predicate, facts: &ValidatedFacts) -> Result<ProofOutcome> {
        self.prove_goal_with(goal, facts, &mut TraitScratch::default())
    }
    fn prove_goal_with(
        &self,
        goal: &Predicate,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<ProofOutcome> {
        self.check_facts(facts, true)?;
        self.prove(goal, facts, scratch.for_world(self.id))
    }
    fn discharge_direct(
        &self,
        target: &CallTarget,
        facts: &ValidatedFacts,
    ) -> Result<DirectCallProof> {
        self.discharge_direct_with(target, facts, &mut TraitScratch::default())
    }
    fn discharge_direct_with(
        &self,
        target: &CallTarget,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<DirectCallProof> {
        self.check_facts(facts, true)?;
        scratch.for_facts(facts);
        let bucket = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            target.hash(&mut hasher);
            hasher.finish()
        };
        if let Some(candidates) = scratch.direct_index.get(&bucket) {
            for &index in candidates {
                charged(&self.budget, self.budget.charge_work(1))?;
                let old = &scratch.directs[index];
                if old.target.declaration == target.declaration
                    && same_types(&old.target.type_args, &target.type_args)
                {
                    retain::<DirectCallProof>(&self.budget, 1)?;
                    return Ok(old.clone());
                }
            }
        }
        let proof = self.discharge_in_with_facts(
            target,
            ProofContext::from_facts(facts),
            scratch.for_world(self.id),
            Some(facts),
        )?;
        if scratch.directs.len() < MAX_SCRATCH_OBLIGATIONS && scratch.directs.try_reserve(1).is_ok()
        {
            scratch
                .direct_index
                .entry(bucket)
                .or_default()
                .push(scratch.directs.len());
            scratch.directs.push(proof.clone());
        }
        Ok(proof)
    }
    fn resolve_trait_call(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<ResolvedTraitCall> {
        self.resolve_trait_call_with(call, facts, &mut TraitScratch::default())
    }
    fn resolve_trait_call_with(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<ResolvedTraitCall> {
        let scratch = scratch.for_world(self.id);
        self.check_facts(facts, true)?;
        let caller = ProofContext::from_facts(facts);
        if matches!(&call.trait_ref.self_ty, Ty::FnItem(_))
            && self
                .schema
                .closure_trait_kind(call.trait_ref.declaration)?
                .is_some()
        {
            // Builtin function item dispatch: no implementation selection.
            let (signature, _) = self.validated_untupled(call, facts)?;
            let evidence = self.require_in(
                &Predicate::Implements(call.trait_ref.clone()),
                caller,
                scratch,
            )?;
            return Ok(ResolvedTraitCall {
                callee: Callee::Untupled(call.clone()),
                signature,
                _evidence: evidence,
                _refinement: None,
                _topology: None,
                _required: None,
            });
        }
        if facts.phase() == FactsPhase::Universal
            && self.schema_contains_projection(&self.item_header(call.item)?.schema)?
            && (!self.goal_is_closed_with(&call.trait_ref, facts.env(), scratch)?
                || !self.method_args_are_closed_with(call, facts.env(), scratch)?)
            && !matches!(&call.trait_ref.self_ty, Ty::Dyn(_))
        {
            let (application, _) = self.instantiate_universal_required_item(call, facts)?;
            let required = self.universal_required_carrier_contract(call, facts)?;
            let evidence = self.require_in(
                &Predicate::Implements(call.trait_ref.clone()),
                caller,
                scratch,
            )?;
            return Ok(ResolvedTraitCall {
                callee: Callee::TraitItem(call.clone()),
                signature: application.signature,
                _evidence: evidence,
                _refinement: None,
                _topology: None,
                _required: Some(required),
            });
        }
        let (signature, caller_topology) = self.trait_signature_scratch(call, caller, scratch)?;
        let predicate = Predicate::Implements(call.trait_ref.clone());
        // Closedness covers the whole call: the trait goal AND the ordered
        // method actuals. A concrete Self with an open method actual (a
        // caller's own type parameter forwarded to a generic trait method)
        // stays in the caller's environment and phase; only a genuinely
        // closed instantiation is re-resolved in the empty closed world.
        if matches!(call.trait_ref.self_ty, Ty::Opaque(_))
            || (facts.phase() == FactsPhase::Universal
                && (!self.goal_is_closed_with(&call.trait_ref, facts.env(), scratch)?
                    || !self.method_args_are_closed_with(call, facts.env(), scratch)?))
        {
            let evidence = self.require_in(&predicate, caller, scratch)?;
            return Ok(ResolvedTraitCall {
                callee: if matches!(&call.trait_ref.self_ty, Ty::Dyn(_)) {
                    Callee::Virtual(ir::VirtualCall { call: call.clone() })
                } else {
                    Callee::TraitItem(call.clone())
                },
                signature,
                _evidence: evidence,
                _refinement: None,
                _topology: caller_topology,
                _required: None,
            });
        }
        // The same obligation engine runs with the world's private empty
        // environment, even when a caller body has unrelated generic bounds.
        let closed = if facts.phase() == FactsPhase::Closed
            && std::ptr::eq(facts.predicates.as_ref(), self.empty.as_ref())
        {
            // Re-resolution has exactly the same closed environment, not a
            // newly invented caller or an upgrade of a raw predicate context.
            ProofContext::from_facts(facts)
        } else {
            ProofContext {
                original: None,
                predicates: &self.empty,
                phase: FactsPhase::Closed,
            }
        };
        let (signature, topology) = self.trait_signature_with_topology(call, closed)?;
        let evidence = self.require_in(&predicate, closed, scratch)?;
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
        if matches!(&evidence.0.origin, ProofOrigin::TraitObject) {
            let Ty::Dyn(object) = &call.trait_ref.self_ty else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "dynamic dispatch lost object identity",
                ));
            };
            let methods = facts.object_vtable_methods(object)?;
            if !methods
                .iter()
                .any(|(application, item)| application == &call.trait_ref && *item == call.item)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "dynamic dispatch item not in exact object vtable",
                ));
            }
            return Ok(ResolvedTraitCall {
                callee: Callee::Virtual(ir::VirtualCall { call: call.clone() }),
                signature,
                _evidence: evidence,
                _refinement: None,
                _topology: topology,
                _required: None,
            });
        }
        let mut target = None;
        if let ProofOrigin::Closure { kind } = &evidence.0.origin {
            let Ty::Closure(closure) = &call.trait_ref.self_ty else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure proof lost its environment type",
                ));
            };
            target = Some(self.closure_target(closure, *kind)?);
        } else if let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &evidence.0.origin
        {
            let positions = self.impl_header_index.get(&self.budget, *declaration)?;
            for h in positions.iter().map(|&p| &self.headers.impls[p]) {
                if h.declaration == *declaration {
                    for (item, body) in h.items.iter() {
                        charged(&self.budget, self.budget.charge_work(1))?;
                        if *item == call.item {
                            let mut type_args = arguments.to_vec();
                            for argument in call.method_args.iter() {
                                push(&self.budget, &mut type_args, argument.clone())?;
                            }
                            target = Some(CallTarget {
                                declaration: *body,
                                type_args: type_args.into(),
                            });
                            break;
                        }
                    }
                    if target.is_none()
                        && let Some(default) = self.item_header(call.item)?.default
                    {
                        self.ensure_provider(default)?;
                        let mut type_args = self.trait_arguments(&call.trait_ref)?;
                        for argument in call.method_args.iter() {
                            push(&self.budget, &mut type_args, argument.clone())?;
                        }
                        target = Some(CallTarget {
                            declaration: default,
                            type_args: type_args.into(),
                        });
                    }
                    break;
                }
            }
        } else if let ProofOrigin::Builtin {
            item: CatalogItem::Clone,
        } = &evidence.0.origin
        {
            let marker_clone_from = call.item == self.catalog.id(CatalogItem::CloneFromItem)
                && matches!(&call.trait_ref.self_ty, Ty::PhantomData(_));
            if call.item != self.catalog.id(CatalogItem::CloneItem) && !marker_clone_from {
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
                Ty::ManuallyDrop(_) => {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: ManuallyDrop Clone executable provider is not implemented",
                    ));
                }
                Ty::PhantomData(marker) => {
                    facts.validate_ty(&call.trait_ref.self_ty)?;
                    push(&self.budget, &mut args, marker.argument().clone())?;
                    if marker_clone_from {
                        RecipeDescriptor::CloneFromPhantomData
                    } else {
                        RecipeDescriptor::ClonePhantomData
                    }
                }
                Ty::Int(i) => RecipeDescriptor::CloneInt(*i),
                Ty::Float(float) => RecipeDescriptor::CloneFloat(*float),
                Ty::Bool => RecipeDescriptor::CloneBool,
                Ty::Char => RecipeDescriptor::CloneChar,
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
        let direct = self.discharge_in_with_facts(&target, closed, scratch, closed.original)?;
        let mut return_matches = self.reference_free_implementation_refines(
            &signature,
            &direct.signature,
            closed.env(),
            closed.original,
        )?;
        if !return_matches
            && !target.type_args.is_empty()
            && self.type_contains_projection(
                &self.item_header(call.item)?.schema.signature.result,
                0,
            )?
        {
            // Generic implementations retain their rigid envelope contract.
            // Compare projected required signatures against the independently
            // derived concrete specialization, not against that wider envelope.
            let results = charged(
                &self.budget,
                ir::derive_return_contracts(
                    &direct.signature.params,
                    &direct.signature.result,
                    &direct.signature.ref_contract,
                    &self.budget,
                ),
            )?;
            let concrete = ir::RefContract {
                results,
                ..(*direct.signature.ref_contract).clone()
            };
            return_matches = self.return_refines(&signature.ref_contract, &concrete)?;
        }
        let types_match = self.eq_types(&signature.params, &direct.signature.params, 0)?
            && self.eq_ty(&signature.result, &direct.signature.result, 0)?;
        if types_match && !return_matches && !target.type_args.is_empty() {
            // Check erased generic result positions before deriving origins from
            // concrete types. Distinct trait parameters may specialize to the
            // same reference type while the provider retains one input binder.
            // The existing erasure proof still requires exact input positions
            // and only admits original input-free, origin-free result regions.
            return_matches = self.erased_result_regions_refine(
                &signature.ref_contract,
                &direct.signature.ref_contract,
            )?;
        }
        if types_match && !return_matches {
            // The required signature may name a graph-realized nominal whose
            // field layout is not visible here, deriving an empty origin set.
            // The types are proven equal, so the canonical required contract
            // is the same derivation over the provider's equal types under the
            // required region structure. Nothing else is relaxed.
            let results = charged(
                &self.budget,
                ir::derive_return_contracts(
                    &direct.signature.params,
                    &direct.signature.result,
                    &signature.ref_contract,
                    &self.budget,
                ),
            )?;
            let required = ir::RefContract {
                results,
                ..(*signature.ref_contract).clone()
            };
            return_matches = self.return_refines(&required, &direct.signature.ref_contract)?;
            if !return_matches && !target.type_args.is_empty() {
                // Substituting an erased reference type argument (`Borrowed =
                // &str`) gives its inner region a fresh identity bound to no
                // input. Generic bodies were already region-checked, so an
                // input-free required region at an equal type position is an
                // erasure artifact: only those positions may differ.
                return_matches =
                    self.erased_result_regions_refine(&required, &direct.signature.ref_contract)?;
            }
        }
        if !types_match || !return_matches {
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
            _required: None,
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
        modules: Arc<ModuleArena>,
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
                    module: fixture_root(&self.modules, &self.budget)?,
                    visibility: crate::compiler::ir::Visibility::Restricted(fixture_root(
                        &self.modules,
                        &self.budget,
                    )?),
                    receiver,
                    region_contract_mode: RegionContractMode::Positioned,
                },
            )
        }
        fn gather(&mut self, ty: &Ty, depth: usize) -> Result<()> {
            charged(&self.budget, self.budget.check_depth(depth))?;
            charged(&self.budget, self.budget.charge_work(1))?;
            if let Some(id) = nominal_id(ty) {
                validate_nominal_modules(ty, &self.modules, &self.budget)?;
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
                | Ty::Slice { element }
                | Ty::RawPtr {
                    pointee: element, ..
                }
                | Ty::Ref {
                    referent: element, ..
                } => self.gather(element, depth + 1)?,
                Ty::Struct(s) => {
                    for f in nominal_fields(&s.def().fields, &self.budget)?.iter() {
                        self.gather(&f.ty, depth + 1)?
                    }
                }
                Ty::Enum(e) => {
                    for v in e.def().variants.iter() {
                        for f in nominal_fields(&v.fields, &self.budget)?.iter() {
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
        | Ty::Slice { element }
        | Ty::Ref {
            referent: element, ..
        } = ty
        {
            return namespace(element, budget, depth + 1);
        }
        Ok(None)
    }
    fn fixture_root(modules: &ModuleArena, budget: &PlanBudget) -> Result<ModuleId> {
        match modules.source_root() {
            Ok(root) => Ok(root),
            Err(ModuleError::SourceUnavailable) => {
                checked_module(budget, modules.check(modules.core_root()))?;
                Ok(modules.core_root())
            }
            Err(error) => checked_module(budget, Err(error)),
        }
    }
    pub(super) fn adapter(
        modules: Arc<ModuleArena>,
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
            None => charged(budget, SnapshotId::standalone())?,
        };
        let mut schema = FixtureSchema {
            registry,
            modules: modules.clone(),
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
            for item in headers.items.iter() {
                charged(budget, budget.charge_work(1))?;
                if item.default == Some(h.declaration) {
                    owner = Some(item.trait_decl);
                    receiver = item.receiver;
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
            for predicate in raw.predicates.iter() {
                for ty in predicate.metadata_types() {
                    schema.gather(ty, 0)?;
                }
            }
        }
        for h in headers.impls.iter() {
            for predicate in h.predicates.iter() {
                for ty in predicate.metadata_types() {
                    schema.gather(ty, 0)?;
                }
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
        let builder = CoreCatalogBuilder::new(registry, modules, budget)?;
        let end = builder.source_start();
        for decl in schema
            .declarations
            .iter()
            .map(|d| d.declaration)
            .chain(schema.nominals.iter().filter_map(nominal_decl))
        {
            charged(budget, budget.charge_work(1))?;
            if !decl.is_source() {
                return Err(FactsError::invalid(
                    budget,
                    "fixture source declaration collides with canonical catalog prefix",
                ));
            }
        }
        schema.catalog = Some(builder.finish(end, RecipeShapeInventory::new(), budget)?);
        Ok(Arc::new(schema))
    }
    impl SchemaCx for FixtureSchema {
        fn impl_trait_safety(
            &self,
            implementation: DeclId,
            trait_decl: DeclId,
        ) -> Result<(bool, bool)> {
            let implementation = self.declaration_info(implementation)?;
            let declaration = self.declaration_info(trait_decl)?;
            if implementation.kind != DeclKind::TraitImpl || declaration.kind != DeclKind::Trait {
                return Err(FactsError::invalid(
                    &self.budget,
                    "fixture safety changed declaration kind",
                ));
            }
            // This fixed synthetic fixture language contains safe traits only.
            Ok((false, false))
        }
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
                carriers: Arc::from([]),
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
            for predicate in schema.predicates.iter() {
                let goal = predicate.trait_ref();
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
                    match predicate {
                        Predicate::Implements(_) => Predicate::Implements(TraitRef {
                            declaration: goal.declaration,
                            self_ty,
                            args: trait_args.into(),
                        }),
                        Predicate::ProjectionEq { projection, ty } => Predicate::ProjectionEq {
                            projection: ir::AssociatedTypeProjection {
                                item: projection.item,
                                trait_ref: TraitRef {
                                    declaration: goal.declaration,
                                    self_ty,
                                    args: trait_args.into(),
                                },
                            },
                            ty: charged(budget, substitute(ty, &mut cx))?,
                        },
                    },
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
            validate_nominal_modules(ty, &self.modules, budget)?;
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
        modules: Arc<ModuleArena>,
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
        let raw = adapter(modules, &headers, &[], budget)?;
        ValidatedTraitWorld::build(headers, raw, budget)
    }
    pub fn facts_for_signature(
        modules: Arc<ModuleArena>,
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
            None => charged(budget, SnapshotId::standalone())?,
        };
        // Discover nominal declaration indices before choosing the fixture's
        // callable identity. A nominal at index zero is not a function at zero.
        let empty_headers = HeaderSet::default();
        let mut raw = FixtureSchema {
            registry,
            modules: modules.clone(),
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
        let raw = adapter(modules, &headers, types, budget)?;
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
    #[test]
    fn scratch_row_storage_exact_short_and_growth_peak() {
        let row = scratch_row_bytes::<u64>().unwrap();
        let overlap = std::mem::size_of::<u64>() + std::mem::size_of::<ir::PlanReservation>();
        for short in [false, true] {
            let budget = PlanBudget::new(ir::PlanLimits {
                bytes: row - usize::from(short),
                ..ir::PlanLimits::default()
            });
            let mut values = Vec::new();
            let mut leases = Vec::new();
            let result = push_scratch_row(&budget, &mut values, &mut leases, 7u64);
            if short {
                assert!(result.unwrap_err().is_resource());
                assert_eq!(values.capacity(), 0);
                assert_eq!(leases.capacity(), 0);
                assert_eq!(budget.usage_snapshot().bytes, 0);
                assert!(budget.first_resource_failure().is_some());
            } else {
                result.unwrap();
                assert_eq!(budget.usage_snapshot().bytes, row);
                assert_eq!(values, [7]);
                drop(values);
                drop(leases);
                assert_eq!(budget.usage_snapshot().bytes, 0);
                assert!(!budget.has_resource_failure());
            }
        }
        for short in [false, true] {
            let budget = PlanBudget::new(ir::PlanLimits {
                bytes: 2 * row + overlap - usize::from(short),
                ..ir::PlanLimits::default()
            });
            let mut values = Vec::new();
            let mut leases = Vec::new();
            push_scratch_row(&budget, &mut values, &mut leases, 1u64).unwrap();
            assert_eq!(values.capacity(), 1);
            assert_eq!(leases.capacity(), 1);
            let result = push_scratch_row(&budget, &mut values, &mut leases, 2u64);
            if short {
                assert!(result.unwrap_err().is_resource());
                assert_eq!(values, [1]);
                assert_eq!(leases.len(), 1);
                assert_eq!(budget.usage_snapshot().bytes, row);
                let first = budget.first_resource_failure();
                drop(values);
                drop(leases);
                assert_eq!(budget.usage_snapshot().bytes, 0);
                assert_eq!(budget.first_resource_failure(), first);
            } else {
                result.unwrap();
                assert_eq!(values, [1, 2]);
                assert_eq!(budget.usage_snapshot().bytes, 2 * row);
                assert_eq!(budget.usage_snapshot().work, 2);
                drop(values);
                drop(leases);
                assert_eq!(budget.usage_snapshot().bytes, 0);
            }
        }
        let budget = PlanBudget::default();
        let mut values = Vec::new();
        let mut leases = Vec::new();
        for value in 0u64..65 {
            push_scratch_row(&budget, &mut values, &mut leases, value).unwrap();
            assert_eq!(values.len(), leases.len());
            assert!(values.capacity() <= 2 * values.len());
            assert!(leases.capacity() <= 2 * leases.len());
            assert_eq!(budget.usage_snapshot().bytes, values.len() * row);
        }
        assert_eq!(budget.usage_snapshot().work, 254);
        drop(values);
        drop(leases);
        assert_eq!(budget.usage_snapshot().bytes, 0);
    }

    #[test]
    fn scratch_row_postgrowth_denial_destroys_buffers_before_refund() {
        let budget = PlanBudget::default();
        let input = Arc::new(23u64);
        let weak = Arc::downgrade(&input);
        retain::<u64>(&budget, 1).unwrap();
        let payload_bytes = budget.usage_snapshot().bytes;
        let mut values = Vec::new();
        let mut leases = Vec::new();
        push_scratch_row(&budget, &mut values, &mut leases, input.clone()).unwrap();
        assert_eq!(Arc::strong_count(&input), 2);
        let before_work = budget.usage_snapshot().work;
        let error = push_scratch_row_with(&budget, &mut values, &mut leases, input.clone(), || {
            charged(&budget, budget.charge_work(usize::MAX))?;
            Ok(())
        })
        .unwrap_err();
        assert!(error.is_resource());
        assert_eq!(values.capacity(), 0);
        assert_eq!(leases.capacity(), 0);
        assert_eq!(Arc::strong_count(&input), 1);
        assert!(weak.upgrade().is_some());
        assert_eq!(budget.usage_snapshot().bytes, payload_bytes);
        assert_eq!(budget.usage_snapshot().work, before_work + 2);
        let first = budget.first_resource_failure();
        drop(values);
        drop(leases);
        assert_eq!(budget.first_resource_failure(), first);
        assert_eq!(budget.usage_snapshot().bytes, payload_bytes);
    }

    #[test]
    fn scratch_row_postgrowth_cleanup_keeps_overlap_paid_during_row_drop() {
        struct Observe<'a> {
            budget: &'a PlanBudget,
            dropped: &'a std::cell::Cell<bool>,
            observe: bool,
        }
        impl Drop for Observe<'_> {
            fn drop(&mut self) {
                if self.observe {
                    let row = scratch_row_bytes::<Self>().unwrap();
                    let overlap =
                        std::mem::size_of::<Self>() + std::mem::size_of::<ir::PlanReservation>();
                    assert_eq!(self.budget.usage_snapshot().bytes, 2 * row + overlap);
                    self.dropped.set(true);
                }
            }
        }
        let budget = PlanBudget::default();
        let dropped = std::cell::Cell::new(false);
        let mut values = Vec::new();
        let mut leases = Vec::new();
        push_scratch_row(
            &budget,
            &mut values,
            &mut leases,
            Observe {
                budget: &budget,
                dropped: &dropped,
                observe: true,
            },
        )
        .unwrap();
        let result = push_scratch_row_with(
            &budget,
            &mut values,
            &mut leases,
            Observe {
                budget: &budget,
                dropped: &dropped,
                observe: false,
            },
            || charged(&budget, budget.charge_work(usize::MAX)),
        );
        assert!(result.unwrap_err().is_resource());
        assert!(dropped.get());
        assert_eq!(values.capacity(), 0);
        assert_eq!(leases.capacity(), 0);
        assert_eq!(budget.usage_snapshot().bytes, 0);
        assert!(budget.has_resource_failure());
    }

    #[test]
    fn scratch_row_poison_preserves_charges_and_first_failure() {
        let budget = PlanBudget::default();
        let mut values = Vec::new();
        let mut leases = Vec::new();
        push_scratch_row(&budget, &mut values, &mut leases, 1u64).unwrap();
        assert!(budget.charge_work(usize::MAX).is_err());
        let first = budget.first_resource_failure();
        let bytes = budget.usage_snapshot().bytes;
        budget.poison_ledger_for_tests();
        drop(values);
        drop(leases);
        assert_eq!(budget.usage_snapshot().bytes, bytes);
        assert_eq!(budget.first_resource_failure(), first);
        let mut values = Vec::new();
        let mut leases = Vec::new();
        assert!(
            push_scratch_row(&budget, &mut values, &mut leases, 2u64)
                .unwrap_err()
                .is_resource()
        );
        assert_eq!(budget.first_resource_failure(), first);
        assert!(values.is_empty());
    }

    #[test]
    fn trait_vector_growth_is_paid_and_amortized() {
        let budget = PlanBudget::new(ir::PlanLimits::default());
        let mut values = Vec::new();
        for index in 0u64..1025 {
            push(&budget, &mut values, index).unwrap();
            assert!(values.capacity() <= 2 * values.len());
        }
        assert_eq!(values, (0u64..1025).collect::<Vec<_>>());
        assert_eq!(budget.usage_snapshot().work, 2047);
        let exhausted = PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..ir::PlanLimits::default()
        });
        let mut unpaid = Vec::new();
        assert!(
            push(&exhausted, &mut unpaid, 1u64)
                .unwrap_err()
                .is_resource()
        );
        assert!(unpaid.is_empty());
    }

    #[test]
    fn local_type_memo_growth_is_paid_checked_and_amortized() {
        assert_eq!(TypeWalk::growth_slots(0, 0), Some(1));
        assert_eq!(TypeWalk::growth_slots(3, 4), Some(0));
        assert_eq!(TypeWalk::growth_slots(4, 4), Some(4));
        assert_eq!(TypeWalk::growth_slots(usize::MAX, usize::MAX), None);
        assert_eq!(TypeWalk::growth_slots(2, 1), None);
        let budget = PlanBudget::new(ir::PlanLimits::default());
        let mut memo = TypeWalk::default();
        for index in 0..65 {
            let before = budget.usage_snapshot();
            let old_capacity = memo.complete.capacity();
            memo.insert(
                (TypeNode::Tuple(index, 1), TypeUse::Root),
                index,
                &Ty::Unit,
                &budget,
            )
            .unwrap();
            let after = budget.usage_snapshot();
            assert_eq!(
                after.work - before.work,
                if index == old_capacity { 2 * index } else { 0 }
            );
            assert!(memo.complete.capacity() <= 2 * memo.complete.len());
            assert!(memo.reservations.capacity() <= 2 * memo.reservations.len());
            assert!(after.bytes > before.bytes);
        }
        assert_eq!(budget.usage_snapshot().work, 254);
        assert_eq!(
            memo.lookup((TypeNode::Tuple(64, 1), TypeUse::Root), &Ty::Unit, &budget)
                .unwrap(),
            Some(64)
        );
        let exhausted = PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..ir::PlanLimits::default()
        });
        let mut unpaid = TypeWalk::default();
        assert!(
            unpaid
                .insert(
                    (TypeNode::Tuple(0, 1), TypeUse::Root),
                    0,
                    &Ty::Unit,
                    &exhausted
                )
                .unwrap_err()
                .is_resource()
        );
        assert!(unpaid.complete.is_empty());
        assert!(unpaid.reservations.is_empty());
    }

    use super::*;
    use ir::{
        BinderId, BorrowKind, FieldDef, IntTy, NominalAppId, RefContract, StructDef, StructTy,
    };

    struct Case {
        modules: Arc<ModuleArena>,
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
    #[test]
    fn body_replay_live_guard_rejects_forged_context_and_module_failure() {
        use crate::compiler::expansion_budget::Counter;
        let program =
            crate::compiler::frontend::parse("fn probe<T>(x:T)->T{x}fn main(){}").unwrap();
        let budget = PlanBudget::default();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let view = facts.carrier_contract(SignatureSubject::Body).unwrap();
        view.check_body_replay_context(&facts).unwrap();
        for control in 0..5 {
            let mut forged = view.as_ref().clone();
            match control {
                0 => forged.registry = SnapshotId::standalone().unwrap(),
                1 => forged.environment = usize::MAX,
                2 => forged.phase = FactsPhase::Schema,
                3 => forged.world = WorldId(SnapshotId::standalone().unwrap()),
                4 => forged.budget = PlanBudget::default(),
                _ => unreachable!(),
            }
            assert!(matches!(forged.check_body_replay_context(&facts),
                Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        }
        let modules = facts.world().catalog.modules();
        let module_budget = modules.budget();
        assert!(
            module_budget
                .charge(
                    Counter::Work,
                    module_budget.limits().counters[Counter::Work as usize]
                )
                .is_err()
        );
        assert!(matches!(view.check_body_replay_context(&facts),
            Err(error) if error.kind() == FactsErrorKind::Resource));
    }

    fn fixture_modules() -> Arc<ModuleArena> {
        super::super::module_identity::ModuleArenaBuilder::for_catalog(
            super::super::expansion_budget::ExpansionBudget::default(),
            crate::compiler::owner_path::standalone_issued(
                &super::super::expansion_budget::ExpansionBudget::default(),
                crate::compiler::owner_path::OwnerDomain::ModuleArena,
            )
            .unwrap(),
        )
        .unwrap()
        .freeze()
        .unwrap()
    }
    fn nominal(
        module: ModuleId,
        registry: SnapshotId,
        index: usize,
        declaration: DeclId,
        args: Vec<Ty>,
    ) -> Ty {
        Ty::Struct(StructTy::new(StructDef {
            id: NominalAppId {
                snapshot: registry,
                index: ir::NominalIndex::SourceSlot(index),
            },
            source_decl: declaration,
            type_args: args.into(),
            name: "fixture".into(),
            module,
            visibility: crate::compiler::ir::Visibility::Restricted(module),
            fields: Arc::<[FieldDef]>::from([]).into(),
        }))
    }
    fn case() -> Case {
        let modules = fixture_modules();
        let budget = PlanBudget::default();
        let registry = SnapshotId::standalone().unwrap();
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
        let leaf = nominal(modules.core_root(), registry, 2, id(6), Vec::new());
        let pattern = nominal(
            modules.core_root(),
            registry,
            0,
            id(4),
            vec![parameter.clone()],
        );
        let boxed = nominal(modules.core_root(), registry, 1, id(4), vec![leaf.clone()]);
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
            constants: Arc::from([]),
            associated_types: Arc::from([]),
            traits: Arc::from([
                TraitHeader {
                    declaration: id(0),
                    self_param: ParamId {
                        owner: marker_self.binder(),
                        index: 0,
                    },
                    env: marker_env,
                    items: Arc::from([]),
                    associated_types: Arc::from([]),
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
                    associated_types: Arc::from([]),
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
                    associated_types: Arc::from([]),
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
                    associated_types: Arc::from([]),
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
            modules,
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

    #[test]
    fn cell_metadata_rejects_catalog_parameter_transplant_and_is_never_copy() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let scalar = Ty::Cell(ir::CellTy::new(&world.catalog, Ty::Bool, &c.budget).unwrap());
        facts.validate_ty(&scalar).unwrap();
        let goal = |item| {
            Predicate::Implements(TraitRef {
                declaration: world.catalog.id(item),
                self_ty: scalar.clone(),
                args: Arc::from([]),
            })
        };
        assert!(matches!(
            world.prove_goal(&goal(CatalogItem::Sized), &facts).unwrap(),
            ProofOutcome::Proved(_)
        ));
        assert!(matches!(
            world.prove_goal(&goal(CatalogItem::Copy), &facts).unwrap(),
            ProofOutcome::NotProven
        ));
        assert!(
            world
                .prove_goal(&goal(CatalogItem::Clone), &facts)
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
        let row = world
            .catalog
            .declaration(world.catalog.id(CatalogItem::CellNew))
            .unwrap();
        let raw = &row.schema.as_ref().unwrap().signature.result;
        assert!(matches!(raw, Ty::Cell(cell) if !cell.is_concrete()));
        let error = facts.validate_ty(raw).unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::Unsupported);
        world
            .validate_ty_metadata_in(raw, &row.info.env, 0)
            .unwrap();
        let other = world
            .catalog
            .declaration(world.catalog.id(CatalogItem::ManuallyDropNew))
            .unwrap();
        assert!(
            world
                .validate_ty_metadata_in(raw, &other.info.env, 0)
                .is_err()
        );
        assert!(
            world
                .catalog
                .validate_cell_schema_environment(&row.info.env, Some(c.caller))
                .is_err()
        );
        for argument in [
            c.caller_parameter.clone(),
            c.leaf.clone(),
            Ty::Unit,
            Ty::Never,
        ] {
            assert!(
                ir::CellTy::new(&world.catalog, argument, &c.budget)
                    .unwrap_err()
                    .starts_with("unsupported:")
            );
        }
    }
    #[test]
    fn impl_owned_phantom_sized_uses_original_parameter_evidence_not_default_assumptions() {
        let program = crate::compiler::frontend::parse(
            "trait Strategy{}struct G<S:Strategy>{marker:core::marker::PhantomData<S>}unsafe impl<S:Strategy> Sync for G<S>{}fn main(){}",
        ).unwrap();
        let budget = PlanBudget::default();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let implementation = world
            .impl_headers()
            .iter()
            .find(|header| header.trait_ref.declaration == world.catalog.id(CatalogItem::Sync))
            .unwrap();
        let parameter = implementation.schema_env.ordered_params().next().unwrap();
        assert!(
            world
                .predicates_prove_sized(parameter, &implementation.predicates)
                .unwrap()
        );
        world
            .validate_phantom_sized_argument(&Ty::Param(parameter), None, 0)
            .unwrap();
        let invalid_index = ParamId {
            index: 1,
            ..parameter
        };
        assert_eq!(
            world
                .validate_phantom_sized_argument(&Ty::Param(invalid_index), None, 0)
                .unwrap_err()
                .kind(),
            FactsErrorKind::InvalidMetadata
        );
        let foreign = ParamId {
            owner: BinderId {
                declaration: DeclId {
                    snapshot: SnapshotId::standalone().unwrap(),
                    index: parameter.owner.declaration.index,
                },
            },
            ..parameter
        };
        assert_eq!(
            world
                .validate_phantom_sized_argument(&Ty::Param(foreign), None, 0)
                .unwrap_err()
                .kind(),
            FactsErrorKind::InvalidMetadata
        );

        // The synthetic fixture has an authenticated generic environment but
        // no declared Sized predicate for its implementation parameter.
        // A default-looking environment alone must not grant permission.
        let c = case();
        let mut fixture = c.build().unwrap();
        let parameter = fixture.impl_headers()[0]
            .schema_env
            .ordered_params()
            .next()
            .unwrap();
        assert!(
            !fixture
                .predicates_prove_sized(parameter, &fixture.impl_headers()[0].predicates)
                .unwrap()
        );
        assert_eq!(
            fixture
                .validate_phantom_sized_argument(&Ty::Param(parameter), None, 0)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Unsupported
        );
        let sized = fixture.catalog.id(CatalogItem::Sized);
        Arc::make_mut(&mut Arc::get_mut(&mut fixture).unwrap().headers.impls)[0].predicates =
            Arc::from([Predicate::Implements(TraitRef {
                declaration: sized,
                self_ty: c.caller_parameter.clone(),
                args: Arc::from([]),
            })]);
        assert_eq!(
            fixture
                .validate_phantom_sized_argument(&Ty::Param(parameter), None, 0)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Unsupported
        );
        Arc::make_mut(&mut Arc::get_mut(&mut fixture).unwrap().headers.impls)[0].schema_env = c
            .headers
            .callables
            .iter()
            .find(|header| header.declaration == c.caller)
            .unwrap()
            .schema
            .env
            .clone();
        assert_eq!(
            fixture
                .validate_phantom_sized_argument(&Ty::Param(parameter), None, 0)
                .unwrap_err()
                .kind(),
            FactsErrorKind::InvalidMetadata
        );
    }

    #[test]
    fn impl_owned_relaxed_parameter_does_not_gain_phantom_sized_permission() {
        let program = crate::compiler::frontend::parse(
            "trait Tr{}struct G<T:?Sized>{p:*const T}impl<T:?Sized> Tr for G<T>{}fn main(){}",
        )
        .unwrap();
        let budget = PlanBudget::default();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let parameter = world.impl_headers()[0]
            .schema_env
            .ordered_params()
            .next()
            .unwrap();
        assert!(
            !world
                .predicates_prove_sized(parameter, &world.impl_headers()[0].predicates)
                .unwrap()
        );
        assert_eq!(
            world
                .validate_phantom_sized_argument(&Ty::Param(parameter), None, 0)
                .unwrap_err()
                .kind(),
            FactsErrorKind::Unsupported
        );
    }

    #[test]
    fn manually_drop_carrier_effect_lens_keeps_payload_and_shared_barriers() {
        let c = case();
        let world = c.build().unwrap();
        let Ty::Param(parameter) = c.caller_parameter.clone() else {
            panic!("parameter fixture")
        };
        let wrapper = Ty::ManuallyDrop(
            ir::ManuallyDropTy::new(&world.catalog, c.caller_parameter.clone(), &c.budget).unwrap(),
        );
        let location = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([
                ir::CarrierStep::Referent,
                ir::CarrierStep::ManuallyDropPayload,
            ])),
            channel: ir::CarrierChannel::ParamTail {
                parameter,
                prefix: 0,
            },
        };
        let exclusive = Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(wrapper.clone()),
        };
        assert!(world.carrier_writable(&exclusive, &location).unwrap());
        let shared = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(wrapper.clone()),
        };
        assert!(!world.carrier_writable(&shared, &location).unwrap());
        let ordinary_field = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([
                ir::CarrierStep::Referent,
                ir::CarrierStep::Field(0),
            ])),
            channel: location.channel.clone(),
        };
        assert!(world.carrier_writable(&exclusive, &ordinary_field).is_err());
        let not_wrapper = Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(Ty::Tuple(Arc::from([c.caller_parameter.clone()]))),
        };
        assert!(world.carrier_writable(&not_wrapper, &location).is_err());
        let actual_locations = ir::carrier_locations(&exclusive, &c.budget).unwrap();
        assert!(actual_locations.iter().any(|actual| actual == &location));
    }
    #[test]
    fn manually_drop_metadata_authenticates_actual_and_copy_requires_payload_copy() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let make = |argument| {
            Ty::ManuallyDrop(ir::ManuallyDropTy::new(&world.catalog, argument, &c.budget).unwrap())
        };
        let canonical_parameter = ParamId {
            owner: ir::BinderId {
                declaration: world.catalog.id(CatalogItem::ManuallyDrop),
            },
            index: 0,
        };
        world
            .validate_phantom_sized_argument(&Ty::Param(canonical_parameter), None, 0)
            .unwrap();
        assert!(
            world
                .validate_phantom_sized_argument(
                    &Ty::Param(ParamId {
                        index: 1,
                        ..canonical_parameter
                    }),
                    None,
                    0
                )
                .is_err()
        );
        let reference = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(Ty::Bool),
        };
        world
            .validate_phantom_sized_argument(&reference, None, 0)
            .unwrap();
        let stored_reference = make(reference.clone());
        facts.validate_ty(&stored_reference).unwrap();
        for rejected_marker_argument in [reference, stored_reference] {
            assert!(
                ir::PhantomDataTy::new(&world.catalog, rejected_marker_argument, &c.budget)
                    .unwrap_err()
                    .starts_with("unsupported:")
            );
        }
        let generic = make(c.caller_parameter.clone());
        facts.validate_ty(&generic).unwrap();
        assert!(
            world
                .callable_facts(c.body)
                .unwrap()
                .validate_ty(&generic)
                .is_err()
        );
        let scalar = make(Ty::Bool);
        let noncopy = make(c.leaf.clone());
        for ty in [&scalar, &noncopy] {
            facts.validate_ty(ty).unwrap();
            let goal = Predicate::Implements(TraitRef {
                declaration: world.catalog.id(CatalogItem::Sized),
                self_ty: ty.clone(),
                args: Arc::from([]),
            });
            assert!(matches!(
                world.prove_goal(&goal, &facts).unwrap(),
                ProofOutcome::Proved(_)
            ));
        }
        let goal = |ty: Ty| {
            Predicate::Implements(TraitRef {
                declaration: world.catalog.id(CatalogItem::Copy),
                self_ty: ty,
                args: Arc::from([]),
            })
        };
        assert!(matches!(
            world.prove_goal(&goal(scalar.clone()), &facts).unwrap(),
            ProofOutcome::Proved(_)
        ));
        assert!(matches!(
            world.prove_goal(&goal(noncopy.clone()), &facts).unwrap(),
            ProofOutcome::NotProven
        ));
        assert!(
            !world
                .types_equal(
                    std::slice::from_ref(&scalar),
                    std::slice::from_ref(&noncopy)
                )
                .unwrap()
        );
        let unsized_argument = make(Ty::Str);
        assert!(
            facts
                .validate_ty(&unsized_argument)
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
    }
    #[test]
    fn phantom_data_metadata_keeps_argument_authority_without_copy_component_bounds() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let pointer = Ty::FnPointer(
            ir::FnPointerTy::new(
                vec![c.caller_parameter.clone()],
                c.caller_parameter.clone(),
                &c.budget,
            )
            .unwrap(),
        );
        let marker =
            Ty::PhantomData(ir::PhantomDataTy::new(&world.catalog, pointer, &c.budget).unwrap());
        facts.validate_ty(&marker).unwrap();
        assert!(
            world
                .callable_facts(c.body)
                .unwrap()
                .validate_ty(&marker)
                .is_err(),
            "foreign hidden parameter was erased"
        );
        let noncopy = Ty::PhantomData(
            ir::PhantomDataTy::new(&world.catalog, c.leaf.clone(), &c.budget).unwrap(),
        );
        facts.validate_ty(&noncopy).unwrap();
        for ty in [&marker, &noncopy] {
            for item in [CatalogItem::Copy, CatalogItem::Clone, CatalogItem::Sized] {
                let goal = Predicate::Implements(TraitRef {
                    declaration: world.catalog.id(item),
                    self_ty: ty.clone(),
                    args: Arc::from([]),
                });
                assert!(matches!(
                    world.prove_goal(&goal, &facts).unwrap(),
                    ProofOutcome::Proved(_)
                ));
            }
        }
        let changed =
            Ty::PhantomData(ir::PhantomDataTy::new(&world.catalog, Ty::Bool, &c.budget).unwrap());
        assert!(
            !world
                .types_equal(
                    std::slice::from_ref(&marker),
                    std::slice::from_ref(&changed)
                )
                .unwrap()
        );
        for unsupported in [
            Ty::Str,
            Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(Ty::Bool),
            },
        ] {
            let message =
                ir::PhantomDataTy::new(&world.catalog, unsupported, &c.budget).unwrap_err();
            assert_eq!(
                FactsError::from_message(&c.budget, &message).kind(),
                FactsErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn function_pointer_metadata_preserves_signature_and_rejects_forged_components() {
        let allocation = ir::PlanBudget::default();
        let signature = ir::FnPointerTy::new(Vec::new(), Ty::Unit, &allocation).unwrap();
        let expected = std::mem::size_of::<ir::FnPointerTy>() + 4 * std::mem::size_of::<usize>();
        assert_eq!(allocation.usage_for_tests(|usage| usage.bytes), expected);
        let before = allocation.usage_for_tests(|usage| usage.bytes);
        signature
            .try_map(&allocation, |e| e, |ty| Ok::<_, String>(ty.clone()))
            .unwrap();
        assert_eq!(
            allocation.usage_for_tests(|usage| usage.bytes) - before,
            expected
        );
        assert!(
            ir::FnPointerTy::storage::<Ty>(usize::MAX, &allocation)
                .unwrap_err()
                .contains("overflow")
        );
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let pointer = |inputs: Vec<Ty>, output| {
            Ty::FnPointer(Arc::new(ir::FnPointerTy {
                inputs: inputs.into(),
                output,
            }))
        };
        let valid = pointer(
            vec![c.caller_parameter.clone(), Ty::Bool],
            c.caller_parameter.clone(),
        );
        facts.validate_ty(&valid).unwrap();
        let equal = pointer(
            vec![c.caller_parameter.clone(), Ty::Bool],
            c.caller_parameter.clone(),
        );
        assert!(
            world
                .types_equal(std::slice::from_ref(&valid), std::slice::from_ref(&equal))
                .unwrap()
        );
        for changed in [
            pointer(
                vec![Ty::Bool, c.caller_parameter.clone()],
                c.caller_parameter.clone(),
            ),
            pointer(vec![c.caller_parameter.clone()], c.caller_parameter.clone()),
            pointer(vec![c.caller_parameter.clone(), Ty::Bool], Ty::Bool),
        ] {
            assert!(
                !world
                    .types_equal(std::slice::from_ref(&valid), std::slice::from_ref(&changed))
                    .unwrap()
            );
        }
        let closed = world.callable_facts(c.body).unwrap();
        assert!(
            closed.validate_ty(&valid).is_err(),
            "foreign signature parameter accepted"
        );
        let Ty::Struct(leaf) = &c.leaf else {
            panic!("nominal fixture");
        };
        let mut altered = leaf.def().clone();
        altered.type_args = Arc::from([Ty::Bool]);
        let forged = pointer(vec![Ty::Struct(StructTy::new(altered))], Ty::Unit);
        assert!(
            facts.validate_ty(&forged).is_err(),
            "forged nominal signature component accepted"
        );
        for invalid in [
            pointer(vec![Ty::Bool; 33], Ty::Unit),
            pointer(vec![Ty::Str], Ty::Unit),
            pointer(vec![Ty::Bool], Ty::Str),
        ] {
            assert!(facts.validate_ty(&invalid).is_err());
        }
        let borrowed = pointer(
            vec![Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(Ty::Bool),
            }],
            Ty::Unit,
        );
        let error = facts.validate_ty(&borrowed).unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::Unsupported);
        let nested = pointer(vec![borrowed], Ty::Unit);
        assert_eq!(
            facts.validate_ty(&nested).unwrap_err().kind(),
            FactsErrorKind::Unsupported
        );
        for item in [CatalogItem::Copy, CatalogItem::Sized] {
            let goal = Predicate::Implements(TraitRef {
                declaration: world.catalog.id(item),
                self_ty: valid.clone(),
                args: Arc::from([]),
            });
            assert!(matches!(
                world.prove_goal(&goal, &facts).unwrap(),
                ProofOutcome::Proved(_)
            ));
        }
    }

    #[test]
    fn scratch_row_real_owners_refund_only_row_buffers() {
        let c = case();
        let world = c.build().unwrap();
        let ty = Ty::Tuple(Arc::from([Ty::Unit]));
        let Ty::Tuple(input) = &ty else {
            unreachable!()
        };
        let input_refs = Arc::strong_count(input);
        let mut raw = body_validation::RawFormations::default();
        raw.check(&world, &ty, world.empty.env(), TypeUse::Root, None, 0)
            .unwrap();
        assert_eq!(raw.retained_roots(), 1);
        let paid = raw.storage_bytes();
        let before_hit = c.budget.usage_snapshot().bytes;
        raw.check(&world, &ty, world.empty.env(), TypeUse::Root, None, 0)
            .unwrap();
        assert_eq!(raw.retained_roots(), 1);
        assert_eq!(c.budget.usage_snapshot().bytes, before_hit);
        assert!(Arc::strong_count(input) > input_refs);
        drop(raw);
        assert_eq!(c.budget.usage_snapshot().bytes, before_hit - paid);
        assert_eq!(Arc::strong_count(input), input_refs);

        let mut scratch = ProofScratch::default();
        world
            .closedness_of_root(&ty, world.empty.env(), TypeUse::Root, 0, &mut scratch)
            .unwrap();
        assert_eq!(scratch.closedness.len(), 1);
        let paid = scratch_row_bytes::<ClosednessValidation>().unwrap();
        let before_hit = c.budget.usage_snapshot().bytes;
        world
            .closedness_of_root(&ty, world.empty.env(), TypeUse::Root, 0, &mut scratch)
            .unwrap();
        assert_eq!(scratch.closedness.len(), 1);
        assert_eq!(c.budget.usage_snapshot().bytes, before_hit);
        assert!(Arc::strong_count(input) > input_refs);
        drop(scratch);
        assert_eq!(c.budget.usage_snapshot().bytes, before_hit - paid);
        assert_eq!(Arc::strong_count(input), input_refs);
        // The original immutable type outlives both memos and remains valid.
        world
            .validate_ty_inner(
                &ty,
                world.empty.env(),
                0,
                TypeUse::Root,
                &mut TypeWalk::default(),
            )
            .unwrap();
    }

    #[test]
    fn closedness_backedge_never_authorizes_a_strong_nominal_root() {
        let c = case();
        let world = c.build().unwrap();
        let Ty::Struct(original) = &c.leaf else {
            panic!("nominal fixture")
        };
        let mut definition = original.def().clone();
        definition.module = fixture_modules().core_root();
        let definition = Arc::new(definition);
        let back = Ty::Struct(StructTy::from_ref(ir::NominalRef::Back(
            Arc::downgrade(&definition),
            definition.id,
        )));
        let strong = Ty::Struct(StructTy::from_ref(ir::NominalRef::Strong(definition)));
        assert!(!same_closedness_input(&back, &strong));
        assert!(!same_closedness_input(&strong, &back));
        let mut scratch = ProofScratch::default();
        world
            .closedness_of_root(&back, world.empty.env(), TypeUse::Root, 0, &mut scratch)
            .unwrap();
        assert!(
            scratch.closedness.is_empty(),
            "a backedge has no standalone validation proof"
        );
        assert!(
            world
                .closedness_of_root(&strong, world.empty.env(), TypeUse::Root, 0, &mut scratch)
                .is_err()
        );
        assert!(scratch.closedness.is_empty());
    }

    #[test]
    fn nominal_public_and_field_schema_rejects_foreign_or_sibling_module_authority() {
        let c = case();
        let Ty::Struct(original) = &c.leaf else {
            panic!("expected nominal fixture")
        };
        let foreign = fixture_modules();
        let mut definition = original.def().clone();
        definition.module = foreign.core_root();
        definition.visibility = ir::Visibility::Public;
        let forged = Ty::Struct(StructTy::new(definition));
        assert!(
            matches!(validate_nominal_modules(&forged, &c.modules, &c.budget), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        assert!(
            matches!(test_support::facts_for_signature(c.modules.clone(), &sig(Vec::new(), Ty::Unit), &[forged], &c.budget), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );

        let mut definition = original.def().clone();
        definition.module = c
            .modules
            .catalog_module(super::super::module_identity::CatalogModule::Clone);
        definition.visibility = ir::Visibility::Public;
        definition.fields = Arc::<[FieldDef]>::from([FieldDef {
            name: "field".into(),
            ty: Ty::Unit,
            visibility: ir::Visibility::Restricted(
                c.modules
                    .catalog_module(super::super::module_identity::CatalogModule::Ops),
            ),
            lifetime_bound: false,
        }])
        .into();
        let forged = Ty::Struct(StructTy::new(definition));
        assert!(
            matches!(test_support::facts_for_signature(c.modules.clone(), &sig(Vec::new(), Ty::Unit), &[forged], &c.budget), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );

        let mut definition = original.def().clone();
        definition.visibility = ir::Visibility::Public;
        definition.fields = Arc::<[FieldDef]>::from([FieldDef {
            name: "field".into(),
            ty: Ty::Unit,
            visibility: ir::Visibility::Public,
            lifetime_bound: false,
        }])
        .into();
        let valid = Ty::Struct(StructTy::new(definition));
        validate_nominal_modules(&valid, &c.modules, &c.budget).unwrap();
    }
    impl Case {
        fn build(&self) -> Result<Arc<ValidatedTraitWorld>> {
            let adapter = test_support::adapter(
                self.modules.clone(),
                &self.headers,
                &self.types,
                &self.budget,
            )?;
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

    fn shared_slice(element: Ty) -> Ty {
        reference(Ty::Slice {
            element: Arc::new(element),
        })
    }

    #[test]
    fn typed_slice_facts_admit_exact_elements_without_requiring_copy() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        for element in [
            Ty::Int(IntTy::I128),
            Ty::Tuple(Arc::from([Ty::Bool, Ty::Int(IntTy::I32)])),
            c.leaf.clone(),
            c.caller_parameter.clone(),
        ] {
            facts.validate_ty(&shared_slice(element)).unwrap();
        }
        facts
            .validate_ty(&shared_slice(reference(Ty::Bool)))
            .unwrap();
        let foreign = case();
        assert!(
            matches!(facts.validate_ty(&shared_slice(foreign.leaf)), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
    }

    #[test]
    fn generic_shared_slice_retains_original_descriptor_and_element_family() {
        let budget = PlanBudget::default();
        let program =
            crate::compiler::frontend::parse("fn size<T>(s:&[T])->usize{s.len()}fn main(){}")
                .unwrap();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let facts = world.callable_facts(declaration).unwrap();
        let parameter = Ty::Param(facts.env().ordered_params().next().unwrap());
        facts.validate_ty(&shared_slice(parameter.clone())).unwrap();
        for element in [
            Ty::Int(IntTy::I32),
            Ty::Int(IntTy::U128),
            Ty::Tuple(Arc::from([Ty::Int(IntTy::I32), Ty::Bool])),
        ] {
            let target = CallTarget {
                declaration,
                type_args: Arc::from([element]),
            };
            let closed = world.instance_facts(&target).unwrap();
            let carrier = closed.carrier_contract(SignatureSubject::Body).unwrap();
            assert_eq!(carrier.original_contract().inputs.len(), 2);
            assert_eq!(carrier.contract().inputs.len(), 1);
            assert!(matches!(
                carrier.contract().inputs[0].location.channel,
                ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 })
            ));
            assert!(carrier.component_mappings().is_empty());
        }
        let referenced = world
            .instance_facts(&CallTarget {
                declaration,
                type_args: Arc::from([Ty::Ref {
                    kind: BorrowKind::Shared,
                    referent: Arc::new(Ty::Int(IntTy::I32)),
                }]),
            })
            .unwrap();
        let carrier = referenced.carrier_contract(SignatureSubject::Body).unwrap();
        assert_eq!(carrier.contract().inputs.len(), 2);
        assert_eq!(carrier.component_mappings().len(), 1);
        let locations = ir::carrier_locations(&shared_slice(parameter), &budget).unwrap();
        assert_eq!(locations.len(), 2);
        assert!(locations[0].path.0.is_empty());
        assert!(matches!(
            locations[0].channel,
            ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 })
        ));
        assert!(matches!(
            world.collect_nominal_mappings(
                &shared_slice(Ty::Bool),
                &shared_slice(Ty::Int(IntTy::I32)),
                &mut Vec::new(),
                0,
            ),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
    }

    fn generic_marker_case() -> Case {
        let mut c = case();
        let declaration = c.headers.traits[0].declaration;
        let parameters = scope(declaration, 2, &c.budget);
        Arc::make_mut(&mut c.headers.traits)[0].env =
            env(std::slice::from_ref(&parameters), &c.budget);
        let implementations = Arc::make_mut(&mut c.headers.impls);
        implementations[1].trait_ref.args = Arc::from([Ty::Bool]);
        let Predicate::Implements(requirement) =
            &mut Arc::make_mut(&mut implementations[0].predicates)[0]
        else {
            panic!("membership fixture")
        };
        requirement.args = Arc::from([Ty::Bool]);
        c
    }

    #[test]
    fn generic_trait_coherence_and_selection_use_all_arguments() {
        let mut c = generic_marker_case();
        let mut alternate = c.headers.impls[1].clone();
        alternate.declaration =
            test_support::source_decl(c.value_trait.snapshot, 21, &c.budget).unwrap();
        alternate.trait_ref.args = Arc::from([Ty::Int(IntTy::I32)]);
        let mut implementations = c.headers.impls.to_vec();
        implementations.push(alternate);
        c.headers.impls = implementations.into();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        for argument in [Ty::Bool, Ty::Int(IntTy::I32)] {
            let goal = Predicate::Implements(TraitRef {
                declaration: c.headers.traits[0].declaration,
                self_ty: c.leaf.clone(),
                args: Arc::from([argument]),
            });
            assert!(matches!(
                world.prove_goal(&goal, &facts).unwrap(),
                ProofOutcome::Proved(_)
            ));
        }
        let mut duplicate = c.headers.impls[2].clone();
        duplicate.declaration =
            test_support::source_decl(c.value_trait.snapshot, 22, &c.budget).unwrap();
        let mut implementations = c.headers.impls.to_vec();
        implementations.push(duplicate);
        c.headers.impls = implementations.into();
        assert!(matches!(c.build(), Err(error) if error.kind() == FactsErrorKind::Ambiguous));
    }

    #[test]
    fn generic_impl_parameter_constrained_only_by_trait_argument_is_matched() {
        let mut c = generic_marker_case();
        let declaration = c.headers.impls[1].declaration;
        let parameters = scope(declaration, 1, &c.budget);
        let parameter = Ty::Param(ParamId {
            owner: parameters.binder(),
            index: 0,
        });
        let implementation = &mut Arc::make_mut(&mut c.headers.impls)[1];
        implementation.schema_env = env(&[parameters], &c.budget);
        implementation.trait_ref.args = Arc::from([parameter]);
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let goal = Predicate::Implements(TraitRef {
            declaration: c.headers.traits[0].declaration,
            self_ty: c.leaf.clone(),
            args: Arc::from([Ty::Int(IntTy::I32)]),
        });
        assert!(matches!(
            world.prove_goal(&goal, &facts).unwrap(),
            ProofOutcome::Proved(_)
        ));
    }

    #[test]
    fn generic_trait_actuals_validate_before_excess_arity() {
        let c = generic_marker_case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let goal = Predicate::Implements(TraitRef {
            declaration: c.headers.traits[0].declaration,
            self_ty: c.leaf.clone(),
            args: Arc::from([Ty::Bool, reference(Ty::Bool)]),
        });
        let error = world.prove_goal(&goal, &facts).unwrap_err();
        assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
        assert!(error.to_string().contains("trait argument arity mismatch"));
    }

    #[test]
    fn generic_trait_reference_nominal_argument_does_not_invent_membership() {
        let mut c = generic_marker_case();
        let declaration = test_support::source_decl(c.value_trait.snapshot, 23, &c.budget).unwrap();
        let Ty::Struct(value) = nominal(
            c.modules.core_root(),
            c.value_trait.snapshot,
            3,
            declaration,
            Vec::new(),
        ) else {
            unreachable!()
        };
        let mut definition = value.def().clone();
        definition.fields = Arc::<[FieldDef]>::from([FieldDef {
            name: "borrow".into(),
            ty: reference(Ty::Bool),
            visibility: ir::Visibility::Public,
            lifetime_bound: false,
        }])
        .into();
        let argument = Ty::Struct(StructTy::new(definition));
        c.types.push(argument.clone());
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let goal = Predicate::Implements(TraitRef {
            declaration: c.headers.traits[0].declaration,
            self_ty: c.leaf.clone(),
            args: Arc::from([argument]),
        });
        assert!(matches!(
            world.prove_goal(&goal, &facts).unwrap(),
            ProofOutcome::NotProven
        ));
    }

    fn generic_value_case() -> Case {
        let mut c = case();
        let parameters = scope(c.value_trait, 2, &c.budget);
        let parameter = Ty::Param(ParamId {
            owner: parameters.binder(),
            index: 1,
        });
        Arc::make_mut(&mut c.headers.traits)[1].env =
            env(std::slice::from_ref(&parameters), &c.budget);
        let item = &mut Arc::make_mut(&mut c.headers.items)[0];
        item.schema.env = env(
            &[parameters, item.schema.env.scopes()[1].clone()],
            &c.budget,
        );
        item.schema.signature.result = parameter;
        Arc::make_mut(&mut c.headers.impls)[0].trait_ref.args = Arc::from([Ty::Int(IntTy::I32)]);
        let Predicate::Implements(requirement) =
            &mut Arc::make_mut(&mut Arc::make_mut(&mut c.headers.callables)[1].schema.predicates)
                [0]
        else {
            panic!("membership fixture")
        };
        requirement.args = Arc::from([Ty::Int(IntTy::I32)]);
        c
    }

    #[test]
    fn generic_trait_item_and_default_dispatch_preserve_full_application() {
        for omit_override in [true, false] {
            let mut c = generic_value_case();
            let default = add_source_default(&mut c, omit_override);
            let world = c.build().unwrap();
            let facts = world.callable_facts(c.caller).unwrap();
            let mut call = c.call(c.boxed.clone());
            call.trait_ref.args = Arc::from([Ty::Int(IntTy::I32)]);
            let signature = world.instantiate_trait_signature(&call, &facts).unwrap();
            assert!(
                world
                    .eq_ty(&signature.result, &Ty::Int(IntTy::I32), 0)
                    .unwrap()
            );
            let resolved = world.resolve_trait_call(&call, &facts).unwrap();
            let Callee::Direct(target) = resolved.callee() else {
                panic!("expected closed provider")
            };
            assert_eq!(
                target.declaration,
                if omit_override { default } else { c.body }
            );
            if omit_override {
                assert!(
                    world
                        .eq_types(
                            &target.type_args,
                            &[c.boxed.clone(), Ty::Int(IntTy::I32)],
                            0
                        )
                        .unwrap()
                );
            } else {
                assert!(
                    world
                        .eq_types(&target.type_args, std::slice::from_ref(&c.leaf), 0)
                        .unwrap()
                );
            }
            world.instance_facts(target).unwrap();
        }
    }

    #[test]
    fn implementation_body_accepts_only_exact_owner_trait_application() {
        for exact in [true, false] {
            let mut c = generic_value_case();
            let mut owner = c.headers.impls[0].trait_ref.clone();
            if !exact {
                owner.args = Arc::from([Ty::Bool]);
            }
            Arc::make_mut(&mut c.headers.callables)[0].schema.predicates =
                Arc::from([Predicate::Implements(owner)]);
            if exact {
                let world = c.build().unwrap();
                let facts = world.callable_facts(c.body).unwrap();
                let goal = Predicate::Implements(c.headers.impls[0].trait_ref.clone());
                assert!(matches!(
                    world.prove_goal(&goal, &facts).unwrap(),
                    ProofOutcome::Proved(_)
                ));
            } else {
                assert!(
                    matches!(c.build(), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
                );
            }
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
            test_support::source_decl(SnapshotId::standalone().unwrap(), 0, budget).unwrap();
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
            static_regions: 0,
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
    fn signature_subtree_walk_retains_inputs_and_rechecks_forgery_context_and_poison() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let parts: Arc<[Ty]> = Arc::from([c.leaf.clone()]);
        let retained = Arc::downgrade(&parts);
        let first = Ty::Tuple(Arc::from([Ty::Tuple(parts.clone()), Ty::Unit]));
        let mut roots = body_validation::Validated::default();
        roots.check_signature_root(&facts, &first).unwrap();
        let rows = roots.retained_subtrees();
        assert!(rows >= 3);
        drop(first);
        drop(parts);
        let parts = retained
            .upgrade()
            .expect("completed memo pins exact child allocation");
        let second = Ty::Tuple(Arc::from([Ty::Tuple(parts), Ty::Bool]));
        roots.check_signature_root(&facts, &second).unwrap();
        assert_eq!(roots.retained_subtrees(), rows + 1);
        let Ty::Struct(leaf) = &c.leaf else { panic!() };
        let mut definition = leaf.def().clone();
        definition.fields = Arc::<[FieldDef]>::from([FieldDef {
            name: "forged".into(),
            ty: Ty::Unit,
            visibility: ir::Visibility::Public,
            lifetime_bound: false,
        }])
        .into();
        let forged = Ty::Tuple(Arc::from([Ty::Struct(StructTy::new(definition))]));
        assert!(matches!(roots.check_signature_root(&facts, &forged),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        assert_eq!(roots.retained_subtrees(), 0);
        roots.check_signature_root(&facts, &second).unwrap();
        let foreign = case();
        assert!(matches!(roots.check_signature_root(&facts, &foreign.leaf),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        roots.check_signature_root(&facts, &second).unwrap();
        let other = world.callable_facts(c.body).unwrap();
        roots.check_signature_root(&other, &c.leaf).unwrap();
        assert_eq!(roots.retained_subtrees(), 1);
        let budget = facts.budget();
        assert!(
            budget
                .charge_work(budget.limits().work.saturating_add(1))
                .is_err()
        );
        let failure = budget.first_resource_failure();
        budget.poison_ledger_for_tests();
        assert!(matches!(roots.check_signature_root(&facts, &second),
            Err(error) if error.kind() == FactsErrorKind::Resource));
        assert_eq!(budget.first_resource_failure(), failure);
    }

    #[test]
    fn local_signature_proof_lifetime_rechecks_forgery_context_and_poison() {
        let budget = PlanBudget::default();
        let header = nested_header(&budget);
        let world =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
        let target = CallTarget {
            declaration: header.declaration,
            type_args: Arc::from([Ty::Int(IntTy::I32)]),
        };
        let facts = world.instance_facts(&target).unwrap();
        let mut scratch = TraitScratch::default();
        let signature = world
            .discharge_direct_with(&target, &facts, &mut scratch)
            .unwrap()
            .signature;
        let callee = Callee::Direct(target.clone());
        let receiver = facts.callee_receiver(&callee).unwrap();
        for _ in 0..8 {
            facts
                .validate_signature_with(SignatureSubject::Call(&callee), &signature, &mut scratch)
                .unwrap();
            assert_eq!(
                facts
                    .callee_receiver_scratch(&callee, &mut scratch)
                    .unwrap(),
                receiver
            );
        }
        assert_eq!(scratch.signatures.len(), 1);
        let signature_roots = scratch.signature_types.retained_roots();
        assert!(signature_roots > 0);
        // Bypass the whole-signature memo: every root still receives exact
        // caller authentication and the stored contract is rechecked.
        for _ in 0..3 {
            world
                .validate_signature_uncached(
                    SignatureSubject::Call(&callee),
                    &signature,
                    &facts,
                    &mut scratch,
                )
                .unwrap();
            assert_eq!(scratch.signature_types.retained_roots(), signature_roots);
        }
        assert!(!signature.ref_contract.results.is_empty());
        let mut forged_contract = signature.clone();
        Arc::make_mut(&mut forged_contract.ref_contract).results = Arc::from([]);
        assert!(matches!(
            world.validate_signature_uncached(
                SignatureSubject::Call(&callee), &forged_contract, &facts, &mut scratch,
            ),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
        let mut forged = signature.clone();
        forged.result = Ty::Bool;
        assert!(matches!(
            facts.validate_signature_with(SignatureSubject::Call(&callee), &forged, &mut scratch),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
        assert_eq!(scratch.signatures.len(), 1);
        let changed = world.callable_facts(header.declaration).unwrap();
        assert_eq!(
            changed
                .callee_receiver_scratch(&callee, &mut scratch)
                .unwrap(),
            receiver
        );
        assert_ne!(facts.predicates.index, changed.predicates.index);
        changed
            .validate_signature_with(SignatureSubject::Call(&callee), &signature, &mut scratch)
            .unwrap();
        assert_eq!(scratch.facts_key.unwrap().0, changed.predicates.index);
        assert_eq!(scratch.signatures.len(), 1);
        let other =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
        let other_facts = other.instance_facts(&target).unwrap();
        assert_eq!(
            other_facts
                .callee_receiver_scratch(&callee, &mut scratch)
                .unwrap(),
            receiver
        );
        other_facts
            .validate_signature_with(SignatureSubject::Call(&callee), &signature, &mut scratch)
            .unwrap();
        assert_eq!(scratch.world, Some(other.id));
        assert_eq!(scratch.signatures.len(), 1);
        facts
            .validate_signature_with(SignatureSubject::Call(&callee), &signature, &mut scratch)
            .unwrap();
        assert_eq!(scratch.world, Some(world.id));
        let mut raw_proofs = ProofScratch::default();
        world
            .discharge_in_with_facts(
                &target,
                ProofContext::from_facts(&facts),
                &mut raw_proofs,
                Some(&facts),
            )
            .unwrap();
        assert!(raw_proofs.memo.is_empty());
        let retained = raw_proofs.checked.as_deref().unwrap() as *const ProofScratch;
        let retained_context = raw_proofs.checked.as_ref().unwrap().checked_context;
        assert!(Arc::ptr_eq(
            raw_proofs
                .checked
                .as_ref()
                .unwrap()
                ._checked_env
                .as_ref()
                .unwrap(),
            facts.env()
        ));
        for _ in 0..3 {
            world
                .discharge_in(&target, ProofContext::from_facts(&facts), &mut raw_proofs)
                .unwrap();
            world
                .discharge_in_with_facts(
                    &target,
                    ProofContext::from_facts(&facts),
                    &mut raw_proofs,
                    Some(&facts),
                )
                .unwrap();
            let arena = raw_proofs.checked.as_deref_mut().unwrap();
            assert_eq!(arena as *const ProofScratch, retained);
            let memo_rows = arena.memo.len();
            arena
                .with_caller_arena(world.id, None, |nested| {
                    assert_ne!(nested as *const ProofScratch, retained);
                    Ok(())
                })
                .unwrap();
            arena
                .with_caller_arena(world.id, Some(&changed), |nested| {
                    assert_ne!(nested as *const ProofScratch, retained);
                    Ok(())
                })
                .unwrap();
            assert_eq!(arena.checked_context, retained_context);
            assert_eq!(arena.memo.len(), memo_rows);
        }
        assert!(matches!(world.discharge_in_with_facts(
            &target, ProofContext::from_facts(&changed), &mut raw_proofs, Some(&facts),
        ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        assert!(matches!(other.discharge_in_with_facts(
            &target, ProofContext::from_facts(&facts), &mut raw_proofs, Some(&facts),
        ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        assert!(facts.budget().charge_work(usize::MAX).is_err());
        let failure = facts.budget().first_resource_failure();
        assert!(matches!(world.discharge_in_with_facts(
            &target, ProofContext::from_facts(&facts), &mut raw_proofs, Some(&facts),
        ), Err(error) if error.kind() == FactsErrorKind::Resource));
        facts.budget().poison_ledger_for_tests();
        assert!(matches!(world.discharge_in_with_facts(
            &target, ProofContext::from_facts(&facts), &mut raw_proofs, Some(&facts),
        ), Err(error) if error.kind() == FactsErrorKind::Resource));
        assert!(matches!(
            facts.callee_receiver_scratch(&callee, &mut scratch),
            Err(error) if error.kind() == FactsErrorKind::Resource
        ));
        assert!(matches!(
            facts.validate_signature_with(SignatureSubject::Call(&callee), &signature, &mut scratch),
            Err(error) if error.kind() == FactsErrorKind::Resource
        ));
        assert_eq!(facts.budget().first_resource_failure(), failure);
        assert!(matches!(
            scratch.signature_types.check_signature_root(&facts, &signature.result),
            Err(error) if error.kind() == FactsErrorKind::Resource
        ));
        assert_eq!(facts.budget().first_resource_failure(), failure);
    }

    #[test]
    fn proof_arena_boxes_release_only_their_own_scoped_storage() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let before = facts.budget().usage_snapshot().bytes;
        {
            let mut scratch = ProofScratch::default();
            scratch
                .with_caller_arena(world.id, Some(&facts), |_| Ok(()))
                .unwrap();
            let live = facts.budget().usage_snapshot().bytes;
            assert_eq!(
                live - before,
                std::mem::size_of::<ProofScratch>() + 2 * std::mem::size_of::<usize>()
            );
            let address = std::ptr::from_ref(scratch.checked.as_deref().unwrap());
            scratch
                .with_caller_arena(world.id, Some(&facts), |arena| {
                    assert_eq!(std::ptr::from_ref(arena), address);
                    Ok(())
                })
                .unwrap();
            assert_eq!(facts.budget().usage_snapshot().bytes, live);
        }
        assert_eq!(facts.budget().usage_snapshot().bytes, before);
        {
            let mut replays = NominalReplays::default();
            assert!(
                replays
                    .raw_metadata_child(&world, facts.env())
                    .unwrap()
                    .is_some()
            );
            let live = facts.budget().usage_snapshot().bytes;
            assert_eq!(
                live - before,
                std::mem::size_of::<NominalReplays>() + 2 * std::mem::size_of::<usize>()
            );
            assert!(
                replays
                    .raw_metadata_child(&world, facts.env())
                    .unwrap()
                    .is_some()
            );
            assert_eq!(facts.budget().usage_snapshot().bytes, live);
        }
        assert_eq!(facts.budget().usage_snapshot().bytes, before);
        assert!(facts.budget().first_resource_failure().is_none());
    }

    #[test]
    fn proof_arena_box_drop_never_erases_poison_or_first_failure() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let mut scratch = ProofScratch::default();
        scratch
            .with_caller_arena(world.id, Some(&facts), |_| Ok(()))
            .unwrap();
        let mut replays = NominalReplays::default();
        assert!(
            replays
                .raw_metadata_child(&world, facts.env())
                .unwrap()
                .is_some()
        );
        let admitted = facts.budget().usage_snapshot().bytes;
        assert!(facts.budget().charge_work(usize::MAX).is_err());
        let first_failure = facts.budget().first_resource_failure();
        assert!(first_failure.is_some());
        facts.budget().poison_ledger_for_tests();
        drop(scratch);
        drop(replays);
        assert_eq!(facts.budget().usage_snapshot().bytes, admitted);
        assert_eq!(facts.budget().first_resource_failure(), first_failure);
        assert!(facts.budget().usage_snapshot().poisoned);
        let mut fresh = ProofScratch::default();
        assert!(matches!(
            fresh.with_caller_arena(world.id, Some(&facts), |_| Ok(())),
            Err(error) if error.kind() == FactsErrorKind::Resource
        ));
        assert_eq!(facts.budget().first_resource_failure(), first_failure);
    }

    #[test]
    fn nested_reference_world_positioned_schema_and_canonical_transport() {
        let budget = PlanBudget::default();
        let header = nested_header(&budget);
        let declaration = header.declaration;
        let world =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
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
            test_support::source_decl(SnapshotId::standalone().unwrap(), 0, &budget).unwrap();
        let empty = env(&[], &budget);
        let raw = CallableHeader {
            declaration,
            schema: GenericSchema {
                env: empty,
                signature: sig(vec![reference(Ty::Int(IntTy::I32))], Ty::Unit),
                predicates: Arc::from([]),
            },
        };
        assert!(test_support::world(fixture_modules(), &[raw], &budget).is_err());
    }

    #[test]
    fn nested_reference_world_per_position_refinement_and_type_use_context() {
        let budget = PlanBudget::default();
        let header = nested_header(&budget);
        let world =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
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
        world.validate_actual(&nested, facts.env()).unwrap();
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
        world
            .validate_executable_actual(&empty_product, facts.env())
            .unwrap();
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
            test_support::source_decl(SnapshotId::standalone().unwrap(), 0, &budget).unwrap();
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
            fixture_modules(),
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
        let aggregate = world
            .instance_facts(&CallTarget {
                declaration,
                type_args: Arc::from([
                    Ty::Tuple(Arc::from([reference(Ty::Int(IntTy::I32))])),
                    Ty::Int(IntTy::I32),
                ]),
            })
            .unwrap();
        let aggregate_view = aggregate.carrier_contract(SignatureSubject::Body).unwrap();
        assert!(aggregate_view.contract().inputs.iter().any(|input| {
            !input.location.path.0.is_empty()
                && matches!(input.location.channel, ir::CarrierChannel::Fixed(_))
        }));
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
        world.validate_actual(&root, universal.env()).unwrap();
        assert!(universal.must_copy(&root).unwrap());
    }

    fn add_source_default(c: &mut Case, omit_override: bool) -> DeclId {
        let default = test_support::source_decl(c.item.snapshot, 20, &c.budget).unwrap();
        let sized = test_support::adapter(c.modules.clone(), &c.headers, &c.types, &c.budget)
            .unwrap()
            .core_catalog()
            .id(CatalogItem::Sized);
        let item = &mut Arc::make_mut(&mut c.headers.items)[0];
        item.default = Some(default);
        if item.receiver == ReceiverMode::Value {
            let mut predicates = item.schema.predicates.to_vec();
            predicates.push(Predicate::Implements(TraitRef {
                declaration: sized,
                self_ty: item.schema.signature.params[0].clone(),
                args: Arc::from([]),
            }));
            item.schema.predicates = predicates.into();
        }
        let mut schema = item.schema.clone();
        let trait_scope = schema.env.scopes()[0].clone();
        schema.env = env(&[trait_scope, scope(default, 0, &c.budget)], &c.budget);
        let mut callables = c.headers.callables.to_vec();
        callables.push(CallableHeader {
            declaration: default,
            schema,
        });
        c.headers.callables = callables.into();
        if omit_override {
            Arc::make_mut(&mut c.headers.impls)[0].items = Arc::from([]);
        }
        default
    }

    #[test]
    fn source_default_provider_dispatch_substitutes_self_and_override_wins() {
        for omit_override in [true, false] {
            let mut c = case();
            let default = add_source_default(&mut c, omit_override);
            let world = c.build().unwrap();
            let facts = world.callable_facts(c.caller).unwrap();
            let resolved = world
                .resolve_trait_call(&c.call(c.boxed.clone()), &facts)
                .unwrap();
            let Callee::Direct(target) = resolved.callee() else {
                panic!("expected direct provider")
            };
            assert_eq!(
                target.declaration,
                if omit_override { default } else { c.body }
            );
            let expected = if omit_override { &c.boxed } else { &c.leaf };
            assert!(
                world
                    .eq_types(&target.type_args, std::slice::from_ref(expected), 0)
                    .unwrap()
            );
            world.instance_facts(target).unwrap();
            let universal = world.callable_facts(default).unwrap();
            let self_ty = Ty::Param(universal.env().ordered_params().next().unwrap());
            assert!(matches!(
                world
                    .prove_goal(&c.goal(self_ty.clone()), &universal)
                    .unwrap(),
                ProofOutcome::Proved(_)
            ));
            let deferred = world
                .resolve_trait_call(&c.call(self_ty), &universal)
                .unwrap();
            assert!(matches!(deferred.callee(), Callee::TraitItem(_)));
            assert!(
                world
                    .schema_facts(c.item)
                    .unwrap()
                    .must_copy(&c.leaf)
                    .is_err()
            );
            assert!(world.callable_facts(c.item).is_err());
            // Default entry still needs a real implementation, not the
            // universal provider's assumed Self:Trait predicate.
            assert!(
                world
                    .instance_facts(&CallTarget {
                        declaration: default,
                        type_args: Arc::from([c.leaf.clone()]),
                    })
                    .is_err()
            );
        }
    }

    #[test]
    fn source_default_borrowed_provider_preserves_contract_and_self_formation() {
        let mut c = borrowed_case();
        let default = add_source_default(&mut c, true);
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let resolved = world
            .resolve_trait_call(&c.call(c.boxed.clone()), &facts)
            .unwrap();
        let Callee::Direct(target) = resolved.callee() else {
            panic!("expected borrowed default")
        };
        assert_eq!(target.declaration, default);
        facts
            .validate_signature(
                SignatureSubject::Call(resolved.callee()),
                resolved.signature(),
            )
            .unwrap();
        assert_eq!(
            facts.callee_receiver(resolved.callee()).unwrap(),
            ReceiverMode::Borrowed {
                kind: BorrowKind::Shared
            }
        );
        let universal = world.callable_facts(default).unwrap();
        let self_ty = Ty::Param(universal.env().ordered_params().next().unwrap());
        universal.validate_ty(&reference(self_ty.clone())).unwrap();
        assert!(
            universal
                .validate_ty(&Ty::Array {
                    element: Arc::new(self_ty),
                    len: 1
                })
                .is_err()
        );
    }

    #[test]
    fn source_default_owned_self_requires_declared_sized_even_when_overridden() {
        for omit_override in [true, false] {
            let mut c = case();
            add_source_default(&mut c, omit_override);
            // Required schemas can describe unsized by-value Self, but the
            // default's executable body must be checked regardless of use.
            Arc::make_mut(&mut c.headers.items)[0].schema.predicates = Arc::from([]);
            Arc::make_mut(&mut c.headers.callables)
                .last_mut()
                .unwrap()
                .schema
                .predicates = Arc::from([]);
            assert!(matches!(c.build(), Err(error) if error.kind() == FactsErrorKind::Unproved));
            let sized = test_support::adapter(c.modules.clone(), &c.headers, &c.types, &c.budget)
                .unwrap()
                .core_catalog()
                .id(CatalogItem::Sized);
            let self_ty = c.headers.items[0].schema.signature.params[0].clone();
            Arc::make_mut(&mut c.headers.traits)[1].supertraits = Arc::from([TraitRef {
                declaration: sized,
                self_ty,
                args: Arc::from([]),
            }]);
            c.build().unwrap();
        }
        let mut c = borrowed_case();
        let default = add_source_default(&mut c, true);
        let world = c.build().unwrap();
        let facts = world.callable_facts(default).unwrap();
        let self_ty = Ty::Param(facts.env().ordered_params().next().unwrap());
        assert!(facts.require_sized(&self_ty).is_err());
        facts.require_sized(&reference(self_ty.clone())).unwrap();
        assert!(
            world
                .schema_facts(c.item)
                .unwrap()
                .require_sized(&c.leaf)
                .is_err()
        );
        assert!(matches!(
            world
                .prove_goal(
                    &Predicate::Implements(TraitRef {
                        declaration: world.catalog.id(CatalogItem::Sized),
                        self_ty,
                        args: Arc::from([]),
                    }),
                    &facts
                )
                .unwrap(),
            ProofOutcome::NotProven
        ));
    }

    #[test]
    fn source_default_provider_requires_registered_callable() {
        let mut c = case();
        add_source_default(&mut c, true);
        c.headers.callables = c.headers.callables[..c.headers.callables.len() - 1].into();
        assert!(matches!(c.build(), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
    }

    #[test]
    fn source_default_provider_rejects_changed_signature_or_obligations() {
        let mut c = case();
        add_source_default(&mut c, true);
        Arc::make_mut(&mut c.headers.callables)
            .last_mut()
            .unwrap()
            .schema
            .signature
            .result = Ty::Bool;
        assert!(matches!(c.build(), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        let mut c = case();
        add_source_default(&mut c, true);
        let self_ty = c.headers.items[0].schema.signature.params[0].clone();
        let marker = c.headers.traits[0].declaration;
        Arc::make_mut(&mut c.headers.callables)
            .last_mut()
            .unwrap()
            .schema
            .predicates = Arc::from([Predicate::Implements(TraitRef {
            declaration: marker,
            self_ty,
            args: Arc::from([]),
        })]);
        assert!(matches!(c.build(), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
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
    fn erased_generic_result_refinement_keeps_input_and_origin_authority() {
        let c = case();
        let world = c.build().unwrap();
        let input = ir::InputReference {
            param: 0,
            position: ir::ReferencePosition { depth: 0 },
        };
        let required = RefContract {
            region_count: 2,
            input_regions: Arc::from([ir::InputRegion {
                input,
                region: ir::RegionParamId(0),
            }]),
            results: Arc::from([ir::ResultReference {
                position: ir::ReferencePosition { depth: 0 },
                region: ir::RegionParamId(1),
                returns: ir::ReturnContract::SharedInputEnvelopes(Arc::from([])),
            }]),
            outlives: Arc::from([]),
            static_regions: 0,
        };
        let mut provided = required.clone();
        provided.region_count = 1;
        Arc::make_mut(&mut provided.results)[0].region = ir::RegionParamId(0);
        Arc::make_mut(&mut provided.results)[0].returns =
            ir::ReturnContract::SharedInputEnvelopes(Arc::from([ir::InputEnvelope { input }]));
        assert!(
            world
                .erased_result_regions_refine(&required, &provided)
                .unwrap()
        );
        for mutation in 0..4 {
            let mut forged = provided.clone();
            match mutation {
                0 => Arc::make_mut(&mut forged.input_regions)[0].input.param = 1,
                1 => Arc::make_mut(&mut forged.results)[0].position.depth = 1,
                2 => forged.static_regions = 1,
                _ => {
                    Arc::make_mut(&mut forged.results)[0].returns =
                        ir::ReturnContract::MutInputEnvelopes(Arc::from([]));
                }
            }
            assert!(
                !world
                    .erased_result_regions_refine(&required, &forged)
                    .unwrap(),
                "mutation {mutation}"
            );
        }
        let mut tied = required.clone();
        Arc::make_mut(&mut tied.results)[0].region = ir::RegionParamId(0);
        assert!(
            !world
                .erased_result_regions_refine(&tied, &provided)
                .unwrap()
        );
        let mut claimed = required.clone();
        Arc::make_mut(&mut claimed.results)[0].returns = provided.results[0].returns.clone();
        assert!(
            !world
                .erased_result_regions_refine(&claimed, &provided)
                .unwrap()
        );
    }

    #[test]
    fn reference_free_impl_input_regions_require_constraint_implication() {
        let refine = |world: &ValidatedTraitWorld, required: &Signature, provided: &Signature| {
            world.reference_free_implementation_refines(required, provided, &world.empty.env, None)
        };
        let c = case();
        let world = c.build().unwrap();
        let contract = |regions: [u16; 4], edges: &[(u16, u16)]| {
            let raw = RefContract {
                static_regions: 0,
                region_count: regions.iter().copied().max().unwrap() + 1,
                input_regions: regions
                    .into_iter()
                    .enumerate()
                    .map(|(i, region)| ir::InputRegion {
                        input: ir::InputReference {
                            param: i / 2,
                            position: ir::ReferencePosition {
                                depth: (i % 2) as u8,
                            },
                        },
                        region: ir::RegionParamId(region),
                    })
                    .collect::<Vec<_>>()
                    .into(),
                results: Arc::from([]),
                outlives: edges
                    .iter()
                    .map(|(a, b)| (ir::RegionParamId(*a), ir::RegionParamId(*b)))
                    .collect::<Vec<_>>()
                    .into(),
            };
            ir::normalize_ref_contract(&raw, &c.budget).unwrap()
        };
        let signature = |contract| Signature {
            params: vec![reference(reference(Ty::Int(IntTy::I32))); 2],
            result: Ty::Bool,
            ref_contract: contract,
        };
        let required = signature(contract([0, 1, 2, 1], &[(1, 0), (1, 2)]));
        let provided = signature(contract([0, 1, 2, 3], &[(1, 0), (3, 2)]));
        assert!(
            !world
                .return_refines(&required.ref_contract, &provided.ref_contract)
                .unwrap()
        );
        assert!(refine(&world, &required, &provided).unwrap());
        assert!(!refine(&world, &provided, &required).unwrap());
        let stronger = signature(contract([0, 1, 2, 3], &[(1, 0), (3, 2), (0, 2)]));
        assert!(!refine(&world, &required, &stronger).unwrap());
        let mut forged = provided.clone();
        Arc::make_mut(&mut Arc::make_mut(&mut forged.ref_contract).input_regions)[3]
            .input
            .param = 0;
        assert!(!refine(&world, &required, &forged).unwrap());
        let mut returns_reference = provided.clone();
        returns_reference.result = reference(Ty::Int(IntTy::I32));
        assert!(!refine(&world, &required, &returns_reference).unwrap());
        let mut hidden_reference = provided.clone();
        hidden_reference.result = Ty::Tuple(Arc::from([reference(Ty::Int(IntTy::I32))]));
        assert!(!refine(&world, &required, &hidden_reference).unwrap());
        let mut required_reference = required.clone();
        required_reference.result = hidden_reference.result.clone();
        assert!(!refine(&world, &required_reference, &provided).unwrap());
        let mut opaque_required = required.clone();
        opaque_required.result = c.caller_parameter.clone();
        assert!(
            !world
                .reference_free_implementation_refines(
                    &opaque_required,
                    &provided,
                    &world.callable_header(c.caller).unwrap().schema.env,
                    None
                )
                .unwrap()
        );
        let mut opaque_provided = provided.clone();
        opaque_provided.result = c.caller_parameter.clone();
        assert!(
            !world
                .reference_free_implementation_refines(
                    &required,
                    &opaque_provided,
                    &world.callable_header(c.caller).unwrap().schema.env,
                    None
                )
                .unwrap()
        );

        // Measure only this proof, not fixture construction. Supply exactly its
        // observed work/retained-byte demand, then one unit less independently.
        // These calls all enter the unequal-region fallback and build the map.
        let before = world.budget.usage_snapshot();
        assert!(refine(&world, &required, &provided).unwrap());
        let after = world.budget.usage_snapshot();
        let work = after.work - before.work;
        let bytes = after.bytes - before.bytes;
        assert!(work > 0 && bytes > 0);
        for guard in 0..4 {
            let guard_case = case();
            let guard_world = guard_case.build().unwrap();
            let budget = &guard_world.budget;
            let used = budget.usage_snapshot();
            let remaining_work = work - usize::from(guard == 1);
            let remaining_bytes = bytes - usize::from(guard == 2);
            budget
                .charge_work(budget.limits().work - used.work - remaining_work)
                .unwrap();
            budget
                .retain_bytes(budget.limits().bytes - used.bytes - remaining_bytes)
                .unwrap();
            if guard == 3 {
                budget.poison_ledger_for_tests();
            }
            let result = refine(&guard_world, &required, &provided);
            if guard == 0 {
                assert!(result.unwrap());
                let exact = budget.usage_snapshot();
                assert_eq!(exact.work, budget.limits().work);
                assert_eq!(exact.bytes, budget.limits().bytes);
            } else {
                assert!(matches!(result, Err(error) if error.kind() == FactsErrorKind::Resource));
                if guard != 3 {
                    let failure = budget.first_resource_failure().unwrap();
                    assert_eq!(
                        failure.kind,
                        if guard == 1 {
                            ir::ResourceBudgetKind::Work
                        } else {
                            ir::ResourceBudgetKind::Bytes
                        }
                    );
                    budget.poison_ledger_for_tests();
                    assert!(matches!(refine(&guard_world, &required, &provided),
                        Err(error) if error.kind() == FactsErrorKind::Resource));
                    assert_eq!(budget.first_resource_failure(), Some(failure));
                }
            }
        }
    }

    #[test]
    fn synthetic_impl_heads_preserve_nonreference_arguments_and_reference_policy() {
        let budget = PlanBudget::default();
        let program = crate::compiler::frontend::parse(
            "trait Raw<Rhs:?Sized>{}impl<T:?Sized> Raw<T> for bool{}trait Literal<Rhs:?Sized>{}impl Literal<str> for bool{}trait TupleArg<Rhs>{}impl TupleArg<(i32,bool)> for bool{}trait Borrowed<Rhs>{}impl<T:?Sized> Borrowed<&T> for &T{}fn raw<T:?Sized>() where bool:Raw<T>{}fn literal<T:Literal<str>>(_:T){}fn tuple<T:TupleArg<(i32,bool)>>(_:T){}fn borrowed<T:Borrowed<T>>(_:T){}fn main(){}",
        ).unwrap();
        let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let raw = plan.function_decl(0).unwrap();
        let literal = plan.function_decl(1).unwrap();
        let tuple = plan.function_decl(2).unwrap();
        let borrowed = plan.function_decl(3).unwrap();
        for actual in [Ty::Str, Ty::Int(IntTy::I32), reference(Ty::Str)] {
            let facts = world.callable_facts(raw).unwrap();
            world
                .discharge_direct(
                    &CallTarget {
                        declaration: raw,
                        type_args: Arc::from([actual]),
                    },
                    &facts,
                )
                .unwrap();
        }
        for declaration in [literal, tuple] {
            let facts = world.callable_facts(declaration).unwrap();
            world
                .discharge_direct(
                    &CallTarget {
                        declaration,
                        type_args: Arc::from([Ty::Bool]),
                    },
                    &facts,
                )
                .unwrap();
        }
        let facts = world.callable_facts(borrowed).unwrap();
        for leaf in [Ty::Int(IntTy::I32), Ty::Str] {
            let mut actual = leaf;
            for _ in 0..3 {
                actual = reference(actual);
                world
                    .discharge_direct(
                        &CallTarget {
                            declaration: borrowed,
                            type_args: Arc::from([actual.clone()]),
                        },
                        &facts,
                    )
                    .unwrap();
            }
        }
        assert!(
            world
                .discharge_direct(
                    &CallTarget {
                        declaration: borrowed,
                        type_args: Arc::from([Ty::Bool]),
                    },
                    &facts
                )
                .is_err()
        );
        // Keep generic_reference_world_transport_and_type_context as the
        // separate genuine callable raw-None denial gate, without relaxation.
    }

    #[test]
    fn reference_free_impl_nominal_results_use_original_facts_not_type_height() {
        use crate::compiler::frontend;
        let budget = PlanBudget::default();
        let program = frontend::parse(
            "enum Ordering{Less,Equal,Greater} enum Option<T>{None,Some(T)} struct Hidden<'a>{loan:&'a i32} enum Borrowed<'a>{None,Some(Hidden<'a>)} fn clean()->Option<Ordering>{Option::None} fn hidden<'a>(x:&'a i32)->Borrowed<'a>{Borrowed::Some(Hidden{loan:x})} fn main(){}",
        ).unwrap();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let clean_id = plan.function_decl(0).unwrap();
        let hidden_id = plan.function_decl(1).unwrap();
        let clean_facts = world.callable_facts(clean_id).unwrap();
        let hidden_facts = world.callable_facts(hidden_id).unwrap();
        let clean = plan.signature(clean_id).unwrap().signature.result;
        let hidden = plan.signature(hidden_id).unwrap().signature.result;
        let (opaque, type_height) =
            ir::reference_type_summary(&clean, false, &budget, 0, &mut 0).unwrap();
        assert!(
            !opaque && type_height > 0,
            "Option<Ordering> is nested but reference-free"
        );
        assert!(
            world
                .result_carriers_absent_for_tests(&clean, &clean_facts)
                .unwrap()
        );
        assert_eq!(ir::root_reference_layers(&hidden).unwrap(), 0);
        assert!(
            !world
                .result_carriers_absent_for_tests(&hidden, &hidden_facts)
                .unwrap()
        );
        let make = |mapping: [u16; 4], result: Ty| {
            let raw = RefContract {
                static_regions: 0,
                region_count: mapping.iter().copied().max().unwrap() + 1,
                input_regions: mapping
                    .into_iter()
                    .enumerate()
                    .map(|(i, region)| ir::InputRegion {
                        input: ir::InputReference {
                            param: i / 2,
                            position: ir::ReferencePosition {
                                depth: (i % 2) as u8,
                            },
                        },
                        region: ir::RegionParamId(region),
                    })
                    .collect::<Vec<_>>()
                    .into(),
                results: Arc::from([]),
                outlives: Arc::from([]),
            };
            Signature {
                params: vec![reference(reference(Ty::Int(IntTy::I32))); 2],
                result,
                ref_contract: ir::normalize_ref_contract(&raw, &budget).unwrap(),
            }
        };
        let required = make([0, 1, 2, 1], clean.clone());
        let provided = make([0, 1, 2, 3], clean.clone());
        assert!(
            !world
                .return_refines(&required.ref_contract, &provided.ref_contract)
                .unwrap()
        );
        assert!(
            world
                .reference_free_implementation_refines(
                    &required,
                    &provided,
                    clean_facts.env(),
                    Some(&clean_facts)
                )
                .unwrap()
        );
        let hidden_required = make([0, 1, 2, 1], hidden.clone());
        let hidden_provided = make([0, 1, 2, 3], hidden.clone());
        assert!(
            !world
                .reference_free_implementation_refines(
                    &hidden_required,
                    &provided,
                    hidden_facts.env(),
                    Some(&hidden_facts)
                )
                .unwrap()
        );
        assert!(
            !world
                .reference_free_implementation_refines(
                    &required,
                    &hidden_provided,
                    hidden_facts.env(),
                    Some(&hidden_facts)
                )
                .unwrap()
        );
        let Ty::Enum(owner) = hidden else {
            panic!("nominal hidden result");
        };
        let mut forged = owner.def().clone();
        let mut variants = forged.variants.to_vec();
        variants[1].fields = Arc::<[FieldDef]>::from([]).into();
        forged.variants = variants.into();
        let forged = Ty::Enum(ir::EnumTy::new(forged));
        assert!(
            matches!(world.result_carriers_absent_for_tests(&forged, &hidden_facts),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
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
        let Err(error) = world.instance_facts(&wrong_entry) else {
            panic!("accepted an instance without its required trait proof");
        };
        assert_eq!(error.kind(), FactsErrorKind::Unproved);
        assert_eq!(
            error.to_string(),
            "E0277: trait obligation is not proven by caller bounds or a conditional implementation"
        );
        let schema = world.schema_facts(c.item).unwrap();
        assert!(schema.must_copy(&Ty::Int(IntTy::I32)).is_err());
        assert!(world.callable_facts(c.item).is_err());
        // Match checked-call finalization: release the local borrow between
        // resolution and final signature replay, but keep the same proofs.
        let scratch = std::cell::RefCell::new(TraitScratch::default());
        let replay = |facts: &ValidatedFacts, call: &TraitCall| {
            let authority = facts.world();
            let signature = authority
                .instantiate_trait_signature_with(call, facts, &mut scratch.borrow_mut())
                .unwrap();
            let mut proofs = scratch.borrow_mut();
            facts
                .validate_signature_with(
                    SignatureSubject::Call(&Callee::TraitItem(call.clone())),
                    &signature,
                    &mut proofs,
                )
                .unwrap();
            let resolved = authority
                .resolve_trait_call_with(call, facts, &mut proofs)
                .unwrap();
            drop(proofs);
            facts
                .validate_signature_with(
                    SignatureSubject::Call(resolved.callee()),
                    resolved.signature(),
                    &mut scratch.borrow_mut(),
                )
                .unwrap();
            resolved
        };
        let call = c.call(c.boxed.clone());
        for _ in 0..8 {
            let repeated = replay(&closed, &call);
            assert!(
                world
                    .signatures_equal(repeated.signature(), resolved.signature())
                    .unwrap()
            );
        }
        assert_eq!(scratch.borrow().formed_trait_signatures.len(), 1);
        assert!(matches!(
            world.instantiate_trait_signature_with(
                &c.call(c.caller_parameter.clone()),
                &closed,
                &mut scratch.borrow_mut(),
            ),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
        assert_eq!(scratch.borrow().formed_trait_signatures.len(), 1);
        let mut forged = resolved.signature().clone();
        forged.result = Ty::Never;
        assert!(matches!(
            closed.validate_signature_with(
                SignatureSubject::Call(resolved.callee()),
                &forged,
                &mut scratch.borrow_mut(),
            ),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
        let deferred = replay(&universal, &c.call(c.caller_parameter.clone()));
        assert!(matches!(deferred.callee(), Callee::TraitItem(_)));
        assert_eq!(
            scratch.borrow().facts_key.unwrap().0,
            universal.predicates.index
        );
        let other = c.build().unwrap();
        let other_closed = other.instance_facts(&target).unwrap();
        replay(&other_closed, &call);
        assert_eq!(scratch.borrow().world, Some(other.id));
        assert!(matches!(
            other.resolve_trait_call_with(&call, &closed, &mut scratch.borrow_mut()),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
        replay(&closed, &call);
        assert_eq!(scratch.borrow().world, Some(world.id));
        let body_signature = world
            .discharge_direct_with(&target, &closed, &mut scratch.borrow_mut())
            .unwrap()
            .signature
            .clone();
        for _ in 0..4 {
            closed
                .validate_signature_with(
                    SignatureSubject::Body,
                    &body_signature,
                    &mut scratch.borrow_mut(),
                )
                .unwrap();
        }
        let mut forged_body = body_signature;
        forged_body.result = Ty::Never;
        assert!(matches!(
            closed.validate_signature_with(
                SignatureSubject::Body,
                &forged_body,
                &mut scratch.borrow_mut(),
            ),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
        assert!(closed.budget().charge_work(usize::MAX).is_err());
        let first_failure = closed.budget().first_resource_failure();
        closed.budget().poison_ledger_for_tests();
        assert!(matches!(
            closed.validate_signature_with(
                SignatureSubject::Call(resolved.callee()),
                resolved.signature(),
                &mut scratch.borrow_mut(),
            ),
            Err(error) if error.kind() == FactsErrorKind::Resource
        ));
        assert_eq!(closed.budget().first_resource_failure(), first_failure);
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
            fn impl_trait_safety(
                &self,
                implementation: DeclId,
                trait_decl: DeclId,
            ) -> Result<(bool, bool)> {
                self.raw.impl_trait_safety(implementation, trait_decl)
            }
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
        let raw =
            test_support::adapter(c.modules.clone(), &c.headers, &c.types, &c.budget).unwrap();
        let erased = Arc::new(ErasedScope {
            raw,
            declaration: c.body,
        });
        assert!(
            matches!(ValidatedTraitWorld::build(c.headers,erased,&c.budget),Err(e)if e.kind()==FactsErrorKind::InvalidMetadata)
        );
    }

    #[test]
    fn generic_resource_diagnostic_preserves_context_after_transport_exhaustion() {
        for operation in 0..3 {
            let budget = PlanBudget::new(ir::PlanLimits {
                work: 0,
                bytes: 0,
                depth: 0,
                ..ir::PlanLimits::default()
            });
            let result = match operation {
                0 => budget.charge_work(1),
                1 => budget.retain_bytes(1),
                _ => budget.check_depth(1),
            };
            let original = result.as_ref().unwrap_err().clone();
            let failure = budget.first_resource_failure();
            let error = charged(&budget, result).unwrap_err();
            assert_eq!(error.kind(), FactsErrorKind::Resource);
            assert_eq!(error.to_string(), original);
            assert_eq!(error.into_legacy_diagnostic(&budget), original);
            assert!(budget.has_resource_failure());
            assert_eq!(budget.first_resource_failure(), failure);
            assert!(budget.charge_work(1).is_err());
        }
        // Source-controlled suffixes cannot gain the static transport exception.
        assert!(
            FactsError::generic_resource_message(
                "generic planning resource limit exceeded: source-controlled suffix"
            )
            .is_none()
        );
    }

    #[test]
    fn predicate_formations_preserve_original_context_and_raw_isolation() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let other = world.callable_facts(c.body).unwrap();
        let predicates = [c.goal(c.boxed.clone())];
        let mut replays = NominalReplays::default();
        world
            .validate_predicates_with_facts(
                &predicates,
                facts.env(),
                None,
                &mut replays,
                Some(&facts),
            )
            .unwrap();
        assert_eq!(replays.raw_formations.retained_roots(), 0);
        let bound = replays.predicate_formations.caller_context;
        let completed = replays.predicate_formations.complete.len();
        assert!(Arc::ptr_eq(
            replays.predicate_formations._caller_env.as_ref().unwrap(),
            facts.env()
        ));
        world
            .validate_predicates_with(&predicates, facts.env(), None, &mut replays)
            .unwrap();
        assert_eq!(replays.predicate_formations.caller_context, bound);
        assert_eq!(replays.predicate_formations.complete.len(), completed);
        world
            .validate_predicates_with_facts(
                &predicates,
                facts.env(),
                None,
                &mut replays,
                Some(&facts),
            )
            .unwrap();
        assert_eq!(replays.predicate_formations.caller_context, bound);
        let foreign = c.build().unwrap();
        assert!(
            matches!(foreign.validate_predicates_with_facts(&predicates, facts.env(), None, &mut replays, Some(&facts)), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        assert!(
            matches!(world.validate_predicates_with_facts(&predicates, other.env(), None, &mut replays, Some(&facts)), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        let failure = facts.budget().first_resource_failure();
        facts.budget().poison_ledger_for_tests();
        assert!(
            matches!(world.validate_predicates_with_facts(&predicates, facts.env(), None, &mut replays, Some(&facts)), Err(error) if error.kind() == FactsErrorKind::Resource)
        );
        assert_eq!(facts.budget().first_resource_failure(), failure);
    }

    #[test]
    fn original_trait_proof_context_keeps_raw_and_foreign_memos_separate() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let other = world.callable_facts(c.body).unwrap();
        let context = ProofContext::from_facts(&facts);
        let goal = c.goal(c.boxed.clone());
        let mut scratch = ProofScratch::default();
        assert!(matches!(
            world.prove_in(&goal, context, &mut scratch).unwrap(),
            ProofOutcome::Proved(_)
        ));
        assert!(scratch.memo.is_empty());
        let checked = scratch.checked.as_deref_mut().unwrap();
        let pointer = checked as *const ProofScratch;
        let bound = checked.checked_context;
        let rows = checked.memo.len();
        assert!(rows > 0);
        assert!(Arc::ptr_eq(
            checked._checked_env.as_ref().unwrap(),
            facts.env()
        ));
        for _ in 0..3 {
            let raw = ProofContext {
                original: None,
                predicates: context.predicates,
                phase: context.phase,
            };
            checked
                .with_caller_arena(world.id, None, |nested| {
                    assert_ne!(nested as *const ProofScratch, pointer);
                    assert!(matches!(
                        world.prove_in(&goal, raw, nested)?,
                        ProofOutcome::Proved(_)
                    ));
                    Ok(())
                })
                .unwrap();
            assert!(matches!(
                world.prove_in(&goal, context, checked).unwrap(),
                ProofOutcome::Proved(_)
            ));
            assert_eq!(checked.checked_context, bound);
            assert_eq!(checked.memo.len(), rows);
            assert_eq!(checked as *const ProofScratch, pointer);
        }
        let foreign_environment = ProofContext {
            original: Some(&facts),
            predicates: other.predicates.as_ref(),
            phase: facts.phase(),
        };
        let foreign_phase = ProofContext {
            phase: FactsPhase::Closed,
            ..context
        };
        for invalid in [foreign_environment, foreign_phase] {
            assert!(
                matches!(world.prove_in(&goal, invalid, checked), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
            );
        }
        let foreign = c.build().unwrap();
        assert!(
            matches!(foreign.prove_in(&goal, context, checked), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        let mut different_budget = case();
        different_budget.budget = PlanBudget::default();
        let different_world = different_budget.build().unwrap();
        assert!(!facts.budget().same_authority(&different_budget.budget));
        assert!(
            matches!(different_world.prove_in(&goal, context, checked), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata)
        );
        let first_failure = facts.budget().first_resource_failure();
        facts.budget().poison_ledger_for_tests();
        assert!(
            matches!(world.prove_in(&goal, context, checked), Err(error) if error.kind() == FactsErrorKind::Resource)
        );
        assert_eq!(facts.budget().first_resource_failure(), first_failure);
    }

    #[test]
    fn checked_direct_targets_share_exact_caller_proof_arena() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let targets = [
            CallTarget {
                declaration: c.body,
                type_args: Arc::from([c.leaf.clone()]),
            },
            CallTarget {
                declaration: c.caller,
                type_args: Arc::from([c.boxed.clone()]),
            },
        ];
        assert_ne!(targets[0].declaration, targets[1].declaration);
        let mut scratch = TraitScratch::default();
        world
            .discharge_direct_with(&targets[0], &facts, &mut scratch)
            .unwrap();
        let arena = scratch.proofs.checked.as_deref().unwrap() as *const ProofScratch;
        world
            .discharge_direct_with(&targets[1], &facts, &mut scratch)
            .unwrap();
        assert_eq!(
            scratch.proofs.checked.as_deref().unwrap() as *const ProofScratch,
            arena
        );
        assert_eq!(scratch.directs.len(), 2);
        for target in targets.iter().rev() {
            world
                .discharge_direct_with(target, &facts, &mut scratch)
                .unwrap();
            assert_eq!(
                scratch.proofs.checked.as_deref().unwrap() as *const ProofScratch,
                arena
            );
        }
        assert_eq!(scratch.directs.len(), 2);
        for target in &targets {
            let callee = Callee::Direct(target.clone());
            let signature = world
                .discharge_direct_with(target, &facts, &mut scratch)
                .unwrap()
                .signature;
            world
                .validate_signature_uncached(
                    SignatureSubject::Call(&callee),
                    &signature,
                    &facts,
                    &mut scratch,
                )
                .unwrap();
        }
        let roots = scratch.signature_types.retained_roots();
        assert!(roots > 0);
        for target in targets.iter().rev() {
            let callee = Callee::Direct(target.clone());
            let signature = world
                .discharge_direct_with(target, &facts, &mut scratch)
                .unwrap()
                .signature;
            world
                .validate_signature_uncached(
                    SignatureSubject::Call(&callee),
                    &signature,
                    &facts,
                    &mut scratch,
                )
                .unwrap();
            assert_eq!(scratch.signature_types.retained_roots(), roots);
        }
    }

    #[test]
    fn checked_type_walk_isolates_nested_raw_and_foreign_callers() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let other = world.callable_facts(c.body).unwrap();
        let ty = Ty::Tuple(Arc::from([c.caller_parameter.clone(), c.leaf.clone()]));
        let mut parent = TypeWalk::default();
        world
            .validate_ty_inner_with_facts(
                &ty,
                facts.env(),
                0,
                TypeUse::Root,
                &mut parent,
                Some(&facts),
            )
            .unwrap();
        let context = parent.caller_context;
        let rows = parent.complete.len();
        assert!(rows > 0);
        assert!(Arc::ptr_eq(
            parent._caller_env.as_ref().unwrap(),
            facts.env()
        ));
        for _ in 0..3 {
            world
                .validate_ty_inner(&ty, facts.env(), 0, TypeUse::Root, &mut parent)
                .unwrap();
            assert_eq!(parent.caller_context, context);
            assert_eq!(parent.complete.len(), rows);
            world
                .validate_ty_inner_with_facts(
                    &ty,
                    facts.env(),
                    0,
                    TypeUse::Root,
                    &mut parent,
                    Some(&facts),
                )
                .unwrap();
            assert!(
                world
                    .validate_ty_inner(&ty, other.env(), 0, TypeUse::Root, &mut parent)
                    .is_err()
            );
            assert!(
                world
                    .validate_ty_inner_with_facts(
                        &ty,
                        other.env(),
                        0,
                        TypeUse::Root,
                        &mut parent,
                        Some(&facts)
                    )
                    .is_err()
            );
            assert_eq!(parent.caller_context, context);
            assert_eq!(parent.complete.len(), rows);
        }
        let foreign = c.build().unwrap();
        assert!(
            foreign
                .validate_ty_inner_with_facts(
                    &ty,
                    facts.env(),
                    0,
                    TypeUse::Root,
                    &mut parent,
                    Some(&facts)
                )
                .is_err()
        );
        let Ty::Struct(original) = &c.leaf else {
            panic!("expected nominal")
        };
        let mut forged = original.def().clone();
        forged.fields = Arc::<[FieldDef]>::from([]).into();
        let forged = Ty::Struct(StructTy::new(forged));
        assert!(
            world
                .validate_ty_inner_with_facts(
                    &forged,
                    facts.env(),
                    0,
                    TypeUse::Root,
                    &mut parent,
                    Some(&facts)
                )
                .is_err()
        );
        assert_eq!(parent.caller_context, context);
        assert_eq!(parent.complete.len(), rows);
        assert!(facts.budget().charge_work(usize::MAX).is_err());
        let failure = facts.budget().first_resource_failure();
        facts.budget().poison_ledger_for_tests();
        assert!(matches!(world.validate_ty_inner_with_facts(
            &ty, facts.env(), 0, TypeUse::Root, &mut parent, Some(&facts),
        ), Err(error) if error.kind() == FactsErrorKind::Resource));
        assert_eq!(facts.budget().first_resource_failure(), failure);
    }

    #[test]
    fn replay_rows_exact_short_overlap_and_postgrowth_preserve_old_owner() {
        for short in [false, true] {
            let c = case();
            let world = c.build().unwrap();
            let budget = &world.budget;
            let mut rows = MeteredReplayRows::<u64>::default();
            rows.prepare(&world).unwrap();
            for value in 0..rows.values.capacity() {
                rows.values.push(value as u64);
            }
            let old_capacity = rows.values.capacity();
            let old_bytes = MeteredReplayRows::<u64>::buffer_bytes(old_capacity).unwrap();
            let prepaid = MeteredReplayRows::<u64>::buffer_bytes(old_capacity * 4).unwrap();
            let used = budget.usage_snapshot().bytes;
            budget
                .retain_bytes(budget.limits().bytes - used - prepaid + usize::from(short))
                .unwrap();
            let before = budget.usage_snapshot().bytes;
            let observed = std::cell::Cell::new(0);
            let result = rows.prepare_with(&world, |actual| {
                assert!(actual >= old_capacity * 2 && actual <= old_capacity * 4);
                observed.set(actual);
                // Entire old owner plus full new allocation admission coexist.
                assert_eq!(budget.usage_snapshot().bytes, before + prepaid);
                Ok(())
            });
            if short {
                assert!(matches!(result, Err(error) if error.is_resource()));
                assert_eq!(observed.get(), 0);
                assert_eq!(rows.values.capacity(), old_capacity);
            } else {
                result.unwrap();
                assert!(observed.get() > 0);
                let new_bytes =
                    MeteredReplayRows::<u64>::buffer_bytes(rows.values.capacity()).unwrap();
                assert_eq!(
                    budget.usage_snapshot().bytes,
                    before - old_bytes + new_bytes
                );
            }
            assert_eq!(rows.len(), old_capacity);
            assert!(
                rows.iter()
                    .enumerate()
                    .all(|(index, value)| index as u64 == *value)
            );
            let paid = MeteredReplayRows::<u64>::buffer_bytes(rows.values.capacity()).unwrap();
            let before_drop = budget.usage_snapshot().bytes;
            let first_failure = budget.first_resource_failure();
            assert_eq!(first_failure.is_some(), short);
            assert!(!budget.usage_snapshot().poisoned);
            drop(rows);
            // A denied reservation latches Resource, not internal ledger poison.
            // Already admitted bytes still refund without clearing the failure.
            assert_eq!(before_drop - budget.usage_snapshot().bytes, paid);
            assert_eq!(budget.first_resource_failure(), first_failure);
        }
        let c = case();
        let world = c.build().unwrap();
        let mut rows = MeteredReplayRows::<u64>::default();
        rows.prepare(&world).unwrap();
        for value in 0..rows.values.capacity() {
            rows.values.push(value as u64);
        }
        let old_capacity = rows.values.capacity();
        let before = world.budget.usage_snapshot().bytes;
        let entered = std::cell::Cell::new(false);
        let error = rows
            .prepare_with(&world, |actual| {
                entered.set(true);
                assert!(actual >= old_capacity * 2);
                assert_eq!(
                    world.budget.usage_snapshot().bytes,
                    before + MeteredReplayRows::<u64>::buffer_bytes(old_capacity * 4).unwrap()
                );
                Err(FactsError {
                    kind: FactsErrorKind::Resource,
                    diagnostic: Diagnostic::Static("post-growth test denial"),
                })
            })
            .unwrap_err();
        assert!(error.is_resource());
        assert!(entered.get());
        assert_eq!(rows.values.capacity(), old_capacity);
        assert_eq!(rows.len(), old_capacity);
        assert_eq!(world.budget.usage_snapshot().bytes, before);
        assert!(
            rows.iter()
                .enumerate()
                .all(|(index, value)| index as u64 == *value)
        );
    }

    #[test]
    fn replay_rows_explicit_ledger_poison_preserves_parent_charge() {
        let c = case();
        let world = c.build().unwrap();
        let budget = &world.budget;
        let before_parent = budget.usage_snapshot().bytes;
        let parent_bytes = 64;
        let parent = budget.reserve_bytes(parent_bytes).unwrap();
        let mut rows = MeteredReplayRows::<u64>::default();
        rows.prepare(&world).unwrap();
        rows.values.push(7);
        let paid = MeteredReplayRows::<u64>::buffer_bytes(rows.values.capacity()).unwrap();
        let before_poison = budget.usage_snapshot().bytes;
        assert_eq!(before_poison, before_parent + parent_bytes + paid);
        assert!(budget.first_resource_failure().is_none());
        budget.poison_ledger_for_tests();
        assert!(budget.usage_snapshot().poisoned);
        assert!(matches!(rows.prepare(&world), Err(error) if error.is_resource()));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], 7);
        assert_eq!(budget.usage_snapshot().bytes, before_poison);
        drop(rows);
        // Explicit internal poison, unlike a Resource latch, conservatively
        // retains both this owner's admission and the independent parent.
        assert_eq!(budget.usage_snapshot().bytes, before_poison);
        drop(parent);
        assert_eq!(budget.usage_snapshot().bytes, before_poison);
        assert!(budget.first_resource_failure().is_none());
    }

    #[test]
    fn replay_rows_drop_payload_before_storage_lease() {
        struct Probe<'a> {
            budget: &'a PlanBudget,
            expected: &'a std::cell::Cell<usize>,
            observed: &'a std::cell::Cell<bool>,
        }
        impl Drop for Probe<'_> {
            fn drop(&mut self) {
                assert_eq!(self.budget.usage_snapshot().bytes, self.expected.get());
                self.observed.set(true);
            }
        }
        let c = case();
        let world = c.build().unwrap();
        let expected = std::cell::Cell::new(0);
        let observed = std::cell::Cell::new(false);
        let before = world.budget.usage_snapshot().bytes;
        let mut rows = MeteredReplayRows::default();
        rows.prepare(&world).unwrap();
        rows.values.push(Probe {
            budget: &world.budget,
            expected: &expected,
            observed: &observed,
        });
        expected.set(world.budget.usage_snapshot().bytes);
        assert!(expected.get() > before);
        drop(rows);
        assert!(observed.get());
        assert_eq!(world.budget.usage_snapshot().bytes, before);
    }

    #[test]
    fn replay_warm_lookup_exact_work_short_and_poison_preserve_rows() {
        for short in [false, true] {
            let c = case();
            let world = c.build().unwrap();
            let facts = world.callable_facts(c.caller).unwrap();
            let mut replays = NominalReplays::default();
            world
                .validate_ty_metadata_with(&c.leaf, facts.env(), 0, &mut replays)
                .unwrap();
            let before = world.budget.usage_snapshot().work;
            let height = replays
                .validated_type(&world, &c.leaf, facts.env())
                .unwrap();
            let cost = world.budget.usage_snapshot().work - before;
            assert!(height.is_some() && cost > 0);
            let used = world.budget.usage_snapshot().work;
            world
                .budget
                .charge_work(world.budget.limits().work - used - cost + usize::from(short))
                .unwrap();
            let rows = (replays.types.len(), replays.type_index.len());
            let result = replays.validated_type(&world, &c.leaf, facts.env());
            if short {
                assert!(matches!(result, Err(error) if error.is_resource()));
                let failure = world.budget.first_resource_failure().unwrap();
                assert!(
                    matches!(replays.validated_type(&world, &c.leaf, facts.env()),
                    Err(error) if error.is_resource())
                );
                assert_eq!(world.budget.first_resource_failure(), Some(failure));
            } else {
                assert_eq!(result.unwrap(), height);
                assert_eq!(
                    world.budget.usage_snapshot().work,
                    world.budget.limits().work
                );
            }
            assert_eq!((replays.types.len(), replays.type_index.len()), rows);
        }
    }

    #[test]
    fn replay_type_second_owner_failure_keeps_completed_rows_and_index() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let mut replays = NominalReplays::default();
        world
            .validate_ty_metadata_with(&c.leaf, facts.env(), 0, &mut replays)
            .unwrap();
        assert!(!replays.types.is_empty());
        // Fill both buffers to the same actual bound using completed genuine
        // metadata roots; duplicate hashes are legitimate completed entries.
        while replays.types.len() < replays.types.values.capacity()
            || replays.type_index.len() < replays.type_index.values.capacity()
        {
            replays
                .record_type(&c.leaf, facts.env(), 0, &world)
                .unwrap();
            assert!(replays.types.len() < 64);
        }
        let rows = replays.types.len();
        let old_types_capacity = replays.types.values.capacity();
        let old_index_capacity = replays.type_index.values.capacity();
        let before = world.budget.usage_snapshot().bytes;
        let first_entered = std::cell::Cell::new(false);
        let second_entered = std::cell::Cell::new(false);
        let result = replays.record_type_with(
            &c.leaf,
            facts.env(),
            0,
            &world,
            || {
                first_entered.set(true);
                Ok(())
            },
            || {
                second_entered.set(true);
                Err(FactsError {
                    kind: FactsErrorKind::Resource,
                    diagnostic: Diagnostic::Static("second-owner post-growth test denial"),
                })
            },
        );
        assert!(matches!(result, Err(error) if error.is_resource()));
        assert!(first_entered.get() && second_entered.get());
        assert_eq!(replays.types.len(), rows);
        assert_eq!(replays.type_index.len(), rows);
        assert!(replays.types.values.capacity() > old_types_capacity);
        assert_eq!(replays.type_index.values.capacity(), old_index_capacity);
        let old_bytes =
            MeteredReplayRows::<(Ty, GenericEnv, usize)>::buffer_bytes(old_types_capacity).unwrap();
        let new_bytes = MeteredReplayRows::<(Ty, GenericEnv, usize)>::buffer_bytes(
            replays.types.values.capacity(),
        )
        .unwrap();
        assert_eq!(
            world.budget.usage_snapshot().bytes,
            before - old_bytes + new_bytes
        );
        for (position, &(_, ordinal)) in replays.type_index.iter().enumerate() {
            assert_eq!(ordinal, position);
            assert!(same_type_allocation(&replays.types[ordinal].0, &c.leaf));
        }
        assert_eq!(
            replays
                .validated_type(&world, &c.leaf, facts.env())
                .unwrap(),
            Some(0)
        );
        // Retrying cannot observe a half-published success row.
        replays
            .record_type(&c.leaf, facts.env(), 0, &world)
            .unwrap();
        assert_eq!(replays.types.len(), rows + 1);
        assert_eq!(replays.type_index.len(), rows + 1);
    }

    #[test]
    fn replay_sorted_candidates_keep_source_env_order_and_live_issuer() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let other = world.callable_facts(c.body).unwrap();
        let mut replays = NominalReplays::default();
        let parameterized = Ty::Tuple(Arc::from([c.caller_parameter.clone(), c.leaf.clone()]));
        world
            .validate_ty_metadata_with(&parameterized, facts.env(), 0, &mut replays)
            .unwrap();
        let first = replays
            .validated_type(&world, &parameterized, facts.env())
            .unwrap()
            .unwrap();
        replays
            .record_type(&parameterized, facts.env(), first + 1, &world)
            .unwrap();
        assert_eq!(
            replays
                .validated_type(&world, &parameterized, facts.env())
                .unwrap(),
            Some(first)
        );
        assert_eq!(
            replays
                .validated_type(&world, &parameterized, other.env())
                .unwrap(),
            None
        );
        assert!(
            world
                .validate_ty_metadata_with(&parameterized, other.env(), 0, &mut replays)
                .is_err()
        );
        let Ty::Struct(original) = &c.leaf else {
            panic!("nominal fixture")
        };
        let mut definition = original.def().clone();
        definition.module = fixture_modules().core_root();
        definition.visibility = ir::Visibility::Public;
        let forged = Ty::Struct(StructTy::new(definition));
        assert_eq!(type_allocation(&c.leaf), type_allocation(&forged));
        assert_eq!(
            replays
                .validated_type(&world, &forged, facts.env())
                .unwrap(),
            None
        );
        assert!(
            world
                .validate_ty_metadata_with(&forged, facts.env(), 0, &mut replays)
                .is_err()
        );
        assert!(replays.type_index.windows(2).all(|rows| rows[0] <= rows[1]));
        let before = replays.types.len();
        // Check the issuer while the planning budget is still healthy. A prior
        // depth denial would otherwise mask this independent authority guard.
        assert!(world.budget.first_resource_failure().is_none());
        assert!(
            world
                .catalog
                .modules()
                .budget()
                .charge(super::super::expansion_budget::Counter::Work, u64::MAX)
                .is_err()
        );
        let failure = world.catalog.modules().budget().snapshot().failure;
        assert!(
            matches!(replays.validated_type(&world, &parameterized, facts.env()),
            Err(error) if error.is_resource())
        );
        assert!(
            matches!(replays.record_type(&c.leaf, facts.env(), 0, &world),
            Err(error) if error.is_resource())
        );
        assert_eq!(world.catalog.modules().budget().snapshot().failure, failure);
        assert_eq!(replays.types.len(), before);
    }

    #[test]
    fn metadata_replay_nested_callers_preserve_parent_and_reject_warm_foreign_inputs() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let other = world.callable_facts(c.body).unwrap();
        let parameterized = Ty::Tuple(Arc::from([c.caller_parameter.clone(), c.leaf.clone()]));
        let mut parent = NominalReplays::default();
        assert!(parent.bind_metadata_caller(world.id, Some(&facts)));
        let context = parent.metadata_context;
        assert!(Arc::ptr_eq(
            parent._metadata_env.as_ref().unwrap(),
            facts.env()
        ));

        // Simulate the nested raw walk before the enclosing Some(facts)
        // validation publishes its success. The parent is bound but unwarmed.
        world
            .validate_ty_metadata_with(&parameterized, facts.env(), 0, &mut parent)
            .unwrap();
        assert_eq!(parent.metadata_context, context);
        assert!(parent.types.is_empty());
        assert!(parent.facts.is_empty());
        // Two distinct raw roots retain their own completed rows, never the
        // genuine caller's enclosing rows or authority.
        world
            .validate_ty_metadata_with(&c.leaf, facts.env(), 0, &mut parent)
            .unwrap();
        let (raw_world, raw_env, raw) = parent.raw_metadata.as_ref().unwrap();
        assert_eq!(*raw_world, world.id);
        assert!(world.same_env(raw_env, facts.env()).unwrap());
        let raw_address = std::ptr::from_ref(raw.as_ref()) as usize;
        let raw_rows = (raw.types.len(), raw.facts.len(), raw.inputs.len());
        assert!(raw_rows.0 > 0);
        world
            .validate_ty_metadata_with_facts(
                &parameterized,
                facts.env(),
                0,
                &mut parent,
                Some(&facts),
            )
            .unwrap();
        let rows = (parent.types.len(), parent.facts.len(), parent.inputs.len());
        assert!(rows.0 > 0);

        for _ in 0..3 {
            // A warm enclosing Some -> nested None -> Some sequence retains
            // the exact parent context and all of its successful records.
            world
                .validate_ty_metadata_with(&parameterized, facts.env(), 0, &mut parent)
                .unwrap();
            world
                .validate_ty_metadata_with(&c.leaf, facts.env(), 0, &mut parent)
                .unwrap();
            let (raw_world, raw_env, raw) = parent.raw_metadata.as_ref().unwrap();
            assert_eq!(*raw_world, world.id);
            assert!(world.same_env(raw_env, facts.env()).unwrap());
            assert_eq!(std::ptr::from_ref(raw.as_ref()) as usize, raw_address);
            assert_eq!(
                (raw.types.len(), raw.facts.len(), raw.inputs.len()),
                raw_rows
            );
            assert_eq!(parent.metadata_context, context);
            assert!(Arc::ptr_eq(
                parent._metadata_env.as_ref().unwrap(),
                facts.env()
            ));
            assert_eq!(
                (parent.types.len(), parent.facts.len(), parent.inputs.len()),
                rows
            );
            world
                .validate_ty_metadata_with_facts(
                    &parameterized,
                    facts.env(),
                    0,
                    &mut parent,
                    Some(&facts),
                )
                .unwrap();
            assert!(matches!(world.validate_ty_metadata_with(
                &parameterized, other.env(), 0, &mut parent,
            ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
            assert!(matches!(world.validate_ty_metadata_with_facts(
                &parameterized, other.env(), 0, &mut parent, Some(&facts),
            ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
            assert_eq!(parent.metadata_context, context);
            assert!(Arc::ptr_eq(
                parent._metadata_env.as_ref().unwrap(),
                facts.env()
            ));
            assert_eq!(
                (parent.types.len(), parent.facts.len(), parent.inputs.len()),
                rows
            );
        }

        let Ty::Struct(original) = &c.leaf else {
            panic!("expected nominal")
        };
        let mut forged = original.def().clone();
        forged.fields = Arc::<[FieldDef]>::from([]).into();
        let forged = Ty::Struct(StructTy::new(forged));
        assert!(matches!(world.validate_ty_metadata_with(
            &forged, facts.env(), 0, &mut parent,
        ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        let foreign_world = c.build().unwrap();
        assert!(matches!(foreign_world.validate_ty_metadata_with_facts(
            &parameterized, facts.env(), 0, &mut parent, Some(&facts),
        ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        // These independently built worlds intentionally share immutable
        // source schemas and binder identities. Raw types carry no WorldId:
        // acceptance must equal a fresh raw validation, never a caller memo hit.
        let fresh_raw = foreign_world.validate_ty_metadata_with(
            &parameterized,
            facts.env(),
            0,
            &mut NominalReplays::default(),
        );
        let nested_raw =
            foreign_world.validate_ty_metadata_with(&parameterized, facts.env(), 0, &mut parent);
        assert!(fresh_raw.is_ok(), "fresh raw result: {fresh_raw:?}");
        assert!(nested_raw.is_ok(), "nested raw result: {nested_raw:?}");
        // A real changed source shape or binder remains invalid in either
        // arena, even though the foreign world has the same source registry.
        for (ty, env) in [(&forged, facts.env()), (&parameterized, other.env())] {
            assert!(matches!(foreign_world.validate_ty_metadata_with(
                ty, env, 0, &mut NominalReplays::default(),
            ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
            assert!(matches!(foreign_world.validate_ty_metadata_with(
                ty, env, 0, &mut parent,
            ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        }
        let other_raw = parent
            .raw_metadata_child(&world, other.env())
            .unwrap()
            .unwrap();
        assert_ne!(std::ptr::from_ref(other_raw) as usize, raw_address);
        assert!(
            parent
                .raw_metadata_child(&foreign_world, facts.env())
                .unwrap()
                .is_none()
        );
        let (_, _, raw) = parent.raw_metadata.as_ref().unwrap();
        assert_eq!(std::ptr::from_ref(raw.as_ref()) as usize, raw_address);
        assert_eq!(
            (raw.types.len(), raw.facts.len(), raw.inputs.len()),
            raw_rows
        );
        assert_eq!(parent.metadata_context, context);
        assert_eq!(
            (parent.types.len(), parent.facts.len(), parent.inputs.len()),
            rows
        );
        assert!(facts.budget().charge_work(usize::MAX).is_err());
        let failure = facts.budget().first_resource_failure();
        facts.budget().poison_ledger_for_tests();
        assert!(matches!(world.validate_ty_metadata_with(
            &parameterized, facts.env(), 0, &mut parent,
        ), Err(error) if error.kind() == FactsErrorKind::Resource));
        assert!(matches!(world.validate_ty_metadata_with_facts(
            &parameterized, facts.env(), 0, &mut parent, Some(&facts),
        ), Err(error) if error.kind() == FactsErrorKind::Resource));
        assert_eq!(facts.budget().first_resource_failure(), failure);
    }

    #[test]
    fn raw_metadata_environment_arenas_retain_only_completed_exact_proofs() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let other = world.callable_facts(c.body).unwrap();
        assert!(!world.same_env(facts.env(), other.env()).unwrap());
        let mut parent = NominalReplays::default();
        assert!(parent.bind_metadata_caller(world.id, Some(&facts)));
        let context = parent.metadata_context;
        let mut addresses = Vec::new();
        for env in [facts.env(), other.env(), world.empty.env()] {
            world
                .validate_ty_metadata_with(&c.leaf, env, 0, &mut parent)
                .unwrap();
            let raw = parent.raw_metadata_child(&world, env).unwrap().unwrap();
            addresses.push(std::ptr::from_ref(raw) as usize);
            assert!(!raw.types.is_empty());
        }
        assert_ne!(addresses[0], addresses[1]);
        assert_ne!(addresses[0], addresses[2]);
        assert_ne!(addresses[1], addresses[2]);
        assert_eq!(parent.raw_metadata_others.values.len(), 2);
        assert!(parent.types.is_empty());
        assert!(parent.facts.is_empty());
        let Ty::Struct(original) = &c.leaf else {
            panic!("nominal fixture")
        };
        let mut foreign_module = original.def().clone();
        foreign_module.module = fixture_modules().core_root();
        foreign_module.visibility = ir::Visibility::Public;
        let foreign_module = Ty::Struct(StructTy::new(foreign_module));
        let mut sibling_field = original.def().clone();
        sibling_field.fields = Arc::<[FieldDef]>::from([FieldDef {
            name: "bad_scope".into(),
            ty: Ty::Unit,
            visibility: ir::Visibility::Restricted(
                c.modules
                    .catalog_module(super::super::module_identity::CatalogModule::Ops),
            ),
            lifetime_bound: false,
        }])
        .into();
        let sibling_field = Ty::Struct(StructTy::new(sibling_field));
        for _ in 0..3 {
            for (index, env) in [facts.env(), other.env(), world.empty.env()]
                .into_iter()
                .enumerate()
            {
                let raw = parent.raw_metadata_child(&world, env).unwrap().unwrap();
                let rows = (raw.types.len(), raw.facts.len(), raw.inputs.len());
                assert_eq!(std::ptr::from_ref(raw) as usize, addresses[index]);
                world
                    .validate_ty_metadata_with(&c.leaf, env, 0, &mut parent)
                    .unwrap();
                for forged in [&foreign_module, &sibling_field] {
                    assert!(matches!(world.validate_ty_metadata_with(
                        forged, env, 0, &mut parent,
                    ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
                }
                let raw = parent.raw_metadata_child(&world, env).unwrap().unwrap();
                assert_eq!((raw.types.len(), raw.facts.len(), raw.inputs.len()), rows);
                assert_eq!(parent.metadata_context, context);
                assert!(parent.types.is_empty());
            }
        }
        // A failed binder check cannot become a success in the second arena.
        for _ in 0..2 {
            assert!(matches!(world.validate_ty_metadata_with(
                &c.caller_parameter, other.env(), 0, &mut parent,
            ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        }
        let foreign_world = c.build().unwrap();
        assert!(
            parent
                .raw_metadata_child(&foreign_world, facts.env())
                .unwrap()
                .is_none()
        );
        fn live_replay_rows<T>(rows: &MeteredReplayRows<T>) -> usize {
            if rows.storage.is_some() {
                MeteredReplayRows::<T>::buffer_bytes(rows.values.capacity()).unwrap()
            } else {
                assert_eq!(rows.values.capacity(), 0);
                0
            }
        }
        fn live_nested_inventory(replays: &NominalReplays) -> usize {
            live_replay_rows(&replays.facts)
                + live_replay_rows(&replays.inputs)
                + live_replay_rows(&replays.types)
                + live_replay_rows(&replays.type_index)
                + replays
                    .raw_metadata
                    .as_ref()
                    .map_or(0, |(_, _, child)| live_nested_inventory(child))
                + replays
                    .raw_metadata_others
                    .values
                    .iter()
                    .map(|row| live_nested_inventory(&row.child))
                    .sum::<usize>()
        }
        let before_drop = facts.budget().usage_snapshot().bytes;
        let reserved = 3
            * (std::mem::size_of::<NominalReplays>() + 2 * std::mem::size_of::<usize>())
            + RawMetadataArenas::buffer_bytes(parent.raw_metadata_others.values.capacity())
                .unwrap()
            + live_nested_inventory(&parent);
        drop(parent);
        assert_eq!(
            before_drop - facts.budget().usage_snapshot().bytes,
            reserved
        );

        // Warm all three environment slots, then poison the exact module issuer.
        let mut poisoned = NominalReplays::default();
        assert!(poisoned.bind_metadata_caller(world.id, Some(&facts)));
        for env in [facts.env(), other.env(), world.empty.env()] {
            world
                .validate_ty_metadata_with(&c.leaf, env, 0, &mut poisoned)
                .unwrap();
        }
        let module_budget = world.catalog.modules().budget();
        assert!(
            module_budget
                .charge(super::super::expansion_budget::Counter::Work, u64::MAX)
                .is_err()
        );
        let failure = module_budget.snapshot().failure;
        for env in [facts.env(), other.env(), world.empty.env()] {
            assert!(matches!(world.validate_ty_metadata_with(
                &c.leaf, env, 0, &mut poisoned,
            ), Err(error) if error.kind() == FactsErrorKind::Resource));
            assert_eq!(module_budget.snapshot().failure, failure);
        }
    }

    #[test]
    fn raw_metadata_additional_environment_exact_short_and_poisoned_admission() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let other = world.callable_facts(c.body).unwrap();
        let mut measured = NominalReplays::default();
        measured
            .raw_metadata_child(&world, facts.env())
            .unwrap()
            .unwrap();
        let before = facts.budget().usage_snapshot();
        let mut peak = before.bytes;
        measured
            .raw_metadata_child_with(&world, other.env(), || {
                peak = facts.budget().usage_snapshot().bytes;
                Ok(())
            })
            .unwrap()
            .unwrap();
        let after = facts.budget().usage_snapshot();
        let work = after.work - before.work;
        let bytes = peak - before.bytes;
        assert!(work > 0 && bytes > 0);
        for guard in 0..4 {
            let c = case();
            let world = c.build().unwrap();
            let facts = world.callable_facts(c.caller).unwrap();
            let other = world.callable_facts(c.body).unwrap();
            let budget = facts.budget();
            let mut parent = NominalReplays::default();
            let primary = parent
                .raw_metadata_child(&world, facts.env())
                .unwrap()
                .unwrap();
            let primary_address = std::ptr::from_ref(primary) as usize;
            let used = budget.usage_snapshot();
            budget
                .charge_work(budget.limits().work - used.work - work + usize::from(guard == 1))
                .unwrap();
            budget
                .retain_bytes(budget.limits().bytes - used.bytes - bytes + usize::from(guard == 2))
                .unwrap();
            if guard == 3 {
                budget.poison_ledger_for_tests();
            }
            let mut observed_peak = 0;
            let result = parent.raw_metadata_child_with(&world, other.env(), || {
                observed_peak = budget.usage_snapshot().bytes;
                Ok(())
            });
            if guard == 0 {
                assert!(result.unwrap().is_some());
                assert_eq!(budget.usage_snapshot().work, budget.limits().work);
                assert_eq!(observed_peak, budget.limits().bytes);
                assert_eq!(parent.raw_metadata_others.values.len(), 1);
            } else {
                assert!(matches!(result, Err(error) if error.kind() == FactsErrorKind::Resource));
                assert!(parent.raw_metadata_others.values.is_empty());
                assert_eq!(
                    std::ptr::from_ref(parent.raw_metadata.as_ref().unwrap().2.as_ref()) as usize,
                    primary_address
                );
                if guard != 3 {
                    let failure = budget.first_resource_failure().unwrap();
                    assert_eq!(
                        failure.kind,
                        if guard == 1 {
                            ir::ResourceBudgetKind::Work
                        } else {
                            ir::ResourceBudgetKind::Bytes
                        }
                    );
                    budget.poison_ledger_for_tests();
                    assert!(
                        matches!(parent.raw_metadata_child(&world, other.env()), Err(error) if error.kind() == FactsErrorKind::Resource)
                    );
                    assert_eq!(budget.first_resource_failure(), Some(failure));
                }
            }
        }
    }

    #[test]
    fn raw_metadata_nonfirst_growth_exact_short_failure_and_drop() {
        fn fixture() -> (PlanBudget, Vec<GenericEnv>, RawMetadataArenas) {
            let budget = PlanBudget::default();
            let registry = SnapshotId::standalone().unwrap();
            let declaration = test_support::source_decl(registry, 0, &budget).unwrap();
            let envs: Vec<_> = (0..16)
                .map(|arity| {
                    let parameter_scope = scope(declaration, arity, &budget);
                    GenericEnv::new(&[parameter_scope], &budget).unwrap()
                })
                .collect();
            let mut rows = RawMetadataArenas::default();
            rows.push_with(&budget, &envs[0], || Ok(())).unwrap();
            rows.push_with(&budget, &envs[1], || Ok(())).unwrap();
            while rows.values.len() < rows.values.capacity() {
                let index = rows.values.len();
                rows.push_with(&budget, &envs[index], || Ok(())).unwrap();
            }
            assert!(rows.values.len() >= 2 && rows.values.len() < envs.len());
            (budget, envs, rows)
        }
        let (budget, envs, mut rows) = fixture();
        let index = rows.values.len();
        let before = budget.usage_snapshot();
        let mut peak = 0;
        rows.push_with(&budget, &envs[index], || {
            peak = budget.usage_snapshot().bytes;
            Ok(())
        })
        .unwrap();
        let work = budget.usage_snapshot().work - before.work;
        let bytes = peak - before.bytes;
        assert!(work >= 2 && bytes > 0);
        for guard in 0..6 {
            let (budget, envs, mut rows) = fixture();
            let count = rows.values.len();
            let old_capacity = rows.values.capacity();
            let old_buffer = rows.values.as_ptr();
            let old_children: Vec<_> = rows
                .values
                .iter()
                .map(|row| std::ptr::from_ref(row.child.as_ref()))
                .collect();
            let used = budget.usage_snapshot();
            if guard <= 2 {
                budget
                    .charge_work(budget.limits().work - used.work - work + usize::from(guard == 1))
                    .unwrap();
                budget
                    .retain_bytes(
                        budget.limits().bytes - used.bytes - bytes + usize::from(guard == 2),
                    )
                    .unwrap();
            }
            if guard == 3 {
                budget.poison_ledger_for_tests();
            }
            let before = budget.usage_snapshot();
            let mut observed_peak = 0;
            let mut grew = false;
            let result = rows.push_with(&budget, &envs[count], || {
                grew = true;
                observed_peak = budget.usage_snapshot().bytes;
                if guard == 5 {
                    budget.poison_ledger_for_tests();
                }
                if guard >= 4 {
                    return Err(FactsError {
                        kind: FactsErrorKind::Resource,
                        diagnostic: Diagnostic::Static("injected after nonfirst raw arena growth"),
                    });
                }
                Ok(())
            });
            if guard == 0 {
                assert_eq!(result.unwrap(), count);
                assert!(grew);
                assert_eq!(observed_peak, budget.limits().bytes);
                assert_eq!(budget.usage_snapshot().work, budget.limits().work);
                assert!(rows.values.capacity() > count);
                assert_ne!(rows.values.as_ptr(), old_buffer);
                for (row, child) in rows.values.iter().zip(&old_children) {
                    assert_eq!(std::ptr::from_ref(row.child.as_ref()), *child);
                }
                let expected = (count + 1)
                    * (std::mem::size_of::<NominalReplays>() + 2 * std::mem::size_of::<usize>())
                    + RawMetadataArenas::buffer_bytes(rows.values.capacity()).unwrap();
                let live = budget.usage_snapshot().bytes;
                drop(rows);
                assert_eq!(live - budget.usage_snapshot().bytes, expected);
            } else {
                assert!(matches!(result, Err(error) if error.kind() == FactsErrorKind::Resource));
                assert_eq!(rows.values.len(), count);
                assert_eq!(rows.values.capacity(), old_capacity);
                assert_eq!(rows.values.as_ptr(), old_buffer);
                for (row, child) in rows.values.iter().zip(&old_children) {
                    assert_eq!(std::ptr::from_ref(row.child.as_ref()), *child);
                }
                if guard == 4 {
                    assert!(grew);
                    assert_eq!(budget.usage_snapshot().bytes, before.bytes);
                    assert!(budget.first_resource_failure().is_none());
                }
                if guard == 1 || guard == 2 {
                    let first = budget.first_resource_failure().unwrap();
                    assert_eq!(
                        first.kind,
                        if guard == 1 {
                            ir::ResourceBudgetKind::Work
                        } else {
                            ir::ResourceBudgetKind::Bytes
                        }
                    );
                    budget.poison_ledger_for_tests();
                    assert!(rows.push_with(&budget, &envs[count], || Ok(())).is_err());
                    assert_eq!(budget.first_resource_failure(), Some(first));
                }
                if guard == 3 || guard == 5 {
                    assert!(budget.usage_snapshot().poisoned);
                    assert_eq!(grew, guard == 5);
                    let charged = budget.usage_snapshot().bytes;
                    drop(rows);
                    assert_eq!(budget.usage_snapshot().bytes, charged);
                }
            }
        }
    }

    #[test]
    fn raw_predicate_formations_reuse_only_complete_exact_context() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let other = world.callable_facts(c.body).unwrap();
        let goal = c.goal(c.leaf.clone());
        let Predicate::Implements(goal) = goal else {
            panic!("trait fixture")
        };
        let mut replays = NominalReplays::default();
        world
            .validate_ref_with(&goal, facts.env(), None, &mut replays)
            .unwrap();
        let roots = replays.raw_formations.retained_roots();
        assert!(roots > 0);
        for _ in 0..3 {
            world
                .validate_ref_with(&goal, facts.env(), None, &mut replays)
                .unwrap();
            assert_eq!(replays.raw_formations.retained_roots(), roots);
        }
        let Ty::Struct(original) = &c.leaf else {
            panic!("nominal fixture")
        };
        assert!(original.def().fields.is_empty());
        let mut definition = original.def().clone();
        definition.fields = Arc::<[FieldDef]>::from([FieldDef {
            name: "forged".into(),
            ty: Ty::Unit,
            visibility: ir::Visibility::Public,
            lifetime_bound: false,
        }])
        .into();
        let forged = Ty::Struct(StructTy::new(definition));
        assert!(matches!(replays.raw_formations.check(
            &world, &forged, facts.env(), TypeUse::Root, None, 0,
        ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        let foreign = case().build().unwrap();
        assert!(matches!(replays.raw_formations.check(
            &foreign, &c.leaf, facts.env(), TypeUse::Root, None, 0,
        ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        let parameterized = Ty::Tuple(Arc::from([c.caller_parameter.clone(), c.leaf.clone()]));
        replays
            .raw_formations
            .check(&world, &parameterized, facts.env(), TypeUse::Root, None, 0)
            .unwrap();
        assert!(matches!(replays.raw_formations.check(
            &world, &parameterized, other.env(), TypeUse::Root, None, 0,
        ), Err(error) if error.kind() == FactsErrorKind::InvalidMetadata));
        let Ty::Param(parameter) = c.caller_parameter else {
            panic!("parameter fixture")
        };
        assert!(matches!(replays.raw_formations.check(
            &world, &parameterized, facts.env(), TypeUse::Root, Some(parameter), 0,
        ), Err(error) if error.kind() == FactsErrorKind::Unsupported));
        // Formation uses the same role distinctions as the original full walk.
        // A bare slice is a legal trait Self root, not an aggregate field.
        let slice = Ty::Slice {
            element: Arc::new(Ty::Int(IntTy::I32)),
        };
        replays
            .raw_formations
            .check(&world, &slice, facts.env(), TypeUse::Root, None, 0)
            .unwrap();
        assert!(matches!(replays.raw_formations.check(
            &world, &slice, facts.env(), TypeUse::Aggregate, None, 0,
        ), Err(error) if error.kind() == FactsErrorKind::Unsupported));
        // Warm roots still enforce their full retained height at a deeper use.
        assert!(matches!(replays.raw_formations.check(
            &world, &parameterized, facts.env(), TypeUse::Root, None, facts.budget().limits().depth,
        ), Err(error) if error.kind() == FactsErrorKind::Resource));
        let failure = facts.budget().first_resource_failure();
        facts.budget().poison_ledger_for_tests();
        assert!(matches!(replays.raw_formations.check(
            &world, &c.leaf, facts.env(), TypeUse::Root, None, 0,
        ), Err(error) if error.kind() == FactsErrorKind::Resource));
        assert_eq!(facts.budget().first_resource_failure(), failure);
    }

    #[test]
    fn local_metadata_memo_preserves_projection_and_nominal_authority() {
        let c = case();
        let projection = |self_ty: Ty, item: DeclId| {
            Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                trait_ref: TraitRef {
                    declaration: c.value_trait,
                    self_ty,
                    args: Arc::from([]),
                },
                item,
            }))
        };
        let first = projection(c.leaf.clone(), c.item);
        let equivalent = projection(c.leaf.clone(), c.item);
        assert!(same_type_allocation(&first, &equivalent));
        assert!(!same_type_allocation(
            &first,
            &projection(c.leaf.clone(), c.body)
        ));
        let Ty::Struct(original) = &c.leaf else {
            panic!("expected nominal")
        };
        let mut forged = original.def().clone();
        forged.fields = Arc::<[FieldDef]>::from([]).into();
        let forged = projection(Ty::Struct(StructTy::new(forged)), c.item);
        assert!(!same_type_allocation(&first, &forged));
        let mut memo = ValidatedTypes::default();
        memo.check(&first, |_| Ok::<_, ()>(())).unwrap();
        memo.check(&equivalent, |_| Err::<(), _>(())).unwrap();
        assert!(memo.check(&forged, |_| Err::<(), _>(())).is_err());

        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let mut replays = NominalReplays::default();
        let nested = Ty::Tuple(Arc::from([c.leaf.clone(), c.leaf.clone()]));
        world
            .validate_ty_metadata_with(&nested, facts.env(), 0, &mut replays)
            .unwrap();
        assert!(
            replays
                .validated_type(&world, &nested, facts.env())
                .unwrap()
                .is_some()
        );
        assert!(
            replays
                .validated_type(&world, &c.leaf, facts.env())
                .unwrap()
                .is_some()
        );
        let Ty::AssociatedType(forged_projection) = &forged else {
            unreachable!()
        };
        assert!(
            world
                .validate_ty_metadata_with(
                    &forged_projection.trait_ref.self_ty,
                    facts.env(),
                    0,
                    &mut replays
                )
                .is_err()
        );
        // Exercise the public node-local metadata API used by function
        // preflight, not only its lower-level replay memo.
        let mut scratch = TraitScratch::default();
        facts
            .validate_ty_metadata_scratch(&nested, facts.budget(), &mut scratch)
            .unwrap();
        for _ in 0..8 {
            facts
                .validate_ty_metadata_scratch(&nested, facts.budget(), &mut scratch)
                .unwrap();
        }
        let forged_nominal = &forged_projection.trait_ref.self_ty;
        let (Ty::Struct(valid), Ty::Struct(changed)) = (&c.leaf, forged_nominal) else {
            unreachable!()
        };
        assert_eq!(valid.id(), changed.id());
        assert!(matches!(
            facts.validate_ty_metadata_scratch(forged_nominal, facts.budget(), &mut scratch),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
        // A warm success cannot change budget authority or bypass sticky
        // resource accounting, even when no nominal replay is needed.
        let foreign = PlanBudget::default();
        assert!(matches!(
            facts.validate_ty_metadata_scratch(&nested, &foreign, &mut scratch),
            Err(error) if error.kind() == FactsErrorKind::InvalidMetadata
        ));
        assert!(facts.budget().charge_work(usize::MAX).is_err());
        let failure = facts.budget().first_resource_failure();
        // A rejected oversized reservation records evidence without consuming
        // the remaining allowance. Poison the ledger explicitly to exercise
        // the independent sticky-corruption boundary.
        facts.budget().poison_ledger_for_tests();
        let poisoned_result =
            facts.validate_ty_metadata_scratch(&nested, facts.budget(), &mut scratch);
        assert!(
            matches!(
                &poisoned_result,
                Err(error) if error.kind() == FactsErrorKind::Resource
            ),
            "unexpected poisoned result: {poisoned_result:?}, failure: {failure:?}"
        );
        assert_eq!(facts.budget().first_resource_failure(), failure);
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
            finalized: world.owner_normalization_finalized,
            goal: goal.clone(),
        };
        // The memo is node-local: a second proof in the same scratch hits it,
        // a fresh scratch recomputes, and only DFS ancestry forms a cycle.
        let mut scratch = ProofScratch::default();
        assert!(matches!(
            world.prove(&goal, &facts, &mut scratch).unwrap(),
            ProofOutcome::Proved(_)
        ));
        let len = scratch.checked.as_ref().unwrap().memo.len();
        assert!(len > 0);
        world.prove(&goal, &facts, &mut scratch).unwrap();
        assert_eq!(scratch.checked.as_ref().unwrap().memo.len(), len);
        world.prove_goal(&goal, &facts).unwrap();
        let mut cyclic = ProofScratch::default();
        cyclic
            .with_caller_arena(world.id, Some(&facts), |arena| {
                arena.active.push(key);
                Ok(())
            })
            .unwrap();
        assert!(
            matches!(world.prove(&goal,&facts,&mut cyclic),Err(e)if e.kind()==FactsErrorKind::Cycle)
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
            test_support::world(c.modules.clone(), &c.headers.callables, &tiny)
                .unwrap_err()
                .is_resource()
        );
    }

    #[test]
    fn canonical_sync_structural_proof_preserves_real_payloads_and_negative_cases() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let cell = Ty::Cell(ir::CellTy::new(&world.catalog, Ty::Bool, &c.budget).unwrap());
        let pointer = Ty::RawPtr {
            kind: BorrowKind::Shared,
            pointee: Arc::new(Ty::Bool),
        };
        let wrap =
            |ty| Ty::ManuallyDrop(ir::ManuallyDropTy::new(&world.catalog, ty, &c.budget).unwrap());
        let marker =
            |ty| Ty::PhantomData(ir::PhantomDataTy::new(&world.catalog, ty, &c.budget).unwrap());
        let prove = |ty| {
            world
                .prove_goal(
                    &Predicate::Implements(TraitRef {
                        declaration: world.catalog.id(CatalogItem::Sync),
                        self_ty: ty,
                        args: Arc::from([]),
                    }),
                    &facts,
                )
                .unwrap()
        };
        for ty in [
            Ty::Bool,
            Ty::Char,
            Ty::Int(IntTy::I128),
            Ty::Float(ir::FloatTy::F64),
            Ty::Unit,
            Ty::Never,
            c.leaf.clone(),
            Ty::Tuple(Arc::from([Ty::Bool, Ty::Char])),
            Ty::Array {
                element: Arc::new(Ty::Bool),
                len: 2,
            },
            Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(Ty::Bool),
            },
            Ty::Ref {
                kind: BorrowKind::Mut,
                referent: Arc::new(Ty::Bool),
            },
            wrap(Ty::Bool),
            marker(Ty::Bool),
            Ty::FnPointer(ir::FnPointerTy::new(vec![Ty::Bool], Ty::Bool, &c.budget).unwrap()),
        ] {
            assert!(matches!(prove(ty), ProofOutcome::Proved(_)));
        }
        for ty in [
            cell.clone(),
            pointer.clone(),
            Ty::Tuple(Arc::from([Ty::Bool, cell.clone()])),
            Ty::Array {
                element: Arc::new(cell.clone()),
                len: 0,
            },
            Ty::Ref {
                kind: BorrowKind::Shared,
                referent: Arc::new(cell.clone()),
            },
            Ty::Ref {
                kind: BorrowKind::Mut,
                referent: Arc::new(cell.clone()),
            },
            wrap(cell.clone()),
            marker(cell),
            marker(pointer),
            c.caller_parameter.clone(),
        ] {
            assert!(matches!(prove(ty), ProofOutcome::NotProven));
        }
        let remaining = c.budget.limits().work - c.budget.usage_snapshot().work;
        c.budget.charge_work(remaining).unwrap();
        assert_eq!(c.budget.usage_snapshot().work, c.budget.limits().work);
        let exhausted = world
            .prove_goal(
                &Predicate::Implements(TraitRef {
                    declaration: world.catalog.id(CatalogItem::Sync),
                    self_ty: Ty::Bool,
                    args: Arc::from([]),
                }),
                &facts,
            )
            .unwrap_err();
        assert_eq!(exhausted.kind(), FactsErrorKind::Resource);
        let failure = c.budget.first_resource_failure().unwrap();
        assert_eq!(failure.kind, ir::ResourceBudgetKind::Work);
        assert_eq!(failure.used, failure.limit);
        assert!(c.budget.charge_work(1).is_err());
        assert_eq!(c.budget.first_resource_failure(), Some(failure));
    }

    #[test]
    fn canonical_sync_exact_assumptions_reject_foreign_owner_predicate_and_cycle() {
        let c = case();
        let original = c.build().unwrap();
        let mut header = c
            .headers
            .callables
            .iter()
            .find(|h| h.declaration == c.caller)
            .unwrap()
            .clone();
        header.schema.predicates = Arc::from([Predicate::Implements(TraitRef {
            declaration: original.catalog.id(CatalogItem::Sync),
            self_ty: c.caller_parameter.clone(),
            args: Arc::from([]),
        })]);
        let world = test_support::world(c.modules.clone(), &[header], &c.budget).unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let goal = Predicate::Implements(TraitRef {
            declaration: world.catalog.id(CatalogItem::Sync),
            self_ty: c.caller_parameter.clone(),
            args: Arc::from([]),
        });
        assert!(matches!(
            world.prove_goal(&goal, &facts).unwrap(),
            ProofOutcome::Proved(_)
        ));
        let mut erased = (*facts).clone();
        erased.predicates = world.empty.clone();
        assert_eq!(
            world.prove_goal(&goal, &erased).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
        let mut foreign = goal.clone();
        let Predicate::Implements(application) = &mut foreign else {
            unreachable!()
        };
        let Ty::Param(parameter) = &mut application.self_ty else {
            unreachable!()
        };
        parameter.owner.declaration.snapshot = SnapshotId::standalone().unwrap();
        assert_eq!(
            world.prove_goal(&foreign, &facts).unwrap_err().kind(),
            FactsErrorKind::InvalidMetadata
        );
        let mut changed_trait = goal.clone();
        let Predicate::Implements(application) = &mut changed_trait else {
            unreachable!()
        };
        application.declaration = world.catalog.id(CatalogItem::Copy);
        assert!(matches!(
            world.prove_goal(&changed_trait, &facts).unwrap(),
            ProofOutcome::NotProven
        ));
        let mut cyclic = ProofScratch::default();
        cyclic
            .with_caller_arena(world.id, Some(&facts), |arena| {
                arena.active.push(GoalKey {
                    environment: facts.predicates.index,
                    phase: facts.phase(),
                    finalized: world.owner_normalization_finalized,
                    goal: goal.clone(),
                });
                Ok(())
            })
            .unwrap();
        assert_eq!(
            world.prove(&goal, &facts, &mut cyclic).unwrap_err().kind(),
            FactsErrorKind::Unsupported
        );
    }

    #[test]
    fn canonical_sync_explicit_unsafe_impl_precedes_fields_and_suppresses_default() {
        for (source, proven) in [
            (
                "struct S<T>{v:T} unsafe impl<T> Sync for S<T>{} fn probe(x:S<*const i32>){} fn main(){}",
                true,
            ),
            (
                "trait Mark{} struct S<T>{v:T} unsafe impl<T:Mark> Sync for S<T>{} fn probe(x:S<i32>){} fn main(){}",
                false,
            ),
        ] {
            let program = crate::compiler::frontend::parse(source).unwrap();
            let budget = PlanBudget::default();
            let plan = crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let declaration = plan.function_decl(0).unwrap();
            let world = plan.trait_world().unwrap();
            let facts = world.callable_facts(declaration).unwrap();
            let schema = world.normalized_callable_schema(declaration).unwrap();
            let goal = Predicate::Implements(TraitRef {
                declaration: world.catalog.id(CatalogItem::Sync),
                self_ty: schema.signature.params[0].clone(),
                args: Arc::from([]),
            });
            assert_eq!(
                matches!(
                    world.prove_goal(&goal, &facts).unwrap(),
                    ProofOutcome::Proved(_)
                ),
                proven
            );
        }
    }

    #[test]
    fn canonical_sync_unused_specialized_unsafe_impl_is_invalid() {
        for source in [
            "struct S<T>{v:T} unsafe impl Sync for S<i32>{} fn main(){}",
            "struct S<T,U>{v:T,w:U} unsafe impl<T> Sync for S<T,T>{} fn main(){}",
        ] {
            let program = crate::compiler::frontend::parse(source).unwrap();
            let budget = PlanBudget::default();
            let error =
                crate::compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap_err();
            assert!(error.contains("E0366"), "{error}");
        }
    }

    #[test]
    fn canonical_copy_logical_permission_and_supertrait_closure() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        assert!(facts.must_copy(&Ty::Int(IntTy::I32)).unwrap());
        assert!(facts.must_copy(&Ty::Char).unwrap());
        assert!(world.eq_ty(&Ty::Char, &Ty::Char, 0).unwrap());
        assert!(!world.eq_ty(&Ty::Char, &Ty::Int(IntTy::U32), 0).unwrap());
        assert!(!world.eq_ty(&Ty::Char, &Ty::Bool, 0).unwrap());
        assert!(!facts.must_copy(&c.caller_parameter).unwrap());
        assert!(!facts.must_copy(&c.leaf).unwrap());
        let f32_ty = Ty::Float(ir::FloatTy::F32);
        let f64_ty = Ty::Float(ir::FloatTy::F64);
        assert!(facts.must_copy(&f32_ty).unwrap());
        assert!(facts.must_copy(&f64_ty).unwrap());
        assert!(world.eq_ty(&f32_ty, &f32_ty, 0).unwrap());
        assert!(world.eq_ty(&f64_ty, &f64_ty, 0).unwrap());
        assert!(!world.eq_ty(&f32_ty, &f64_ty, 0).unwrap());
        assert!(!world.eq_ty(&f32_ty, &Ty::Int(IntTy::I32), 0).unwrap());
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
        let bounded = test_support::world(c.modules.clone(), &[callable], &c.budget).unwrap();
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
    fn wide_integer_identity_and_builtin_scalar_facts() {
        let c = case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        for kind in [IntTy::I128, IntTy::U128] {
            let ty = Ty::Int(kind);
            assert!(facts.must_copy(&ty).unwrap());
            assert!(world.eq_ty(&ty, &ty, 0).unwrap());
            for other in [IntTy::I64, IntTy::U64, IntTy::I128, IntTy::U128] {
                assert_eq!(world.eq_ty(&ty, &Ty::Int(other), 0).unwrap(), kind == other);
            }
            for item in [CatalogItem::Copy, CatalogItem::Clone, CatalogItem::Sized] {
                let predicate = Predicate::Implements(TraitRef {
                    declaration: world.catalog.id(item),
                    self_ty: ty.clone(),
                    args: Arc::from([]),
                });
                assert!(matches!(
                    world.prove_goal(&predicate, &facts).unwrap(),
                    ProofOutcome::Proved(_)
                ));
            }
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
        let snapshot = SnapshotId::standalone().unwrap();
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
    fn original_callable_view_retains_rigid_header_and_exact_subject() {
        let budget = PlanBudget::default();
        let header = carrier_header(&budget, 1, |p| sig(vec![p[0].clone()], p[0].clone()));
        let world =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
        let universal = world.callable_facts(header.declaration).unwrap();
        let target = CallTarget {
            declaration: header.declaration,
            type_args: Arc::from([reference(Ty::Int(IntTy::I32))]),
        };
        let closed = world.instance_facts(&target).unwrap();
        for facts in [&universal, &closed] {
            let view = facts
                .original_callable_schema(OriginalCallableSubject::Direct(header.declaration))
                .unwrap();
            assert_eq!(view.declaration(), header.declaration);
            assert!(
                matches!(view.subject(), OriginalCallableSubject::Direct(d) if d == header.declaration)
            );
            assert!(Arc::ptr_eq(view.facts(), facts));
            assert!(
                world
                    .same_env(view.generic_env(), &header.schema.env)
                    .unwrap()
            );
            assert!(
                world
                    .eq_signature(view.signature(), &header.schema.signature)
                    .unwrap()
            );
            assert_eq!(
                view.predicates().as_ref(),
                universal.predicates().predicates()
            );
            assert_eq!(view.source_region_env().declaration, header.declaration);
            assert_eq!(view.receiver(), ReceiverMode::None);
            assert!(matches!(view.signature().result, Ty::Param(_)));
            view.validate_for(facts).unwrap();
            assert!(facts.is_source_callable(header.declaration).unwrap());
        }
        assert!(
            !closed
                .carrier_contract(SignatureSubject::Body)
                .unwrap()
                .ref_contract()
                .results
                .is_empty()
        );
        assert!(header.schema.signature.ref_contract.results.is_empty());
    }

    #[test]
    fn original_callable_view_rejects_foreign_schema_and_rebound_contexts() {
        let c = borrowed_case();
        let world = c.build().unwrap();
        let facts = world.callable_facts(c.caller).unwrap();
        let view = facts
            .original_callable_schema(OriginalCallableSubject::Direct(c.caller))
            .unwrap();
        let rebound = Arc::new((*facts).clone());
        assert!(
            matches!(view.validate_for(&rebound), Err(e) if e.kind() == FactsErrorKind::InvalidMetadata)
        );
        let other = c.build().unwrap();
        assert!(
            view.validate_for(&other.callable_facts(c.caller).unwrap())
                .is_err()
        );
        let mut foreign = c.caller;
        foreign.snapshot = SnapshotId::standalone().unwrap();
        assert!(
            matches!(facts.original_callable_schema(OriginalCallableSubject::Direct(foreign)), Err(e) if e.kind() == FactsErrorKind::InvalidMetadata)
        );
        let schema = world.schema_facts(c.item).unwrap();
        assert!(
            matches!(schema.original_callable_schema(OriginalCallableSubject::Direct(c.caller)), Err(e) if e.kind() == FactsErrorKind::InvalidMetadata)
        );
        let required = facts
            .original_callable_schema(OriginalCallableSubject::TraitItem(
                &c.call(c.caller_parameter.clone()),
            ))
            .unwrap();
        assert_eq!(required.declaration(), c.item);
        required.validate_for(&facts).unwrap();
        assert!(matches!(required.validate_for(&rebound), Err(e)
            if e.kind() == FactsErrorKind::InvalidMetadata));
        let mut forged = (*facts).clone();
        forged.predicates = world.empty.clone();
        assert!(
            matches!(Arc::new(forged).original_callable_schema(OriginalCallableSubject::Direct(c.caller)), Err(e) if e.kind() == FactsErrorKind::InvalidMetadata)
        );
    }

    #[test]
    fn original_callable_view_uses_existing_ledger_on_every_lookup() {
        let budget = PlanBudget::default();
        let header = carrier_header(&budget, 1, |p| sig(vec![p[0].clone()], p[0].clone()));
        let world =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
        let facts = world.callable_facts(header.declaration).unwrap();
        let first = facts
            .original_callable_schema(OriginalCallableSubject::Direct(header.declaration))
            .unwrap();
        let second = facts
            .original_callable_schema(OriginalCallableSubject::Direct(header.declaration))
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(first.facts(), second.facts()));
        // Failed oversized charges do not consume the remaining allowance.
        for bit in (0..usize::BITS).rev() {
            let _ = budget.charge_work(1usize << bit);
        }
        assert!(budget.charge_work(1).is_err());
        assert!(
            matches!(facts.original_callable_schema(OriginalCallableSubject::Direct(header.declaration)), Err(e) if e.is_resource())
        );
        assert!(matches!(first.validate_for(&facts), Err(e) if e.is_resource()));
    }

    #[test]
    fn generic_carrier_world_same_parameter_transport_and_context_authority() {
        let budget = PlanBudget::default();
        let header = carrier_header(&budget, 2, |p| {
            sig(vec![p[0].clone(), p[1].clone()], p[0].clone())
        });
        let world =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
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
        let other =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
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
        let world =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
        let universal = world.callable_facts(header.declaration).unwrap();
        let rigid = universal.carrier_contract(SignatureSubject::Body).unwrap();
        assert_eq!(rigid.contract().writes.len(), 2);
        assert_eq!(rigid.contract().writes[0].destination.param, 0);
        assert_eq!(rigid.contract().writes[1].destination.param, 1);
        assert!(matches!(
            rigid.contract().writes[1].destination.location.channel,
            ir::CarrierChannel::ParamTail { prefix: 0, .. }
        ));
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
        let original_world = test_support::world(
            fixture_modules(),
            std::slice::from_ref(&original_header),
            &budget,
        )
        .unwrap();
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
    fn owned_generic_write_envelopes_filter_concrete_shared_paths() {
        let budget = PlanBudget::default();
        let header = carrier_header(&budget, 1, |p| {
            reference_signature(vec![p[0].clone()], Ty::Unit, &budget)
        });
        let world =
            test_support::world(fixture_modules(), std::slice::from_ref(&header), &budget).unwrap();
        let universal = world.callable_facts(header.declaration).unwrap();
        let retained = universal.carrier_contract(SignatureSubject::Body).unwrap();
        assert_eq!(retained.contract().writes.len(), 1);
        let reference = |kind, ty| Ty::Ref {
            kind,
            referent: Arc::new(ty),
        };
        for (actual, expected) in [
            (Ty::Int(IntTy::I32), 0),
            (reference(BorrowKind::Mut, Ty::Int(IntTy::I32)), 0),
            (
                reference(
                    BorrowKind::Shared,
                    reference(BorrowKind::Mut, Ty::Int(IntTy::I32)),
                ),
                0,
            ),
            (
                reference(
                    BorrowKind::Mut,
                    reference(BorrowKind::Mut, Ty::Int(IntTy::I32)),
                ),
                1,
            ),
        ] {
            let target = CallTarget {
                declaration: header.declaration,
                type_args: Arc::from([actual]),
            };
            let facts = world.instance_facts(&target).unwrap();
            let view = facts.carrier_contract(SignatureSubject::Body).unwrap();
            assert_eq!(view.contract().writes.len(), expected);
            if expected == 1 {
                assert_eq!(
                    view.contract().writes[0].destination.location.channel,
                    ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 })
                );
            }
        }
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
            fn impl_trait_safety(
                &self,
                implementation: DeclId,
                trait_decl: DeclId,
            ) -> Result<(bool, bool)> {
                self.raw.impl_trait_safety(implementation, trait_decl)
            }
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
                    carriers: Arc::from([]),
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
            constants: Arc::from([]),
            traits: Arc::from([]),
            associated_types: Arc::from([]),
            items: Arc::from([]),
            impls: Arc::from([]),
            callables: Arc::from([header.clone()]),
        };
        let raw = test_support::adapter(fixture_modules(), &headers, &[], &budget).unwrap();
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
            original: None,
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
