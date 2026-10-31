//! Immutable SCC publication and genuine typed component reads.
//! Candidate symbols remain distinct from executable, rooted nominal shells.
use super::{
    Context, Error, FieldCoordinate, Key, OutputIdentity, ReadNominal, ReadSource, RealizationKey,
    component_key::{ComponentKey, application_words, copy_application},
    coordinates::Term,
    ir, producer, symbolic, topology, traits,
};
use std::sync::Arc;

#[derive(Clone)]
struct Member {
    key: RealizationKey,
    source: ReadSource,
    actuals: Arc<[Term]>,
    fields: Arc<[(FieldCoordinate, Term)]>,
}

#[derive(Clone)]
struct ComponentFact {
    key: ComponentKey,
    identity: OutputIdentity,
    members: Arc<[Member]>,
    external: Arc<[(RealizationKey, ReadNominal)]>,
    layout_order: Arc<[usize]>,
    metadata_heights: Arc<[usize]>,
    pointer_kinds: Arc<[Option<ir::OwnedPointerKind>]>,
}
impl crate::engine::Fact for ComponentFact {}

#[cfg(test)]
mod tests;

/// Constructible only by an exact, successful typed graph read below. Neither
/// source identities nor symbolic coordinates can manufacture this capability.
#[derive(Clone)]
pub(in crate::compiler) struct ReadComponent {
    fact: Arc<ComponentFact>,
}
impl std::fmt::Debug for ReadComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ReadComponent")
            .field(&self.fact.identity)
            .finish()
    }
}

fn member_keys(fact: &ComponentFact, b: &ir::PlanBudget) -> Result<Vec<RealizationKey>, Error> {
    let mut keys = topology::storage(fact.members.len(), b)?;
    for member in fact.members.iter() {
        keys.push(copy_application(&member.key, b)?);
    }
    Ok(keys)
}

/// One fully paid edge buffer, with no growth or overlapping relocation.
/// The completed vector escapes into the replay's application inventory, so
/// its original topology storage charge remains retained, never refunded.
struct ReplayEdges {
    edges: Vec<topology::Edge>,
    count: usize,
}
impl ReplayEdges {
    fn new(count: usize, b: &ir::PlanBudget) -> Result<Self, Error> {
        Ok(Self {
            edges: topology::storage(count, b)?,
            count,
        })
    }
    fn push(&mut self, edge: topology::Edge, b: &ir::PlanBudget) -> Result<(), Error> {
        b.charge_work(1).map_err(Error::new)?;
        if self.edges.len() >= self.count || self.edges.capacity() < self.count {
            return Err(Error::new("component dependency count changed"));
        }
        self.edges.push(edge);
        Ok(())
    }
    fn finish(self) -> Result<Vec<topology::Edge>, Error> {
        if self.edges.len() != self.count {
            return Err(Error::new("component dependency count changed"));
        }
        Ok(self.edges)
    }
}

