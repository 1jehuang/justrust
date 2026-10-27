//! Typed graph nominal source and closed field realization.
//! Outputs preserve original source slots and require explicit read capabilities.
mod coordinates;
mod preparation;
pub(super) use preparation::shape as source_shape;
mod component;
mod component_key;
mod materialization;
mod producer;
mod reads;
pub(super) mod substitution;
mod symbolic;
mod topology;
pub(super) use materialization::SymbolicFields;
use reads::NominalPublication;
pub(super) use reads::{ReadNominal, read_manifest};

/// Real source checking suspends only on explicit typed input demands. The
/// transient attempt keeps its original constant/source inventory across reads.
pub(super) async fn publish_callable(
    cx: &crate::engine::Context,
    types: &super::frontend::PlanCtx,
    declaration: ir::DeclId,
    mut constants: super::frontend::SealedBodyConstants,
    budget: &ir::PlanBudget,
) -> Result<
    (
        super::frontend::PublishedCallableBody,
        Vec<super::frontend::closures::check::CheckedClosure>,
        Vec<ir::OpaqueHiddenType>,
    ),
    Error,
> {
    let mut inputs = preparation::Capabilities::new();
    // Exact successful proofs remain node-local across missing-input retries.
    // No source-check result or unavailable graph capability is cached here.
    let mut scratch = traits::TraitScratch::default();
    loop {
        let attempt = inputs
            .with_inputs(types, declaration, |borrowed| {
                types
                    .publish_callable_body_with_nominal_inputs(
                        declaration,
                        constants,
                        budget,
                        None,
                        &borrowed,
                        &mut scratch,
                    )
                    .map_err(|e| e.to_string())
            })
            .map_err(Error::new)?;
        match attempt {
            super::frontend::NominalCallablePublication::Ready(body, closures, hidden) => {
                return Ok((body, closures, hidden));
            }
            super::frontend::NominalCallablePublication::Blocked {
                preparation,
                constants: retained,
            } => {
                constants = *retained;
                inputs.satisfy(cx, types, declaration, preparation).await?;
            }
        }
    }
}

use super::{ir, traits};
use crate::engine::{Context, Error, Key, OutputIdentity};
use std::sync::Arc;

pub(super) const KIND: u32 = 139;
const VERSION: u64 = 1;
const SOURCE_SCHEMA: u64 = 0;
const CLOSED_REALIZATION: u64 = 1;

#[derive(Clone, Debug)]
struct NominalSourceFact {
    identity: OutputIdentity,
    declaration: ir::DeclId,
    source: ir::Ty,
    proof: Arc<traits::NominalDeclarationProof>,
}

fn source_key(declaration: ir::DeclId) -> Key {
    crate::compiler::keys::params(
        KIND,
        VERSION,
        vec![
            SOURCE_SCHEMA,
            declaration.snapshot.graph_word(),
            u64::from(declaration.index),
        ],
    )
}

/// A real graph source-schema producer. Closed realization never reads the
/// mutable source interner as a replacement for this typed dependency.
pub(super) async fn run(cx: &Context, key: Key) -> Result<(), Error> {
    if key.params.first() != Some(&VERSION) {
        return Err(Error::new("unsupported nominal realization key version"));
    }
    match key.params.get(1) {
        Some(&SOURCE_SCHEMA) => {
            if key.params.len() != 4 {
                return Err(Error::new("nominal source schema key has invalid length"));
            }
            let declaration = ir::DeclId {
                snapshot: ir::SnapshotId::from_graph_word(key.params[2]),
                index: key.params[3],
            };
            if source_key(declaration) != key {
                return Err(Error::new("noncanonical nominal source schema key"));
            }
            let types_handle = cx
                .request(crate::compiler::keys::SignaturesDebtRoot.key())
                .get(0);
            let proof_handle = cx
                .request(crate::compiler::keys::params(
                    super::NOMINAL_DECL_WELL_FORMED,
                    declaration.snapshot.graph_word(),
                    vec![u64::from(declaration.index)],
                ))
                .get(0);
            let types: Arc<super::frontend::PlanCtx> = cx.read_fact(types_handle).await?;
            let proof: Arc<traits::NominalDeclarationProof> = cx.read_fact(proof_handle).await?;
            types
                .trait_world()
                .map_err(Error::new)?
                .replay_nominal_declaration(declaration, &proof)
                .map_err(|e| Error::new(format!("{e}")))?;
            let source = types
                .nominal_realization_template(declaration)
                .map_err(Error::new)?;
            cx.publish_fact(
                0,
                NominalSourceFact {
                    identity: cx.output_identity(0)?,
                    declaration,
                    source,
                    proof,
                },
            )
        }
        Some(&CLOSED_REALIZATION) => producer::run(cx, key).await,
        Some(&component_key::DOMAIN) => component::run(cx, key).await,
        _ => Err(Error::new("invalid nominal realization key domain")),
    }
}

#[derive(Clone)]
pub(super) struct ReadSource {
    fact: Arc<NominalSourceFact>,
}
impl ReadSource {
    pub(super) fn declaration(&self) -> ir::DeclId {
        self.fact.declaration
    }
    pub(super) fn template(&self) -> &ir::Ty {
        &self.fact.source
    }
    pub(super) fn proof(&self) -> &traits::NominalDeclarationProof {
        &self.fact.proof
    }
}

