use super::{
    AcyclicRead, Context, Error, OutputIdentity, RealizationIdentity, RealizationKey,
    RealizedNominal, component, ir, materialization, topology, traits,
};
use std::sync::Arc;

/// A successful closed node publishes either its acyclic result or an alias
/// selecting a member of an already successfully read immutable component.
/// Alias output identities never stand in for a component's typed read.
#[derive(Clone)]
pub(super) enum NominalPublication {
    Acyclic(Arc<RealizedNominal>),
    Component {
        key: RealizationKey,
        identity: OutputIdentity,
        read: component::ReadComponent,
        member: u32,
    },
}
impl crate::engine::Fact for NominalPublication {}

#[derive(Clone)]
pub(in crate::compiler) enum ReadNominal {
    Acyclic(AcyclicRead),
    Component(materialization::RootedNominal),
}
impl ReadNominal {
    pub(in crate::compiler) fn application_key(&self) -> &RealizationKey {
        match self {
            Self::Acyclic(read) => &read.fact.key,
            Self::Component(read) => read.key(),
        }
    }
    pub(in crate::compiler) fn component_identity(&self) -> Option<OutputIdentity> {
        match self {
            Self::Acyclic(_) => None,
            Self::Component(read) => Some(read.identity()),
        }
    }
    pub(in crate::compiler) fn is_component(&self) -> bool {
        matches!(self, Self::Component(_))
    }
    pub(in crate::compiler) fn component_metadata_height(&self) -> Option<usize> {
        match self {
            Self::Component(read) => Some(read.metadata_height()),
            Self::Acyclic(_) => None,
        }
    }
    pub(in crate::compiler) fn component_reference_type_summary(
        &self,
        budget: &ir::PlanBudget,
        depth: usize,
        visits: &mut usize,
    ) -> Option<Result<(bool, usize), Error>> {
        match self {
            Self::Component(read) => Some(read.reference_type_summary(budget, depth, visits)),
            Self::Acyclic(_) => None,
        }
    }
    pub(in crate::compiler) fn component_slice_boundary_height(
        &self,
        allow_references: bool,
        allow_universal_projections: bool,
        budget: &ir::PlanBudget,
        depth: usize,
        work: &mut usize,
    ) -> Option<Result<usize, Error>> {
        match self {
            Self::Component(read) => Some(read.slice_boundary_height(
                allow_references,
                allow_universal_projections,
                budget,
                depth,
                work,
            )),
            Self::Acyclic(_) => None,
        }
    }
    pub(in crate::compiler) fn component_ownership_preflight(
        &self,
        budget: &ir::PlanBudget,
        depth: usize,
        event: &mut impl FnMut(usize, Option<(&ir::Ty, usize)>) -> Result<(), Error>,
    ) -> Option<Result<(), Error>> {
        match self {
            Self::Component(read) => Some(read.ownership_preflight(budget, depth, event)),
            Self::Acyclic(_) => None,
        }
    }
    pub(in crate::compiler) fn component_pointer_kind(&self) -> Option<ir::OwnedPointerKind> {
        match self {
            Self::Component(read) => read.pointer_kind(),
            Self::Acyclic(_) => None,
        }
    }
    pub(in crate::compiler) fn replay_component_dependency(
        &self,
        ty: &ir::Ty,
        world: &traits::ValidatedTraitWorld,
        depth: usize,
    ) -> Result<(), Error> {
        world.budget().check_depth(depth).map_err(Error::new)?;
        match self {
            Self::Component(read) => read.replay_at(ty, world, depth),
            Self::Acyclic(_) => self.replay(ty, world),
        }
    }
    pub(in crate::compiler) fn engine_authority(&self) -> crate::engine::EngineAuthority {
        match self {
            Self::Acyclic(read) => read.fact.identity.0.authority(),
            Self::Component(read) => read.identity().authority(),
        }
    }
    pub(in crate::compiler) fn definition(&self) -> Result<ir::Ty, Error> {
        match self {
            Self::Acyclic(read) => read.definition(),
            Self::Component(read) => read.definition(),
        }
    }
    pub(in crate::compiler) fn check_definition(&self, ty: &ir::Ty) -> Result<(), Error> {
        match self {
            Self::Acyclic(read) => read.check_definition(ty),
            Self::Component(read) => read.check(ty),
        }
    }
    pub(in crate::compiler) fn source_application(&self) -> ir::NominalAppId {
        match self {
            Self::Acyclic(read) => read.source_application(),
            Self::Component(read) => read.source_application(),
        }
    }
    pub(in crate::compiler) fn same_fact(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Acyclic(a), Self::Acyclic(b)) => a.same_fact(b),
            (Self::Component(a), Self::Component(b)) => a.same(b),
            _ => false,
        }
    }
    pub(in crate::compiler) fn replay(
        &self,
        ty: &ir::Ty,
        world: &traits::ValidatedTraitWorld,
    ) -> Result<(), Error> {
        self.replay_with(ty, world, &mut traits::NominalReplays::default())
    }
    pub(in crate::compiler) fn replay_with(
        &self,
        ty: &ir::Ty,
        world: &traits::ValidatedTraitWorld,
        replays: &mut traits::NominalReplays,
    ) -> Result<(), Error> {
        self.check_definition(ty)?;
        if replays.contains_fact(self) {
            return Ok(());
        }
        match self {
            Self::Acyclic(read) => read.replay_uncached(world, replays)?,
            Self::Component(read) => read.replay(ty, world)?,
        }
        // Only a COMPLETELY validated component is recorded. Internal members
        // never borrow success from an in-progress replay marker.
        replays
            .record_fact(self, world.budget())
            .map_err(|e| Error::new(e.into_legacy_diagnostic(world.budget())))
    }
    pub(in crate::compiler) fn retain_for_substitution(
        &self,
        ty: &ir::Ty,
        engine: crate::engine::EngineAuthority,
        caller: ir::FactsCtx<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<ir::Ty, String> {
        if let Self::Acyclic(read) = self {
            return read.retain_for_substitution(ty, engine, caller, budget);
        }
        if self.engine_authority() != engine {
            return Err("foreign engine recursive nominal substitution capability".into());
        }
        let facts = caller
            .validated()
            .ok_or("recursive nominal substitution requires genuine caller facts")?;
        if !facts.budget().same_authority(budget) {
            return Err("recursive nominal substitution budget authority mismatch".into());
        }
        // Component replay checks the entire finite symbolic source inventory
        // and every successful external receipt under the same engine/world.
        self.replay(ty, facts.world()).map_err(|e| e.to_string())?;
        caller.validate_ty_with_budget(ty, budget)?;
        budget.retain_bytes(std::mem::size_of::<ir::Ty>())?;
        Ok(ty.clone())
    }
}