/// Reconstruct every dependency and size equation from the retained typed
/// source proof. No optimistic visited-member success and no field IR exist
/// during this validation. External dependencies must already be real reads.
fn validate(
    fact: &ComponentFact,
    world: &traits::ValidatedTraitWorld,
    depth: usize,
) -> Result<(), Error> {
    let b = world.budget();
    b.check_depth(depth).map_err(Error::new)?;
    b.charge_work(1).map_err(Error::new)?;
    let error = |e: traits::FactsError| Error::new(e.into_legacy_diagnostic(b));
    let keys = member_keys(fact, b)?;
    let expected = ComponentKey::encode(world.nominal_realization_world(), &keys, b)?;
    if fact.key != expected || keys.windows(2).any(|w| w[0].0.params >= w[1].0.params) {
        return Err(Error::new("component key, world or member order changed"));
    }
    let application_count = fact
        .members
        .len()
        .checked_add(fact.external.len())
        .ok_or_else(|| Error::new("component application count overflow"))?;
    let mut apps = topology::storage(application_count, b)?;
    for member in fact.members.iter() {
        apps.push(topology::Application {
            key: topology::ApplicationWords(application_words(&member.key, b)?),
            edges: Vec::new(),
        });
    }
    for (key, read) in fact.external.iter() {
        if keys.contains(key)
            || read.application_key() != key
            || read.component_identity() == Some(fact.identity)
            || read.engine_authority() != fact.identity.authority()
        {
            return Err(Error::new(
                "component external receipt aliases an internal member",
            ));
        }
        let ty = read.definition()?;
        read.replay_component_dependency(&ty, world, depth + 1)?;
        world.require_symbolic_external_sized(&ty).map_err(error)?;
        apps.push(topology::Application {
            key: topology::ApplicationWords(application_words(key, b)?),
            edges: Vec::new(),
        });
    }
    if fact
        .external
        .windows(2)
        .any(|w| w[0].0.0.params >= w[1].0.0.params)
    {
        return Err(Error::new("component external receipt order changed"));
    }
    let mut used = topology::storage(fact.external.len(), b)?;
    used.resize(fact.external.len(), false);
    if fact.metadata_heights.len() != fact.members.len()
        || fact.pointer_kinds.len() != fact.members.len()
    {
        return Err(Error::new("component metadata witness omitted members"));
    }
    for (index, member) in fact.members.iter().enumerate() {
        let (member_world, declaration, actuals) = producer::decode(&member.key.0, b)?;
        if member_world != world.nominal_realization_world()
            || member.source.fact.identity.authority() != fact.identity.authority()
            || actuals.as_slice() != member.actuals.as_ref()
            || declaration != member.source.declaration()
        {
            return Err(Error::new("component member application changed"));
        }
        let requirements = world
            .symbolic_nominal_requirements(
                declaration,
                member.source.proof(),
                member.source.template(),
            )
            .map_err(error)?;
        let env = &member.source.proof().schema().env;
        if fact.metadata_heights[index] != member_height(&member.actuals, &member.fields, b)?
            || fact.pointer_kinds[index] != pointer_kind(member, b)?
        {
            return Err(Error::new("component compressed metadata witness changed"));
        }
        if actuals.len() != env.total_arity() {
            return Err(Error::new("component actual arity changed"));
        }
        for actual in &actuals {
            metadata(actual, b, 0)?;
        }
        // Only the edge inventory survives this replay. Borrow each exact
        // source actual list while encoding its key, never retain temporary
        // cloned dependency terms as if they were published payloads.
        // A charged count pass preserves the original single paid edge
        // allocation. Closing requirements twice is deliberate paid work,
        // not an unmetered clone inventory or incremental buffer growth.
        let mut edge_count = 0usize;
        let mut count = |_: ir::DeclId, _: &[Term], _: bool| -> Result<(), Error> {
            edge_count = edge_count
                .checked_add(1)
                .ok_or_else(|| Error::new("component dependency count overflow"))?;
            Ok(())
        };
        for requirement in &requirements {
            let term = symbolic::close_term(requirement, env, &actuals, b, 0)?;
            symbolic::visit_dependencies(&term, false, b, 0, &mut count)?;
        }
        for actual in &actuals {
            symbolic::visit_dependencies(actual, false, b, 0, &mut count)?;
        }
        for (_, retained) in member.fields.iter() {
            symbolic::visit_dependencies(retained, true, b, 0, &mut count)?;
        }
        let mut edges = ReplayEdges::new(edge_count, b)?;
        let mut edge = |declaration, actuals: &[Term], by_value| -> Result<(), Error> {
            let key = producer::key(member_world, declaration, actuals, b)?;
            let mut target = keys.iter().position(|k| k == &key);
            if target.is_none()
                && let Some(external) = fact.external.iter().position(|(k, _)| k == &key)
            {
                used[external] = true;
                target = Some(fact.members.len() + external);
            }
            let target = target
                .ok_or_else(|| Error::new("component omitted exact nominal dependency receipt"))?;
            edges.push(topology::Edge { target, by_value }, b)
        };
        for requirement in &requirements {
            let term = symbolic::close_term(requirement, env, &actuals, b, 0)?;
            metadata(&term, b, 0)?;
            sized(&term, b, 0)?;
            symbolic::visit_dependencies(&term, false, b, 0, &mut edge)?;
        }
        let original = producer::source_fields(member.source.template(), b)?;
        if original.len() != member.fields.len() {
            return Err(Error::new("component field inventory changed"));
        }
        for actual in &actuals {
            symbolic::visit_dependencies(actual, false, b, 0, &mut edge)?;
        }
        for ((coordinate, source), (retained_coordinate, retained)) in
            original.iter().zip(member.fields.iter())
        {
            let expected = symbolic::close_term(source, env, &actuals, b, 0)?;
            if coordinate != retained_coordinate || &expected != retained {
                return Err(Error::new("component symbolic source field changed"));
            }
            metadata(retained, b, 0)?;
            sized(retained, b, 0)?;
            symbolic::visit_dependencies(retained, true, b, 0, &mut edge)?;
        }
        apps[index].edges = edges.finish()?;
    }
    if used.iter().any(|used| !used) {
        return Err(Error::new("component includes unrelated external receipt"));
    }
    let groups = topology::components(&apps, b)?;
    let own = groups
        .iter()
        .find(|group| group.members.contains(&0))
        .ok_or_else(|| Error::new("component has no canonical first member"))?;
    if own.members.len() != fact.members.len()
        || own.members.iter().copied().ne(0..fact.members.len())
    {
        return Err(Error::new(
            "component membership or finite layout witness changed",
        ));
    }
    let mut position = topology::storage(fact.members.len(), b)?;
    position.resize(fact.members.len(), usize::MAX);
    if fact.layout_order.len() != fact.members.len() {
        return Err(Error::new("component layout order omitted members"));
    }
    for (ordinal, &member) in fact.layout_order.iter().enumerate() {
        let slot = position
            .get_mut(member)
            .ok_or_else(|| Error::new("component layout ordinal out of bounds"))?;
        if *slot != usize::MAX {
            return Err(Error::new("component layout order repeats member"));
        }
        *slot = ordinal;
    }
    for (member, app) in apps[..fact.members.len()].iter().enumerate() {
        for edge in &app.edges {
            b.charge_work(1).map_err(Error::new)?;
            if edge.by_value
                && edge.target < fact.members.len()
                && position[edge.target] >= position[member]
            {
                return Err(Error::new("component layout order is not dependency-first"));
            }
        }
    }
    Ok(())
}