/// Original source schemas are demanded per declaration, not enumerated from
/// all known applications. They carry no closed layout permission.
pub(super) async fn read_sources(
    cx: &Context,
    declarations: &[ir::DeclId],
) -> Result<Vec<ReadSource>, Error> {
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(declarations.len())
        .map_err(|_| Error::new("source schema demand allocation failed"))?;
    for declaration in declarations {
        let handle = cx.request(source_key(*declaration)).get(0);
        let identity = handle.identity()?;
        pending.push((*declaration, identity, handle));
    }
    let mut sources = Vec::new();
    sources
        .try_reserve_exact(pending.len())
        .map_err(|_| Error::new("source schema capability allocation failed"))?;
    for (declaration, identity, handle) in pending {
        let fact: Arc<NominalSourceFact> = cx.read_fact(handle).await?;
        if fact.declaration != declaration || fact.identity != identity {
            return Err(Error::new("source schema typed read identity mismatch"));
        }
        sources.push(ReadSource { fact });
    }
    Ok(sources)
}

/// Coordinates only. Decoding/requesting this key does not grant type authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RealizationKey(Key);
impl RealizationKey {
    /// Necessary admission only. Complete source/actual/proof replay still
    /// follows on a cold path, and only completed facts may memoize success.
    fn check_replay_world(&self, world: u64, budget: &ir::PlanBudget) -> Result<(), Error> {
        budget.charge_work(1).map_err(Error::new)?;
        if self.0.kind != KIND
            || self.0.params.len() < 6
            || self.0.params[0] != VERSION
            || self.0.params[1] != CLOSED_REALIZATION
            || self.0.params[5] > (self.0.params.len() - 6) as u64
        {
            return Err(Error::new("invalid closed nominal replay header"));
        }
        if self.0.params[2] != world {
            return Err(Error::new("foreign closed nominal replay world"));
        }
        Ok(())
    }

    pub(super) fn encode(
        world: u64,
        declaration: ir::DeclId,
        arguments: &[Vec<u64>],
        max_words: usize,
    ) -> Result<Self, Error> {
        let mut count = 6usize;
        for argument in arguments {
            if argument.is_empty() {
                return Err(Error::new("empty nominal argument coordinate"));
            }
            count = count
                .checked_add(1)
                .and_then(|n| n.checked_add(argument.len()))
                .ok_or_else(|| Error::new("nominal realization key overflow"))?;
        }
        if count > max_words {
            return Err(Error::new("nominal realization key resource limit"));
        }
        let mut words = Vec::new();
        words
            .try_reserve_exact(count)
            .map_err(|_| Error::new("nominal realization key allocation failed"))?;
        words.extend([
            VERSION,
            CLOSED_REALIZATION,
            world,
            declaration.snapshot.graph_word(),
            u64::from(declaration.index),
            arguments.len() as u64,
        ]);
        for argument in arguments {
            words.push(argument.len() as u64);
            words.extend_from_slice(argument);
        }
        Ok(Self(crate::compiler::keys::words(KIND, words)?))
    }
}

/// Graph-issued identity in a separate domain from ir::NominalAppId source slots.
/// This remains separate until the explicit IR identity migration is complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RealizationIdentity(OutputIdentity);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FieldCoordinate {
    /// None is a struct field, Some is the original enum variant ordinal.
    pub(super) variant: Option<usize>,
    pub(super) field: usize,
}

/// Both source and normalized terms are retained. No source schema is overwritten.
#[derive(Clone, Debug)]
struct FieldRealization {
    coordinate: FieldCoordinate,
    source: ir::Ty,
    normalized: ir::Ty,
}

/// Immutable output payload. Construction belongs exclusively to the 139 producer.
/// It deliberately retains no PlanCtx, world, Facts, budget, lock or callback.
#[derive(Clone, Debug)]
struct RealizedNominal {
    key: RealizationKey,
    identity: RealizationIdentity,
    declaration: ir::DeclId,
    source_application: ir::NominalAppId,
    source_definition: ir::Ty,
    definition: ir::Ty,
    actuals: Arc<[ir::Ty]>,
    declaration_proof: traits::NominalDeclarationProof,
    fields: Arc<[FieldRealization]>,
    normalization: Arc<[traits::NormalizedAssociatedType]>,
    input: traits::ClosedNominalInput,
}

/// Only the producer calls this after canonical schema/projection proof replay.
/// Structural row checks do not replace that proof and grant no membership.
fn materialize_definition(
    source: &ir::Ty,
    identity: RealizationIdentity,
    actuals: Arc<[ir::Ty]>,
    rows: &[FieldRealization],
    budget: &ir::PlanBudget,
) -> Result<ir::Ty, Error> {
    let nominal_id = |original: ir::NominalAppId| ir::NominalAppId {
        snapshot: original.snapshot,
        index: ir::NominalIndex::GraphRealization(identity.0),
    };
    let mut consumed = 0usize;
    let mut fields = |original: &[ir::FieldDef], variant: Option<usize>| {
        budget.charge_work(original.len()).map_err(Error::new)?;
        budget
            .retain_bytes(std::mem::size_of_val(original))
            .map_err(Error::new)?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(original.len())
            .map_err(|_| Error::new("realized nominal field allocation failed"))?;
        for (field, source) in original.iter().enumerate() {
            budget.retain_bytes(source.name.len()).map_err(Error::new)?;
            let row = rows
                .get(consumed)
                .ok_or_else(|| Error::new("realized nominal omitted a field"))?;
            if row.coordinate != (FieldCoordinate { variant, field }) || row.source != source.ty {
                return Err(Error::new("realized nominal source field order changed"));
            }
            let mut output = source.clone();
            output.ty = row.normalized.clone();
            result.push(output);
            consumed += 1;
        }
        Ok::<Arc<[ir::FieldDef]>, Error>(result.into())
    };
    let result = match source {
        ir::Ty::Struct(source) => {
            budget
                .retain_bytes(source.def().name.len())
                .map_err(Error::new)?;
            let mut definition = source.def().clone();
            definition.id = nominal_id(definition.id);
            definition.type_args = actuals;
            definition.fields =
                fields(&definition.fields.read(budget).map_err(Error::new)?, None)?.into();
            ir::Ty::Struct(
                ir::StructTy::new(definition)
                    .with_native_sequence_binding(source.native_sequence_binding().cloned()),
            )
        }
        ir::Ty::Enum(source) => {
            budget
                .retain_bytes(source.def().name.len())
                .map_err(Error::new)?;
            let mut definition = source.def().clone();
            definition.id = nominal_id(definition.id);
            definition.type_args = actuals;
            let mut variants = Vec::new();
            budget
                .retain_bytes(std::mem::size_of_val(definition.variants.as_ref()))
                .map_err(Error::new)?;
            variants
                .try_reserve_exact(definition.variants.len())
                .map_err(|_| Error::new("realized nominal variant allocation failed"))?;
            for (ordinal, original) in definition.variants.iter().enumerate() {
                budget.charge_work(1).map_err(Error::new)?;
                budget
                    .retain_bytes(original.name.len())
                    .map_err(Error::new)?;
                let mut variant = original.clone();
                variant.fields = fields(
                    &original.fields.read(budget).map_err(Error::new)?,
                    Some(ordinal),
                )?
                .into();
                variants.push(variant);
            }
            definition.variants = variants.into();
            ir::Ty::Enum(ir::EnumTy::new(definition))
        }
        _ => {
            return Err(Error::new(
                "nominal realization source is not a nominal schema",
            ));
        }
    };
    if consumed != rows.len() {
        return Err(Error::new("realized nominal contains extra field rows"));
    }
    Ok(result)
}