pub(in crate::compiler) async fn read_manifest(
    cx: &Context,
    keys: &[RealizationKey],
    budget: &ir::PlanBudget,
) -> Result<Vec<ReadNominal>, Error> {
    let mut pending = topology::storage(keys.len(), budget)?;
    for key in keys {
        let handle = cx.request(key.0.clone()).get(0);
        let identity = handle.identity()?;
        pending.push((key, identity, handle));
    }
    let mut capabilities = topology::storage(pending.len(), budget)?;
    for (key, identity, handle) in pending {
        let publication: Arc<NominalPublication> = cx.read_fact(handle).await?;
        let read = match publication.as_ref() {
            NominalPublication::Acyclic(fact) => {
                if &fact.key != key || fact.identity != RealizationIdentity(identity) {
                    return Err(Error::new(
                        "nominal realization typed read identity mismatch",
                    ));
                }
                ReadNominal::Acyclic(AcyclicRead { fact: fact.clone() })
            }
            NominalPublication::Component {
                key: alias,
                identity: alias_identity,
                read,
                member,
            } => {
                if alias != key
                    || *alias_identity != identity
                    || read.member_key(*member)? != key
                    || read.identity().authority() != identity.authority()
                {
                    return Err(Error::new(
                        "nominal component alias typed read identity mismatch",
                    ));
                }
                // Only after the actual successful component read may any
                // executable shell or field projection be materialized.
                ReadNominal::Component(read.materialize(*member, budget)?)
            }
        };
        capabilities.push(read);
    }
    Ok(capabilities)
}