/// Initial recursive scope has only Sized nominal applications. Unsized
/// recursive equations remain explicit unsupported cases, never thin-pointer
/// guesses. Nonrecursive producers retain their existing DST support.
fn metadata(term: &Term, b: &ir::PlanBudget, depth: usize) -> Result<(), Error> {
    b.check_depth(depth).map_err(Error::new)?;
    b.charge_work(1).map_err(Error::new)?;
    match term {
        Term::Str | Term::Slice(_) | Term::Dyn(..) => {
            return Err(Error::new(
                "unsupported: recursive component unsized metadata equations",
            ));
        }
        Term::Ref(_, t) | Term::RawPtr(_, t) => metadata(t, b, depth + 1)?,
        Term::PhantomData(_, t) => {
            metadata(t, b, depth + 1)?;
            return Err(Error::new(
                "unsupported: recursive marker components require sealed catalog construction authority",
            ));
        }
        Term::Array(t, len) => {
            if *len > ir::MAX_ARRAY_TYPE_LEN {
                return Err(Error::new("array length limit exceeded"));
            }
            metadata(t, b, depth + 1)?;
        }
        Term::FnPointer(inputs, output) => {
            if inputs.len() > 32 {
                return Err(Error::new("function pointer parameter limit exceeded"));
            }
            for t in inputs.iter().chain(std::iter::once(output.as_ref())) {
                metadata(t, b, depth + 1)?;
            }
        }
        Term::Tuple(types) if types.is_empty() => {
            return Err(Error::new("empty tuple must have canonical Unit type"));
        }
        Term::Tuple(types) | Term::Nominal(_, types) => {
            for ty in types {
                metadata(ty, b, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn sized(term: &Term, b: &ir::PlanBudget, depth: usize) -> Result<(), Error> {
    b.check_depth(depth).map_err(Error::new)?;
    b.charge_work(1).map_err(Error::new)?;
    match term {
        Term::Str | Term::Slice(_) | Term::Dyn(..) => {
            return Err(Error::new(
                "invalid: unsized component field or Sized obligation",
            ));
        }
        Term::Array(t, _) => sized(t, b, depth + 1)?,
        Term::Tuple(types) => {
            for ty in types {
                sized(ty, b, depth + 1)?;
            }
        }
        // Nominal equations are all validated together in dependency-first
        // order below. A nominal type argument alone is not a size edge.
        _ => {}
    }
    Ok(())
}

fn pointer_kind(
    member: &Member,
    b: &ir::PlanBudget,
) -> Result<Option<ir::OwnedPointerKind>, Error> {
    let ir::Ty::Struct(source) = member.source.template() else {
        return Ok(None);
    };
    let [actual] = member.actuals.as_ref() else {
        return Ok(None);
    };
    let original = source.def().fields.read(b).map_err(Error::new)?;
    let Some((first, term)) = member.fields.first() else {
        return Ok(None);
    };
    if *first
        != (FieldCoordinate {
            variant: None,
            field: 0,
        })
        || original.first().is_none_or(|field| field.name != "ptr")
        || !matches!(term, Term::RawPtr(ir::BorrowKind::Mut, pointee) if pointee.as_ref() == actual)
    {
        return Ok(None);
    }
    // The name is the existing reserved ownership-protocol identity only.
    // Size propagation was independently derived from every raw field term.
    match (
        source.def().name.as_str(),
        original.len(),
        member.fields.as_ref(),
    ) {
        ("crate::__justrust_std::boxed::Box", 1, [_]) => Ok(Some(ir::OwnedPointerKind::Box)),
        (
            "crate::__justrust_std::rc::Rc",
            2,
            [_, (coordinate, Term::RawPtr(ir::BorrowKind::Mut, count))],
        ) if *coordinate
            == (FieldCoordinate {
                variant: None,
                field: 1,
            })
            && original[1].name == "strong"
            && count.as_ref() == &Term::Int(ir::IntTy::Usize) =>
        {
            Ok(Some(ir::OwnedPointerKind::Rc))
        }
        _ => Ok(None),
    }
}

fn term_height(term: &Term, b: &ir::PlanBudget, depth: usize) -> Result<usize, Error> {
    b.check_depth(depth).map_err(Error::new)?;
    b.charge_work(1).map_err(Error::new)?;
    Ok(match term {
        Term::Ref(_, t)
        | Term::RawPtr(_, t)
        | Term::Array(t, _)
        | Term::Slice(t)
        | Term::PhantomData(_, t) => term_height(t, b, depth + 1)?.saturating_add(1),
        Term::FnPointer(inputs, output) => {
            let mut height = 0;
            for t in inputs.iter().chain(std::iter::once(output.as_ref())) {
                height = height.max(term_height(t, b, depth + 1)?.saturating_add(1));
            }
            height
        }
        Term::Tuple(types) | Term::Nominal(_, types) => {
            let mut height = 0;
            for term in types {
                height = height.max(term_height(term, b, depth + 1)?.saturating_add(1));
            }
            height
        }
        Term::Dyn(..) => return Err(Error::new("unsupported: recursive dynamic metadata height")),
        _ => 0,
    })
}
fn member_height(
    actuals: &[Term],
    fields: &[(FieldCoordinate, Term)],
    b: &ir::PlanBudget,
) -> Result<usize, Error> {
    let mut height = 0;
    for term in actuals.iter().chain(fields.iter().map(|(_, term)| term)) {
        height = height.max(term_height(term, b, 0)?.saturating_add(1));
    }
    Ok(height)
}

pub(super) async fn run(cx: &Context, raw: Key) -> Result<(), Error> {
    let fact = produce(cx, raw).await?;
    cx.publish_fact(0, fact)
}

async fn produce(cx: &Context, raw: Key) -> Result<ComponentFact, Error> {
    let handle = cx
        .request(crate::compiler::keys::SignaturesDebtRoot.key())
        .get(0);
    let types: Arc<super::super::frontend::PlanCtx> = cx.read_fact(handle).await?;
    let world = types.trait_world().map_err(Error::new)?;
    let b = world.budget();
    let (key, world_word, requested) = ComponentKey::decode(&raw, b)?;
    if world_word != world.nominal_realization_world() {
        return Err(Error::new("foreign component world"));
    }
    let discovered = symbolic::discover(cx, copy_application(&requested[0], b)?, b, &world).await?;
    let group = discovered
        .components
        .iter()
        .find(|group| group.members.contains(&0))
        .ok_or_else(|| Error::new("component discovery omitted canonical root"))?;
    let mut keys = topology::storage(group.members.len(), b)?;
    for &index in &group.members {
        keys.push(copy_application(&discovered.members[index].key, b)?);
    }
    if keys != requested {
        return Err(Error::new(
            "component key is not complete canonical SCC membership",
        ));
    }
    let mut external_keys = topology::storage(group.external.len(), b)?;
    for &index in &group.external {
        external_keys.push(copy_application(&discovered.members[index].key, b)?);
    }
    // Entire external frontier is requested before awaiting any result.
    let external_reads = super::read_manifest(cx, &external_keys, b).await?;
    let mut external = topology::storage(external_keys.len(), b)?;
    external.extend(external_keys.into_iter().zip(external_reads));
    let mut members = topology::storage(group.members.len(), b)?;
    for &index in &group.members {
        let candidate = &discovered.members[index];
        let mut actuals = topology::storage(candidate.actuals.len(), b)?;
        for term in &candidate.actuals {
            actuals.push(Term::decode(&term.encode(b)?, b)?);
        }
        let mut fields = topology::storage(candidate.fields.len(), b)?;
        for (coordinate, term) in &candidate.fields {
            fields.push((*coordinate, Term::decode(&term.encode(b)?, b)?));
        }
        members.push(Member {
            key: copy_application(&candidate.key, b)?,
            source: candidate.source.clone(),
            actuals: actuals.into(),
            fields: fields.into(),
        });
    }
    // Recompute the component-local layout order. Global discovery indices
    // must never escape as member coordinates or proof identities.
    let mut order = topology::storage(group.layout_order.len(), b)?;
    for global in &group.layout_order {
        order.push(
            group
                .members
                .iter()
                .position(|index| index == global)
                .ok_or_else(|| Error::new("layout order leaves component"))?,
        );
    }
    let mut heights = topology::storage(members.len(), b)?;
    let mut pointers = topology::storage(members.len(), b)?;
    for member in &members {
        heights.push(member_height(&member.actuals, &member.fields, b)?);
        pointers.push(pointer_kind(member, b)?);
    }
    let fact = ComponentFact {
        key,
        identity: cx.output_identity(0)?,
        members: members.into(),
        external: external.into(),
        layout_order: order.into(),
        metadata_heights: heights.into(),
        pointer_kinds: pointers.into(),
    };
    // Complete source, predicate, membership, pointer and size validation is
    // mandatory before publication. This is not an executable nominal read.
    validate(&fact, &world, 0)?;
    Ok(fact)
}

pub(super) async fn read(
    cx: &Context,
    key: &ComponentKey,
    world: &traits::ValidatedTraitWorld,
) -> Result<ReadComponent, Error> {
    let handle = cx.request(key.0.clone()).get(0);
    let identity = handle.identity()?;
    let fact: Arc<ComponentFact> = cx.read_fact(handle).await?;
    if fact.key != *key || fact.identity != identity {
        return Err(Error::new("component typed read identity mismatch"));
    }
    validate(&fact, world, 0)?;
    Ok(ReadComponent { fact })
}

/// A complete finite carrier-policy traversal. Pending members are equations
/// still to inspect, never provisional successful subtrees.
struct SliceInventory<'a> {
    read: &'a ReadComponent,
    budget: &'a ir::PlanBudget,
    work: &'a mut usize,
    allow_references: bool,
    allow_universal_projections: bool,
    queued: Vec<bool>,
    pending: Vec<usize>,
}
impl SliceInventory<'_> {
    fn charge(&mut self) -> Result<(), Error> {
        self.budget.charge_work(1).map_err(Error::new)?;
        *self.work = self
            .work
            .checked_add(1)
            .ok_or_else(|| Error::new("slice type work overflow"))?;
        if *self.work > ir::MAX_PATTERN_PROJECTION_WORK {
            return Err(Error::new("slice type resource limit exceeded"));
        }
        Ok(())
    }
    fn term(&mut self, term: &Term, depth: usize) -> Result<usize, Error> {
        let b = self.budget;
        b.check_depth(depth).map_err(Error::new)?;
        self.charge()?;
        let mut height = 0usize;
        match term {
            Term::Ref(_, child) => {
                if !self.allow_references {
                    return Err(Error::new(
                        "unsupported: slice elements containing references or slices, or nested slice carriers",
                    ));
                }
                height = self
                    .term(child, depth + 1)?
                    .checked_add(1)
                    .ok_or_else(|| Error::new("slice type depth overflow"))?;
            }
            Term::RawPtr(_, child) => {
                // The ordinary boundary walker checks all pointee metadata for
                // slices, then treats raw storage as a leaf. In particular,
                // ReferenceFree does not reject references behind raw storage.
                let pointee = self.read.resolve(child, b, 0)?;
                if pointee.contains_slice(b).map_err(Error::new)? {
                    return Err(Error::new(
                        "unsupported: raw storage of slice carriers requires heap lifetime retention",
                    ));
                }
            }
            Term::Array(child, _) => {
                height = self
                    .term(child, depth + 1)?
                    .checked_add(1)
                    .ok_or_else(|| Error::new("slice type depth overflow"))?;
            }
            Term::Tuple(children) | Term::Nominal(_, children) => {
                for child in children {
                    height = height.max(
                        self.term(child, depth + 1)?
                            .checked_add(1)
                            .ok_or_else(|| Error::new("slice type depth overflow"))?,
                    );
                }
                if let Term::Nominal(declaration, actuals) = term {
                    let key = producer::key(self.read.world_word(), *declaration, actuals, b)?;
                    let mut target = None;
                    let read = self.read;
                    for (index, member) in read.fact.members.iter().enumerate() {
                        self.charge()?;
                        if member.key == key {
                            target = Some(index);
                            break;
                        }
                    }
                    if let Some(target) = target {
                        if !self.queued[target] {
                            self.queued[target] = true;
                            self.pending.push(target);
                        }
                    } else {
                        b.charge_work(self.read.fact.external.len())
                            .map_err(Error::new)?;
                        let external = self.read.external(&key).ok_or_else(|| {
                            Error::new("slice inventory omitted nominal dependency")
                        })?;
                        let ty = external.definition()?;
                        external.check_definition(&ty)?;
                        let external_height = ir::slice_boundary_height(
                            &ty,
                            self.allow_references,
                            self.allow_universal_projections,
                            b,
                            depth,
                            self.work,
                        )
                        .map_err(Error::new)?;
                        height = height.max(external_height);
                    }
                }
            }
            // Such terms cannot occur in a successful closed component read.
            Term::Str | Term::Slice(_) | Term::Dyn(..) => {
                return Err(Error::new(
                    "unsupported: recursive component unsized metadata equations",
                ));
            }
            Term::PhantomData(..)
            | Term::FnPointer(..)
            | Term::Int(_)
            | Term::Float(_)
            | Term::Bool
            | Term::Unit
            | Term::Never
            | Term::Char => {}
        }
        Ok(height)
    }
}
impl ReadComponent {
    pub(super) fn slice_boundary_height(
        &self,
        member: u32,
        allow_references: bool,
        allow_universal_projections: bool,
        b: &ir::PlanBudget,
        depth: usize,
        work: &mut usize,
    ) -> Result<usize, Error> {
        let root =
            usize::try_from(member).map_err(|_| Error::new("component member ordinal overflow"))?;
        if root >= self.fact.members.len() {
            return Err(Error::new("component member ordinal out of bounds"));
        }
        let mut queued = topology::storage(self.fact.members.len(), b)?;
        queued.resize(self.fact.members.len(), false);
        let mut pending = topology::storage(self.fact.members.len(), b)?;
        queued[root] = true;
        pending.push(root);
        let mut inventory = SliceInventory {
            read: self,
            budget: b,
            work,
            allow_references,
            allow_universal_projections,
            queued,
            pending,
        };
        let mut height = 0usize;
        while let Some(index) = inventory.pending.pop() {
            inventory.charge()?;
            let member = &self.fact.members[index];
            for term in member
                .actuals
                .iter()
                .chain(member.fields.iter().map(|(_, term)| term))
            {
                height = height.max(
                    inventory
                        .term(term, depth + 1)?
                        .checked_add(1)
                        .ok_or_else(|| Error::new("slice type depth overflow"))?,
                );
            }
        }
        b.check_depth(
            depth
                .checked_add(height)
                .ok_or_else(|| Error::new("slice type depth overflow"))?,
        )
        .map_err(Error::new)?;
        Ok(height)
    }
}

/// Complete reference-contract inventory over the finite symbolic component.
/// The queue records pending equations, not provisional successful answers.
/// No result escapes until every reachable member and external read is scanned.
struct ReferenceInventory<'a> {
    read: &'a ReadComponent,
    budget: &'a ir::PlanBudget,
    visits: &'a mut usize,
    queued: Vec<bool>,
    pending: Vec<usize>,
    opaque: bool,
}
impl ReferenceInventory<'_> {
    fn term(&mut self, term: &Term, depth: usize, reference_layers: usize) -> Result<usize, Error> {
        let b = self.budget;
        b.check_depth(depth).map_err(Error::new)?;
        b.charge_work(1).map_err(Error::new)?;
        *self.visits = self
            .visits
            .checked_add(1)
            .ok_or_else(|| Error::new("reference type visit overflow"))?;
        if *self.visits > ir::MAX_PATTERN_PROJECTION_WORK {
            return Err(Error::new("reference type work limit exceeded"));
        }
        let mut height = 0usize;
        match term {
            Term::Ref(_, child) => {
                let layers = reference_layers
                    .checked_add(1)
                    .ok_or_else(|| Error::new("reference depth overflow"))?;
                if layers > 128 {
                    return Err(Error::new("reference depth resource limit exceeded"));
                }
                height = self
                    .term(child, depth + 1, layers)?
                    .checked_add(1)
                    .ok_or_else(|| Error::new("reference type depth overflow"))?;
            }
            Term::RawPtr(_, child) => {
                self.opaque = true;
                height = self
                    .term(child, depth + 1, 0)?
                    .checked_add(1)
                    .ok_or_else(|| Error::new("reference type depth overflow"))?;
            }
            Term::Array(child, len) => {
                if *len > ir::MAX_ARRAY_TYPE_LEN {
                    return Err(Error::new("array length resource limit exceeded"));
                }
                height = self
                    .term(child, depth + 1, 0)?
                    .checked_add(1)
                    .ok_or_else(|| Error::new("reference type depth overflow"))?;
            }
            Term::Tuple(children) | Term::Nominal(_, children) => {
                for child in children {
                    height = height.max(
                        self.term(child, depth + 1, 0)?
                            .checked_add(1)
                            .ok_or_else(|| Error::new("reference type depth overflow"))?,
                    );
                }
                if let Term::Nominal(declaration, actuals) = term {
                    let key = producer::key(self.read.world_word(), *declaration, actuals, b)?;
                    let mut target = None;
                    for (index, member) in self.read.fact.members.iter().enumerate() {
                        b.charge_work(1).map_err(Error::new)?;
                        if member.key == key {
                            target = Some(index);
                            break;
                        }
                    }
                    if let Some(target) = target {
                        if !self.queued[target] {
                            self.queued[target] = true;
                            self.pending.push(target);
                        }
                    } else {
                        b.charge_work(self.read.fact.external.len())
                            .map_err(Error::new)?;
                        let external = self.read.external(&key).ok_or_else(|| {
                            Error::new("reference inventory omitted nominal dependency")
                        })?;
                        let ty = external.definition()?;
                        external.check_definition(&ty)?;
                        let (opaque, external_height) =
                            ir::reference_type_summary(&ty, true, b, depth, self.visits)
                                .map_err(Error::new)?;
                        self.opaque |= opaque;
                        height = height.max(external_height);
                    }
                }
            }
            Term::Str | Term::Slice(_) | Term::Dyn(..) => {
                return Err(Error::new(
                    "unsupported: recursive component unsized metadata equations",
                ));
            }
            Term::PhantomData(..)
            | Term::FnPointer(..)
            | Term::Int(_)
            | Term::Float(_)
            | Term::Bool
            | Term::Unit
            | Term::Never
            | Term::Char => {}
        }
        Ok(height)
    }
}
impl ReadComponent {
    pub(super) fn reference_type_summary(
        &self,
        member: u32,
        b: &ir::PlanBudget,
        depth: usize,
        visits: &mut usize,
    ) -> Result<(bool, usize), Error> {
        let root =
            usize::try_from(member).map_err(|_| Error::new("component member ordinal overflow"))?;
        if root >= self.fact.members.len() {
            return Err(Error::new("component member ordinal out of bounds"));
        }
        let mut queued = topology::storage(self.fact.members.len(), b)?;
        queued.resize(self.fact.members.len(), false);
        let mut pending = topology::storage(self.fact.members.len(), b)?;
        pending.push(root);
        queued[root] = true;
        let mut inventory = ReferenceInventory {
            read: self,
            budget: b,
            visits,
            queued,
            pending,
            opaque: false,
        };
        let mut height = 0usize;
        while let Some(index) = inventory.pending.pop() {
            b.charge_work(1).map_err(Error::new)?;
            let member = &self.fact.members[index];
            for term in member
                .actuals
                .iter()
                .chain(member.fields.iter().map(|(_, term)| term))
            {
                height = height.max(
                    inventory
                        .term(term, depth + 1, 0)?
                        .checked_add(1)
                        .ok_or_else(|| Error::new("reference type depth overflow"))?,
                );
            }
        }
        let deepest = depth
            .checked_add(height)
            .ok_or_else(|| Error::new("reference type depth overflow"))?;
        b.check_depth(deepest).map_err(Error::new)?;
        Ok((inventory.opaque, height))
    }
}

impl ReadComponent {
    pub(super) fn identity(&self) -> OutputIdentity {
        self.fact.identity
    }
    pub(super) fn metadata_height(&self, member: u32) -> usize {
        self.fact.metadata_heights[member as usize]
    }
    pub(super) fn pointer_kind(&self, member: u32) -> Option<ir::OwnedPointerKind> {
        self.fact.pointer_kinds[member as usize]
    }
    pub(super) fn world_word(&self) -> u64 {
        self.fact.key.0.params[2]
    }
    pub(super) fn find_member(&self, key: &RealizationKey) -> Option<u32> {
        self.fact
            .members
            .iter()
            .position(|member| &member.key == key)
            .and_then(|index| u32::try_from(index).ok())
    }
    pub(super) fn external(&self, key: &RealizationKey) -> Option<&ReadNominal> {
        self.fact
            .external
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, read)| read)
    }
    pub(super) fn source(&self, member: u32) -> Result<&ir::Ty, Error> {
        self.fact
            .members
            .get(member as usize)
            .map(|member| member.source.template())
            .ok_or_else(|| Error::new("component source ordinal out of bounds"))
    }
    pub(super) fn actuals(&self, member: u32) -> Result<&[Term], Error> {
        self.fact
            .members
            .get(member as usize)
            .map(|member| member.actuals.as_ref())
            .ok_or_else(|| Error::new("component actual ordinal out of bounds"))
    }
    pub(super) fn field_term(
        &self,
        member: u32,
        coordinate: FieldCoordinate,
    ) -> Result<&Term, Error> {
        self.fact
            .members
            .get(member as usize)
            .and_then(|member| member.fields.iter().find(|(field, _)| *field == coordinate))
            .map(|(_, term)| term)
            .ok_or_else(|| Error::new("component field coordinate out of bounds"))
    }
    pub(super) fn member_for(&self, key: &RealizationKey) -> Result<u32, Error> {
        let index = self
            .fact
            .members
            .iter()
            .position(|member| &member.key == key)
            .ok_or_else(|| Error::new("component read does not contain exact application"))?;
        u32::try_from(index).map_err(|_| Error::new("component member ordinal overflow"))
    }
    pub(super) fn member_key(&self, member: u32) -> Result<&RealizationKey, Error> {
        self.fact
            .members
            .get(member as usize)
            .map(|member| &member.key)
            .ok_or_else(|| Error::new("component member ordinal out of bounds"))
    }
    pub(super) fn replay(&self, world: &traits::ValidatedTraitWorld) -> Result<(), Error> {
        validate(&self.fact, world, 0)
    }
    pub(super) fn replay_at(
        &self,
        world: &traits::ValidatedTraitWorld,
        depth: usize,
    ) -> Result<(), Error> {
        validate(&self.fact, world, depth)
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.fact, &other.fact)
    }
}