/// A typed read receipt, bound to the exact requested key and issuing graph node.
/// This is NOT a StructDef/EnumDef and cannot silently enter executable IR.
#[derive(Clone)]
pub(super) struct AcyclicRead {
    fact: Arc<RealizedNominal>,
}
impl AcyclicRead {
    pub(super) fn retain_for_substitution(
        &self,
        ty: &ir::Ty,
        engine: crate::engine::EngineAuthority,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        if self.fact.identity.0.authority() != engine {
            return Err("foreign engine nominal substitution capability".into());
        }
        let facts = caller
            .validated()
            .ok_or("typed nominal substitution requires genuine caller facts")?;
        if !facts.budget().same_authority(budget) {
            return Err("typed nominal substitution budget authority mismatch".into());
        }
        // A parent's read cannot lend its engine authority to a child read.
        // Inspect the exact retained closed definition, not a world-only view.
        fn engines(
            ty: &ir::Ty,
            engine: crate::engine::EngineAuthority,
            budget: &ir::PlanBudget,
            depth: usize,
        ) -> Result<(), String> {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            let child = |ty: &ir::Ty| engines(ty, engine, budget, depth + 1);
            match ty {
                ir::Ty::Struct(n) => {
                    if n.id().index.is_graph() {
                        let cap = n
                            .realization()
                            .ok_or("nested graph nominal omitted typed read")?;
                        if cap.engine_authority() != engine {
                            return Err("foreign engine child nominal capability".into());
                        }
                        cap.check_definition(ty).map_err(|e| e.to_string())?;
                        if cap.is_component() {
                            return Ok(());
                        }
                    }
                    if n.is_back_edge() {
                        return Err("unsupported: recursive nominal substitution provenance".into());
                    }
                    for ty in n.def().type_args.iter() {
                        child(ty)?;
                    }
                    for field in n.def().fields.read(budget)?.iter() {
                        child(&field.ty)?;
                    }
                }
                ir::Ty::Enum(n) => {
                    if n.id().index.is_graph() {
                        let cap = n
                            .realization()
                            .ok_or("nested graph nominal omitted typed read")?;
                        if cap.engine_authority() != engine {
                            return Err("foreign engine child nominal capability".into());
                        }
                        cap.check_definition(ty).map_err(|e| e.to_string())?;
                        if cap.is_component() {
                            return Ok(());
                        }
                    }
                    if n.is_back_edge() {
                        return Err("unsupported: recursive nominal substitution provenance".into());
                    }
                    for ty in n.def().type_args.iter() {
                        child(ty)?;
                    }
                    for variant in n.def().variants.iter() {
                        for field in variant.fields.read(budget)?.iter() {
                            child(&field.ty)?;
                        }
                    }
                }
                ir::Ty::Tuple(types) => {
                    for ty in types.iter() {
                        child(ty)?;
                    }
                }
                ir::Ty::Ref { referent: ty, .. }
                | ir::Ty::RawPtr { pointee: ty, .. }
                | ir::Ty::Array { element: ty, .. }
                | ir::Ty::Slice { element: ty } => child(ty)?,
                // A closed object carries no nominal children to authenticate.
                ir::Ty::Dyn(object) if object.is_closed_leaf() => {}
                ir::Ty::Closure(_)
                | ir::Ty::Dyn(_)
                | ir::Ty::AssociatedType(_)
                | ir::Ty::Param(_) => {
                    return Err("unsupported: nonclosed nominal substitution provenance".into());
                }
                _ => {}
            }
            Ok(())
        }
        engines(ty, engine, budget, 0)?;
        // This replays the exact definition/key, declaration, ordered closed
        // actuals, source fields and complete normalization receipt inventory.
        // Empty-environment closedness means substitution cannot change a field
        // or an actual. Retain this same typed read, never re-intern its ID.
        self.replay(ty, facts.world())
            .map_err(|error| error.to_string())?;
        caller.validate_ty_with_budget(ty, budget)?;
        budget.retain_bytes(std::mem::size_of::<ir::Ty>())?;
        Ok(ty.clone())
    }