/// A finite drop query over an already authenticated component. Queue
/// membership only schedules equations. Zero arrays suppress drop contribution,
/// never child validation, unresolved status, or failure. Each member has at
/// most two scheduled modes and no answer escapes before all modes finish.
struct DropPresenceInventory<'a, F> {
    read: &'a ReadComponent,
    budget: &'a ir::PlanBudget,
    event: &'a mut F,
    queued: Vec<[bool; 2]>,
    pending: Vec<(usize, bool)>,
    required: bool,
    unresolved: bool,
}
impl<F: FnMut(&ir::Ty, bool, usize) -> Result<(bool, bool), Error>> DropPresenceInventory<'_, F> {
    fn term(&mut self, term: &Term, depth: usize, contributes: bool) -> Result<(), Error> {
        self.budget.check_depth(depth).map_err(Error::new)?;
        self.budget.charge_work(1).map_err(Error::new)?;
        match term {
            Term::Ref(..) | Term::RawPtr(..) => {}
            Term::Array(child, len) => {
                self.term(child, depth + 1, contributes && *len > 0)?;
            }
            Term::Tuple(children) => {
                for child in children {
                    self.term(child, depth + 1, contributes)?;
                }
            }
            Term::Nominal(declaration, actuals) => {
                let key =
                    producer::key(self.read.world_word(), *declaration, actuals, self.budget)?;
                let mut target = None;
                for (index, member) in self.read.fact.members.iter().enumerate() {
                    self.budget.charge_work(1).map_err(Error::new)?;
                    if member.key == key {
                        target = Some(index);
                        break;
                    }
                }
                if let Some(index) = target {
                    let mode = usize::from(contributes);
                    if !self.queued[index][mode] {
                        self.queued[index][mode] = true;
                        self.pending.push((index, contributes));
                    }
                } else {
                    self.budget
                        .charge_work(self.read.fact.external.len())
                        .map_err(Error::new)?;
                    let external = self.read.external(&key).ok_or_else(|| {
                        Error::new("drop presence omitted successful external read")
                    })?;
                    let ty = external.definition()?;
                    external.check_definition(&ty)?;
                    let (required, unresolved) = (self.event)(&ty, false, depth)?;
                    self.required |= contributes && required;
                    self.unresolved |= unresolved;
                }
            }
            Term::Str | Term::Slice(_) | Term::Dyn(..) => {
                return Err(Error::new(
                    "unsupported recursive drop presence unsized equation",
                ));
            }
            Term::PhantomData(..)
            | Term::FnPointer(..)
            | Term::Int(_)
            | Term::Float(_)
            | Term::Bool
            | Term::Unit
            | Term::Never
            | Term::Char => {}
        }
        Ok(())
    }
    fn finish(mut self, depth: usize) -> Result<(bool, bool), Error> {
        let read = self.read;
        let budget = self.budget;
        while let Some((index, contributes)) = self.pending.pop() {
            budget.charge_work(1).map_err(Error::new)?;
            let ordinal = u32::try_from(index)
                .map_err(|_| Error::new("component member ordinal overflow"))?;
            let ty = read.materialize(ordinal, budget)?.definition()?;
            let (required, unresolved) = (self.event)(&ty, true, depth)?;
            self.required |= contributes && required;
            self.unresolved |= unresolved;
            let member = &read.fact.members[index];
            if read.pointer_kind(ordinal) == Some(ir::OwnedPointerKind::Rc) {
                let [pointee] = member.actuals.as_ref() else {
                    return Err(Error::new("Rc component changed pointee arity"));
                };
                self.term(pointee, depth + 1, contributes)?;
            } else {
                for (_, term) in member.fields.iter() {
                    self.term(term, depth + 1, contributes)?;
                }
            }
        }
        Ok((self.required, self.unresolved))
    }
}
impl ReadComponent {
    pub(super) fn drop_presence(
        &self,
        member: u32,
        budget: &ir::PlanBudget,
        depth: usize,
        event: &mut impl FnMut(&ir::Ty, bool, usize) -> Result<(bool, bool), Error>,
    ) -> Result<(bool, bool), Error> {
        budget.check_depth(depth).map_err(Error::new)?;
        let root =
            usize::try_from(member).map_err(|_| Error::new("component member ordinal overflow"))?;
        if root >= self.fact.members.len() {
            return Err(Error::new("component member ordinal out of bounds"));
        }
        let mut queued = topology::storage(self.fact.members.len(), budget)?;
        queued.resize(self.fact.members.len(), [false; 2]);
        queued[root][1] = true;
        let mode_count = self
            .fact
            .members
            .len()
            .checked_mul(2)
            .ok_or_else(|| Error::new("drop presence mode count overflow"))?;
        let mut pending = topology::storage(mode_count, budget)?;
        pending.push((root, true));
        DropPresenceInventory {
            read: self,
            budget,
            event,
            queued,
            pending,
            required: false,
            unresolved: false,
        }
        .finish(depth)
    }
}

/// Finite ownership preflight over authenticated component equations. The
/// callback charges weighted visits (`None`) or recursively checks a genuine
/// external nominal (`Some`). A queued bit denotes pending work, never success.
struct OwnershipInventory<'a, F> {
    read: &'a ReadComponent,
    budget: &'a ir::PlanBudget,
    event: &'a mut F,
    queued: Vec<bool>,
    pending: Vec<usize>,
}
impl<F: FnMut(usize, Option<(&ir::Ty, usize)>) -> Result<(), Error>> OwnershipInventory<'_, F> {
    fn term(&mut self, term: &Term, depth: usize) -> Result<(), Error> {
        let b = self.budget;
        b.check_depth(depth).map_err(Error::new)?;
        (self.event)(
            depth
                .checked_add(1)
                .ok_or_else(|| Error::new("ownership depth overflow"))?,
            None,
        )?;
        if depth > 128 {
            return Err(Error::new("ownership type depth limit exceeded"));
        }
        match term {
            Term::Ref(_, child) => self.term(child, depth + 1)?,
            Term::RawPtr(..) => {} // Ordinary preflight treats raw storage as a leaf.
            Term::Array(child, len) => {
                if *len > ir::MAX_ARRAY_LEN {
                    if matches!(
                        child.as_ref(),
                        Term::Int(_) | Term::Float(_) | Term::Bool | Term::Char
                    ) {
                        return Ok(());
                    }
                    return Err(Error::new("ownership array expansion limit exceeded"));
                }
                for _ in 0..(*len).max(1) {
                    self.term(child, depth + 1)?;
                }
            }
            Term::Tuple(children) => {
                for child in children {
                    self.term(child, depth + 1)?;
                }
            }
            Term::Nominal(declaration, actuals) => {
                for actual in actuals {
                    self.term(actual, depth + 1)?;
                }
                let key = producer::key(self.read.world_word(), *declaration, actuals, b)?;
                let mut target = None;
                for (index, member) in self.read.fact.members.iter().enumerate() {
                    (self.event)(1, None)?;
                    if member.key == key {
                        target = Some(index);
                        break;
                    }
                }
                if let Some(index) = target {
                    if !self.queued[index] {
                        self.queued[index] = true;
                        self.pending.push(index);
                    }
                } else {
                    (self.event)(self.read.fact.external.len(), None)?;
                    let external = self.read.external(&key).ok_or_else(|| {
                        Error::new("ownership inventory omitted successful external read")
                    })?;
                    let ty = external.definition()?;
                    external.check_definition(&ty)?;
                    // Its ordinary preflight includes its own root and actuals.
                    (self.event)(0, Some((&ty, depth)))?;
                }
            }
            Term::Str | Term::Slice(_) | Term::Dyn(..) => {
                return Err(Error::new(
                    "unsupported recursive ownership unsized equation",
                ));
            }
            Term::PhantomData(..)
            | Term::FnPointer(..)
            | Term::Int(_)
            | Term::Float(_)
            | Term::Bool
            | Term::Unit
            | Term::Never
            | Term::Char => {}
        }
        Ok(())
    }
}
impl ReadComponent {
    pub(super) fn ownership_preflight(
        &self,
        member: u32,
        b: &ir::PlanBudget,
        depth: usize,
        event: &mut impl FnMut(usize, Option<(&ir::Ty, usize)>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        b.check_depth(depth).map_err(Error::new)?;
        let root =
            usize::try_from(member).map_err(|_| Error::new("component member ordinal overflow"))?;
        if root >= self.fact.members.len() {
            return Err(Error::new("component member ordinal out of bounds"));
        }
        let mut queued = topology::storage(self.fact.members.len(), b)?;
        queued.resize(self.fact.members.len(), false);
        let mut pending = topology::storage(self.fact.members.len(), b)?;
        pending.push(root);
        queued[root] = true;
        let mut inventory = OwnershipInventory {
            read: self,
            budget: b,
            event,
            queued,
            pending,
        };
        // Root has already paid its ordinary preflight visit. Each equation is
        // scanned completely at the finite component boundary, not unfolded at
        // every recursive edge. Every edge and expanded array occurrence pays.
        for actual in self.fact.members[root].actuals.iter() {
            inventory.term(actual, depth + 1)?;
        }
        while let Some(index) = inventory.pending.pop() {
            let member = &self.fact.members[index];
            if index != root {
                (inventory.event)(depth + 1, None)?;
                for actual in member.actuals.iter() {
                    inventory.term(actual, depth + 1)?;
                }
            }
            if let ir::Ty::Enum(value) = member.source.template() {
                (inventory.event)(value.def().variants.len(), None)?;
            }
            for (_, term) in member.fields.iter() {
                inventory.term(term, depth + 1)?;
            }
        }
        Ok(())
    }
}