    /// Only a successful typed graph read can attach this capability. The fact
    /// itself retains the raw definition without its own receipt, avoiding a
    /// self-referential Arc cycle.
    pub(super) fn definition(&self) -> Result<ir::Ty, Error> {
        self.check_definition(&self.fact.definition)?;
        Ok(match &self.fact.definition {
            ir::Ty::Struct(definition) => ir::Ty::Struct(
                definition
                    .clone()
                    .with_realization(ReadNominal::Acyclic(self.clone())),
            ),
            ir::Ty::Enum(definition) => ir::Ty::Enum(
                definition
                    .clone()
                    .with_realization(ReadNominal::Acyclic(self.clone())),
            ),
            _ => {
                return Err(Error::new(
                    "typed nominal realization has no nominal definition",
                ));
            }
        })
    }

    /// Output coordinates alone are never authority. Require the exact
    /// definition allocation from the successfully read immutable output.
    pub(super) fn check_definition(&self, ty: &ir::Ty) -> Result<(), Error> {
        let (identity, declaration, actuals, same_definition) = match (ty, &self.fact.definition) {
            (ir::Ty::Struct(actual), ir::Ty::Struct(retained)) => (
                actual.id(),
                actual.def().source_decl,
                actual.def().type_args.as_ref(),
                std::ptr::eq(actual.def(), retained.def()),
            ),
            (ir::Ty::Enum(actual), ir::Ty::Enum(retained)) => (
                actual.id(),
                actual.def().source_decl,
                actual.def().type_args.as_ref(),
                std::ptr::eq(actual.def(), retained.def()),
            ),
            _ => {
                return Err(Error::new(
                    "nominal realization representation kind changed",
                ));
            }
        };
        if !same_definition
            || identity.index != ir::NominalIndex::GraphRealization(self.fact.identity.0)
            || identity.snapshot != self.fact.source_application.snapshot
            || declaration != self.fact.declaration
            || actuals != self.fact.actuals.as_ref()
        {
            return Err(Error::new(
                "nominal realization definition does not match typed read",
            ));
        }
        Ok(())
    }

    pub(super) fn replay(
        &self,
        ty: &ir::Ty,
        world: &traits::ValidatedTraitWorld,
    ) -> Result<(), Error> {
        self.replay_with(ty, world, &mut traits::NominalReplays::default())
    }

    /// Replay against `world`, skipping the world proof replay when this
    /// node's computation already replayed the same retained fact. The exact
    /// definition check always runs. Only successes are recorded.
    pub(super) fn replay_with(
        &self,
        ty: &ir::Ty,
        world: &traits::ValidatedTraitWorld,
        replays: &mut traits::NominalReplays,
    ) -> Result<(), Error> {
        ReadNominal::Acyclic(self.clone()).replay_with(ty, world, replays)
    }

    fn replay_uncached(
        &self,
        world: &traits::ValidatedTraitWorld,
        replays: &mut traits::NominalReplays,
    ) -> Result<(), Error> {
        let ty = &self.fact.definition;
        let budget = world.budget();
        let error = |e: traits::FactsError| Error::new(e.into_legacy_diagnostic(budget));
        let mut actual_terms = Vec::new();
        budget
            .retain_bytes(
                self.fact
                    .actuals
                    .len()
                    .saturating_mul(std::mem::size_of::<coordinates::Term>()),
            )
            .map_err(Error::new)?;
        actual_terms
            .try_reserve_exact(self.fact.actuals.len())
            .map_err(|_| Error::new("nominal replay key allocation failed"))?;
        for actual in self.fact.actuals.iter() {
            actual_terms.push(coordinates::Term::from_ty(actual, budget, 0)?);
        }
        let expected_key = producer::key(
            world.nominal_realization_world(),
            self.fact.declaration,
            &actual_terms,
            budget,
        )?;
        if self.fact.key != expected_key {
            return Err(Error::new(
                "nominal realization key differs from proof world, declaration or ordered actuals",
            ));
        }
        world
            .replay_closed_nominal_application(
                replays,
                &self.fact.input,
                self.fact.declaration,
                &self.fact.declaration_proof,
                &self.fact.source_definition,
                &self.fact.actuals,
            )
            .map_err(error)?;
        let source_identity = match &self.fact.source_definition {
            ir::Ty::Struct(source) => source.id(),
            ir::Ty::Enum(source) => source.id(),
            _ => {
                return Err(Error::new(
                    "nominal source application has no source definition",
                ));
            }
        };
        if self.fact.source_application != source_identity {
            return Err(Error::new(
                "nominal source application differs from authenticated source schema",
            ));
        }
        world
            .replay_nominal_normalizations(
                replays,
                &self.fact.input,
                &self.fact.source_definition,
                &self.fact.normalization,
            )
            .map_err(error)?;
        let mut ordinal = 0usize;
        let mut fields = |original: &[ir::FieldDef],
                          realized: &[ir::FieldDef],
                          variant: Option<usize>|
         -> Result<(), Error> {
            if original.len() != realized.len() {
                return Err(Error::new("realized nominal field count changed"));
            }
            for (field, (source, actual)) in original.iter().zip(realized).enumerate() {
                budget.charge_work(1).map_err(Error::new)?;
                let row = self
                    .fact
                    .fields
                    .get(ordinal)
                    .ok_or_else(|| Error::new("realized nominal field receipt omitted"))?;
                if row.coordinate != (FieldCoordinate { variant, field })
                    || row.source != source.ty
                    || row.normalized != actual.ty
                    || source.name != actual.name
                    || source.visibility != actual.visibility
                    || source.lifetime_bound != actual.lifetime_bound
                {
                    return Err(Error::new(
                        "realized nominal field order, source or visibility changed",
                    ));
                }
                world
                    .replay_nominal_field_term(replays, &self.fact.input, &source.ty, &actual.ty, 0)
                    .map_err(error)?;
                ordinal += 1;
            }
            Ok(())
        };
        match (&self.fact.source_definition, ty) {
            (ir::Ty::Struct(source), ir::Ty::Struct(actual)) => {
                let (a, b) = (source.def(), actual.def());
                if a.name != b.name || a.module != b.module || a.visibility != b.visibility {
                    return Err(Error::new("realized struct source shell changed"));
                }
                fields(
                    &a.fields.read(budget).map_err(Error::new)?,
                    &b.fields.read(budget).map_err(Error::new)?,
                    None,
                )?;
            }
            (ir::Ty::Enum(source), ir::Ty::Enum(actual)) => {
                let (a, b) = (source.def(), actual.def());
                if a.name != b.name
                    || a.module != b.module
                    || a.visibility != b.visibility
                    || a.variants.len() != b.variants.len()
                {
                    return Err(Error::new("realized enum source shell changed"));
                }
                for (variant, (a, b)) in a.variants.iter().zip(b.variants.iter()).enumerate() {
                    if a.name != b.name || a.shape != b.shape {
                        return Err(Error::new("realized enum variant order or shape changed"));
                    }
                    fields(
                        &a.fields.read(budget).map_err(Error::new)?,
                        &b.fields.read(budget).map_err(Error::new)?,
                        Some(variant),
                    )?;
                }
            }
            _ => return Err(Error::new("realized nominal source representation changed")),
        }
        if ordinal != self.fact.fields.len() {
            return Err(Error::new("realized nominal contains extra field receipts"));
        }
        Ok(())
    }

    /// Same retained immutable fact allocation (node-local replay dedup only).
    pub(super) fn same_fact(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.fact, &other.fact)
    }

    pub(super) fn source_application(&self) -> ir::NominalAppId {
        self.fact.source_application
    }
}

/// Async frontier shared by BODY and future layout-free source preparation.
/// All independent requests are issued before the first await. It reads only
/// the caller-supplied per-item manifest, never a registry-wide application set.
/// Facts remain keyed by exact ordered coordinate words even under collisions.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_world_header_rejects_malformed_and_preserves_resource_precedence() {
        let declaration = ir::DeclId {
            snapshot: ir::SnapshotId::from_graph_word(41),
            index: 2,
        };
        let key = RealizationKey::encode(7, declaration, &[], 32).unwrap();
        let budget = ir::PlanBudget::default();
        key.check_replay_world(7, &budget).unwrap();
        assert!(
            key.check_replay_world(8, &budget)
                .unwrap_err()
                .to_string()
                .contains("foreign closed nominal replay world")
        );
        for length in 0..6 {
            let mut short = key.clone();
            short.0.params.truncate(length);
            assert!(
                short
                    .check_replay_world(7, &budget)
                    .unwrap_err()
                    .to_string()
                    .contains("invalid closed nominal replay header")
            );
        }
        for field in [0, 1, 5] {
            let mut malformed = key.clone();
            malformed.0.params[field] = u64::MAX;
            assert!(
                malformed
                    .check_replay_world(7, &budget)
                    .unwrap_err()
                    .to_string()
                    .contains("invalid closed nominal replay header")
            );
        }
        let mut wrong_kind = key.clone();
        wrong_kind.0.kind = 0;
        assert!(wrong_kind.check_replay_world(7, &budget).is_err());
        assert!(budget.first_resource_failure().is_none());
        for short in [false, true] {
            let budget = ir::PlanBudget::default();
            budget
                .charge_work(budget.limits().work - usize::from(!short))
                .unwrap();
            let result = key.check_replay_world(7, &budget);
            if short {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("planning resource")
                );
                let failure = budget.first_resource_failure().unwrap();
                // A malformed/foreign header must not replace sticky Resource
                // with an invalid-input diagnostic when admission is poisoned.
                let error = wrong_kind.check_replay_world(8, &budget).unwrap_err();
                assert!(error.to_string().contains("planning"));
                assert_eq!(budget.first_resource_failure(), Some(failure));
            } else {
                result.unwrap();
                assert_eq!(budget.usage_snapshot().work, budget.limits().work);
                assert!(budget.first_resource_failure().is_none());
            }
        }
    }

    #[derive(Clone)]
    struct FactProvider {
        key: RealizationKey,
        source: ir::Ty,
        proof: traits::NominalDeclarationProof,
        input: traits::ClosedNominalInput,
        stolen: Option<Arc<RealizedNominal>>,
        mutate: fn(&mut RealizedNominal),
        fields: Arc<[FieldRealization]>,
    }
    impl crate::engine::Provider for FactProvider {
        fn outputs(&self, _: &Key) -> usize {
            1
        }
        fn start(&self, cx: Context, key: Key) -> crate::engine::TaskFuture {
            let this = self.clone();
            Box::pin(async move {
                if key.kind == 998 {
                    let mut capabilities =
                        read_manifest(&cx, &[this.key], &ir::PlanBudget::default()).await?;
                    return cx
                        .publish_fact(0, capabilities.pop().expect("one successful typed read"));
                }
                if key.kind == 999 {
                    let capabilities =
                        read_manifest(&cx, &[this.key], &ir::PlanBudget::default()).await?;
                    return cx.publish_fact(0, capabilities.len());
                }
                if let Some(stolen) = this.stolen {
                    return cx.publish_fact(0, reads::NominalPublication::Acyclic(stolen));
                }
                let ir::Ty::Struct(source) = &this.source else {
                    return Err(Error::new("test requires a source struct"));
                };
                let identity = RealizationIdentity(cx.output_identity(0)?);
                let definition = materialize_definition(
                    &this.source,
                    identity,
                    Arc::from([]),
                    &this.fields,
                    &ir::PlanBudget::default(),
                )?;
                let mut fact = RealizedNominal {
                    key: this.key,
                    identity,
                    declaration: source.def().source_decl,
                    source_application: source.id(),
                    source_definition: this.source.clone(),
                    definition,
                    actuals: Arc::from([]),
                    declaration_proof: this.proof,
                    fields: this.fields,
                    normalization: Arc::from([]),
                    input: this.input,
                };
                (this.mutate)(&mut fact);
                cx.publish_fact(0, reads::NominalPublication::Acyclic(Arc::new(fact)))
            })
        }
    }

    #[test]
    fn typed_realization_read_rejects_successful_fact_from_foreign_engine() {
        let program = super::super::frontend::parse("struct Empty;fn main(){}").unwrap();
        let plan =
            super::super::frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default())
                .unwrap();
        let declaration = plan.nominal_declaration(0).unwrap();
        let proof = plan.publish_nominal_declaration(declaration).unwrap();
        let source = plan.nominal_realization_template(declaration).unwrap();
        let key = RealizationKey::encode(7, declaration, &[], 32).unwrap();
        let input = plan
            .trait_world()
            .unwrap()
            .close_nominal_input(declaration, &proof, &[])
            .unwrap();
        let provider = FactProvider {
            key: key.clone(),
            source,
            proof,
            input,
            stolen: None,
            mutate: |_| {},
            fields: Arc::from([]),
        };
        let first = crate::engine::Engine::new(provider.clone());
        let own = first.request(crate::compiler::keys::params(999, 0, vec![]));
        first.run(1).unwrap();
        own.completion().unwrap();
        let publication = first
            .fact_of::<reads::NominalPublication>(&key.0, 0)
            .unwrap();
        let reads::NominalPublication::Acyclic(successful) = publication.as_ref() else {
            panic!("expected acyclic fixture")
        };
        let successful = successful.clone();
        let foreign = crate::engine::Engine::new(FactProvider {
            stolen: Some(successful),
            ..provider
        });
        let demand = foreign.request(crate::compiler::keys::params(999, 0, vec![]));
        foreign.run(1).unwrap();
        assert!(demand.completion().is_err());
        assert!(
            foreign
                .fact_of::<usize>(&crate::compiler::keys::params(999, 0, vec![]), 0)
                .is_none()
        );
    }

    #[test]
    fn current_engine_substitution_replays_typed_read_and_rejects_foreign_engine() {
        let budget = ir::PlanBudget::default();
        let program = super::super::frontend::parse("struct Empty;fn main(){}").unwrap();
        let plan = super::super::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.nominal_declaration(0).unwrap();
        let proof = plan.publish_nominal_declaration(declaration).unwrap();
        let source = plan.nominal_realization_template(declaration).unwrap();
        let world = plan.trait_world().unwrap();
        let input = world.close_nominal_input(declaration, &proof, &[]).unwrap();
        let key = RealizationKey::encode(world.nominal_realization_world(), declaration, &[], 32)
            .unwrap();
        let provider = FactProvider {
            key,
            source,
            proof,
            input,
            stolen: None,
            mutate: |_| {},
            fields: Arc::from([]),
        };
        let engine = crate::engine::Engine::new(provider.clone());
        let demand = engine.request(crate::compiler::keys::params(998, 0, vec![]));
        engine.run(1).unwrap();
        demand.completion().unwrap();
        let capability = demand.fact::<ReadNominal>(0).unwrap();
        let ty = capability.definition().unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let caller = ir::FactsCtx::Validated(&facts);
        let super::super::frontend::PublishedCallableBody::Source(publication) = plan
            .publish_callable_body(plan.function_decl(0).unwrap(), &budget)
            .unwrap()
        else {
            panic!("expected canonical main publication")
        };
        let target = ir::CallTarget {
            declaration: plan.function_decl(0).unwrap(),
            type_args: Arc::from([]),
        };
        let target = &target;
        let mut specialization = super::super::opaque_specialization::Specialization::new(
            &publication,
            target,
            &plan,
            &budget,
            engine.authority(),
        )
        .unwrap();
        assert!(specialization.ty(&ty).is_ok());
        capability
            .retain_for_substitution(&ty, engine.authority(), caller, &budget)
            .unwrap();
        let foreign_engine = crate::engine::Engine::new(provider);
        let mut foreign_specialization = super::super::opaque_specialization::Specialization::new(
            &publication,
            target,
            &plan,
            &budget,
            foreign_engine.authority(),
        )
        .unwrap();
        assert!(foreign_specialization.ty(&ty).is_err());
        assert!(
            capability
                .retain_for_substitution(&ty, foreign_engine.authority(), caller, &budget)
                .is_err()
        );
        assert!(
            ir::SubstitutionCx::retain_realized_nominal(&mut plan.clone(), &ty, caller, &budget)
                .is_err()
        );
        let ir::Ty::Struct(nominal) = &ty else {
            panic!("struct capability required")
        };
        for mode in 0..4 {
            let mut definition = nominal.def().clone();
            match mode {
                0 => {} // same identity, separately allocated definition
                1 => definition.source_decl.index += 1,
                2 => definition.type_args = Arc::from([ir::Ty::Bool]),
                _ => definition.id.index = ir::NominalIndex::SourceSlot(0),
            }
            let changed = ir::Ty::Struct(
                ir::StructTy::new(definition).with_realization(capability.as_ref().clone()),
            );
            assert!(
                capability
                    .retain_for_substitution(&changed, engine.authority(), caller, &budget)
                    .is_err(),
                "typed read accepted altered definition mode {mode}"
            );
        }
        let missing = ir::Ty::Struct(ir::StructTy::new(nominal.def().clone()));
        assert!(specialization.ty(&missing).is_err());
        let mut local_plan = plan.clone();
        let empty = ir::GenericEnv::new(&[], &budget).unwrap();
        assert!(
            substitution::substitute_ty(
                &missing,
                &empty,
                &[],
                caller,
                &mut local_plan,
                &budget,
                Some(engine.authority())
            )
            .is_err()
        );
        let foreign_program = super::super::frontend::parse("struct Empty;fn main(){}").unwrap();
        let foreign_plan =
            super::super::frontend::PlanCtx::from_program(&foreign_program, &budget).unwrap();
        let foreign_facts = foreign_plan
            .callable_facts(foreign_plan.function_decl(0).unwrap())
            .unwrap();
        assert!(
            capability
                .retain_for_substitution(
                    &ty,
                    engine.authority(),
                    ir::FactsCtx::Validated(&foreign_facts),
                    &budget
                )
                .is_err()
        );
    }

    #[test]
    fn closed_call_replay_rejects_changed_source_application() {
        let budget = ir::PlanBudget::default();
        let program =
            super::super::frontend::parse("struct Empty;struct Wrapper<T>{value:T}fn consume(value:Empty){}fn main(){}fn nested(value:Wrapper<Empty>){}")
                .unwrap();
        let plan = super::super::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.nominal_declaration(0).unwrap();
        let proof = plan.publish_nominal_declaration(declaration).unwrap();
        let source = plan.nominal_realization_template(declaration).unwrap();
        let world = plan.trait_world().unwrap();
        let input = world.close_nominal_input(declaration, &proof, &[]).unwrap();
        let key = RealizationKey::encode(world.nominal_realization_world(), declaration, &[], 32)
            .unwrap();
        let engine = crate::engine::Engine::new(FactProvider {
            key,
            source,
            proof,
            input,
            stolen: None,
            mutate: |_| {},
            fields: Arc::from([]),
        });
        let demand = engine.request(crate::compiler::keys::params(998, 0, vec![]));
        engine.run(1).unwrap();
        demand.completion().unwrap();
        let capability = demand.fact::<ReadNominal>(0).unwrap();
        let target = ir::Callee::Direct(ir::CallTarget {
            declaration: plan.function_decl(0).unwrap(),
            type_args: Arc::from([]),
        });
        let facts = plan
            .instance_facts(&ir::CallTarget {
                declaration: plan.function_decl(1).unwrap(),
                type_args: Arc::from([]),
            })
            .unwrap();
        let ir::Callee::Direct(call) = &target else {
            unreachable!()
        };
        let canonical = facts
            .service()
            .discharge_direct(call, &facts)
            .unwrap()
            .signature()
            .clone();
        let mut mapped = canonical.clone();
        mapped.params[0] = capability.definition().unwrap();
        substitution::replay_closed_signature(
            engine.authority(),
            &target,
            &canonical,
            &mapped,
            ir::FactsCtx::from_validated(&facts),
            &budget,
        )
        .unwrap();
        let nested_target = ir::CallTarget {
            declaration: plan.function_decl(2).unwrap(),
            type_args: Arc::from([]),
        };
        let nested_canonical = facts
            .service()
            .discharge_direct(&nested_target, &facts)
            .unwrap()
            .signature()
            .clone();
        let wrapper = plan.nominal_declaration(1).unwrap();
        let mut nested_mapped = nested_canonical.clone();
        nested_mapped.params[0] = plan
            .intern_nominal(
                wrapper,
                &[capability.definition().unwrap()],
                ir::FactsCtx::from_validated(&facts),
                &budget,
            )
            .unwrap();
        substitution::replay_closed_signature(
            engine.authority(),
            &ir::Callee::Direct(nested_target.clone()),
            &nested_canonical,
            &nested_mapped,
            ir::FactsCtx::from_validated(&facts),
            &budget,
        )
        .unwrap();
        nested_mapped.params[0] = plan
            .intern_nominal(
                wrapper,
                &[ir::Ty::Bool],
                ir::FactsCtx::from_validated(&facts),
                &budget,
            )
            .unwrap();
        assert!(
            substitution::replay_closed_signature(
                engine.authority(),
                &ir::Callee::Direct(nested_target),
                &nested_canonical,
                &nested_mapped,
                ir::FactsCtx::from_validated(&facts),
                &budget,
            )
            .is_err(),
            "nested source-slot equivalence accepted a changed actual"
        );
        let mut changed_contract = mapped.clone();
        let mut invalid_contract = changed_contract.ref_contract.as_ref().clone();
        invalid_contract.region_count = invalid_contract.region_count.checked_add(1).unwrap();
        changed_contract.ref_contract = Arc::new(invalid_contract);
        assert!(
            substitution::replay_closed_signature(
                engine.authority(),
                &target,
                &canonical,
                &changed_contract,
                ir::FactsCtx::from_validated(&facts),
                &budget,
            )
            .is_err(),
            "closed canonical contract adoption laundered invalid mapped contract"
        );
        let ReadNominal::Acyclic(acyclic) = capability.as_ref() else {
            panic!("expected acyclic fixture")
        };
        let mut changed_fact = acyclic.fact.as_ref().clone();
        let ir::NominalIndex::SourceSlot(index) = changed_fact.source_application.index else {
            panic!("source schema slot required")
        };
        changed_fact.source_application.index = ir::NominalIndex::SourceSlot(index + 1);
        let changed = ReadNominal::Acyclic(AcyclicRead {
            fact: Arc::new(changed_fact),
        });
        mapped.params[0] = changed.definition().unwrap();
        assert!(
            substitution::replay_closed_signature(
                engine.authority(),
                &target,
                &canonical,
                &mapped,
                ir::FactsCtx::from_validated(&facts),
                &budget
            )
            .is_err(),
            "closed receipt replay accepted changed source schema application"
        );
    }

    #[test]
    fn same_engine_typed_read_rejects_wrong_world_key_on_consumption() {
        let budget = ir::PlanBudget::default();
        let program = super::super::frontend::parse("struct Empty;fn main(){}").unwrap();
        let plan = super::super::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.nominal_declaration(0).unwrap();
        let proof = plan.publish_nominal_declaration(declaration).unwrap();
        let source = plan.nominal_realization_template(declaration).unwrap();
        let world = plan.trait_world().unwrap();
        let input = world.close_nominal_input(declaration, &proof, &[]).unwrap();
        // The key and successful output agree with each other, but not with
        // the genuine source proof's world. Typed read alone is insufficient.
        let key = RealizationKey::encode(
            world.nominal_realization_world().wrapping_add(1),
            declaration,
            &[],
            32,
        )
        .unwrap();
        let engine = crate::engine::Engine::new(FactProvider {
            key,
            source,
            proof,
            input,
            stolen: None,
            mutate: |_| {},
            fields: Arc::from([]),
        });
        let demand = engine.request(crate::compiler::keys::params(998, 0, vec![]));
        engine.run(1).unwrap();
        demand.completion().unwrap();
        let capability = demand.fact::<ReadNominal>(0).unwrap();
        let ty = capability.definition().unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        assert!(
            capability
                .retain_for_substitution(
                    &ty,
                    engine.authority(),
                    ir::FactsCtx::Validated(&facts),
                    &budget
                )
                .is_err()
        );
    }

    #[test]
    fn current_engine_parent_rejects_same_world_foreign_engine_child() {
        let budget = ir::PlanBudget::default();
        let program =
            super::super::frontend::parse("struct Empty;struct Parent{child:Empty}fn main(){}")
                .unwrap();
        let plan = super::super::frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let provider = |ordinal| {
            let declaration = plan.nominal_declaration(ordinal).unwrap();
            let proof = plan.publish_nominal_declaration(declaration).unwrap();
            let source = plan.nominal_realization_template(declaration).unwrap();
            let input = world.close_nominal_input(declaration, &proof, &[]).unwrap();
            let key =
                RealizationKey::encode(world.nominal_realization_world(), declaration, &[], 32)
                    .unwrap();
            FactProvider {
                key,
                source,
                proof,
                input,
                stolen: None,
                mutate: |_| {},
                fields: Arc::from([]),
            }
        };
        let foreign = crate::engine::Engine::new(provider(0));
        let child = foreign.request(crate::compiler::keys::params(998, 0, vec![]));
        foreign.run(1).unwrap();
        child.completion().unwrap();
        let child_capability = child.fact::<ReadNominal>(0).unwrap();
        let mut parent_provider = provider(1);
        let ir::Ty::Struct(source_parent) = &parent_provider.source else {
            panic!("parent struct required")
        };
        parent_provider.fields = Arc::from([FieldRealization {
            coordinate: FieldCoordinate {
                variant: None,
                field: 0,
            },
            source: source_parent.def().fields.read(&budget).unwrap()[0]
                .ty
                .clone(),
            normalized: child_capability.definition().unwrap(),
        }]);
        let current = crate::engine::Engine::new(parent_provider);
        let parent = current.request(crate::compiler::keys::params(998, 0, vec![]));
        current.run(1).unwrap();
        parent.completion().unwrap();
        let capability = parent.fact::<ReadNominal>(0).unwrap();
        let ty = capability.definition().unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        assert!(
            capability
                .retain_for_substitution(
                    &ty,
                    current.authority(),
                    ir::FactsCtx::Validated(&facts),
                    &budget
                )
                .is_err(),
            "parent's current-engine identity authorized foreign-engine child layout"
        );
    }

    struct IdentityProvider;
    impl crate::engine::Provider for IdentityProvider {
        fn outputs(&self, _: &Key) -> usize {
            1
        }
        fn start(&self, cx: Context, _: Key) -> crate::engine::TaskFuture {
            Box::pin(async move {
                cx.publish_fact(
                    0,
                    ir::NominalIndex::GraphRealization(cx.output_identity(0)?),
                )
            })
        }
    }

    #[test]
    fn graph_identity_never_falls_back_to_source_slot_or_foreign_engine() {
        let key = crate::compiler::keys::params(KIND, 1, vec![]);
        let first = crate::engine::Engine::new(IdentityProvider);
        let second = crate::engine::Engine::new(IdentityProvider);
        let _first_demand = first.request(key.clone());
        let _second_demand = second.request(key.clone());
        first.run(1).unwrap();
        second.run(1).unwrap();
        let a = first.fact_of::<ir::NominalIndex>(&key, 0).unwrap();
        let b = second.fact_of::<ir::NominalIndex>(&key, 0).unwrap();
        assert_ne!(a, b);
        assert!(a.source_slot().is_err());
        assert_ne!(*a, ir::NominalIndex::SourceSlot(0));
        assert_eq!(ir::NominalIndex::SourceSlot(0).source_slot().unwrap(), 0);
    }

    #[test]
    fn coordinates_preserve_order_and_never_accept_empty_terms() {
        let snapshot = ir::SnapshotId::standalone().unwrap();
        let declaration = ir::DeclId { snapshot, index: 3 };
        let first = RealizationKey::encode(7, declaration, &[vec![1], vec![2, 3]], 32).unwrap();
        let swapped = RealizationKey::encode(7, declaration, &[vec![2, 3], vec![1]], 32).unwrap();
        assert_ne!(first, swapped);
        assert!(RealizationKey::encode(7, declaration, &[vec![]], 32).is_err());
        assert!(RealizationKey::encode(7, declaration, &[vec![1; 32]], 32).is_err());
    }
}

impl crate::engine::Fact for NominalSourceFact {}

impl crate::engine::Fact for RealizedNominal {}

impl crate::engine::Fact for ReadNominal {}
