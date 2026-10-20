//! Abstract return-position `impl Trait` identities and declaration-owned bounds.
//! Hidden representations are separate immutable body outputs, never slots here.
use super::*;

/// A self-free trait application. Its `Self` is supplied only when proving a
/// declared bound for this opaque type, avoiding recursive Arc metadata.
/// Trait defaults such as `trait Tr<T = Self>` refer to the eventual opaque
/// identity without embedding an Arc cycle or a fabricated concrete type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum OpaqueBoundArg {
    SelfType,
    Type(Ty),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OpaqueBound {
    declaration: DeclId,
    args: Arc<[OpaqueBoundArg]>,
    equalities: Arc<[(DeclId, Ty)]>,
}

impl OpaqueBound {
    pub(in crate::compiler) fn new(
        declaration: DeclId,
        args: Arc<[OpaqueBoundArg]>,
        equalities: Arc<[(DeclId, Ty)]>,
    ) -> Self {
        Self {
            declaration,
            args,
            equalities,
        }
    }
    pub fn declaration(&self) -> DeclId {
        self.declaration
    }
    pub fn args(&self) -> &[OpaqueBoundArg] {
        &self.args
    }
    pub fn application(&self, self_ty: Ty) -> TraitRef {
        TraitRef {
            declaration: self.declaration,
            args: self
                .args
                .iter()
                .map(|arg| match arg {
                    OpaqueBoundArg::SelfType => self_ty.clone(),
                    OpaqueBoundArg::Type(ty) => ty.clone(),
                })
                .collect::<Vec<_>>()
                .into(),
            self_ty,
        }
    }
    pub fn equalities(&self) -> &[(DeclId, Ty)] {
        &self.equalities
    }
    pub(in crate::compiler) fn try_map<E>(
        &self,
        map: &mut impl FnMut(&Ty) -> Result<Ty, E>,
    ) -> Result<Self, E> {
        Ok(Self::new(
            self.declaration,
            self.args
                .iter()
                .map(|arg| {
                    Ok(match arg {
                        OpaqueBoundArg::SelfType => OpaqueBoundArg::SelfType,
                        OpaqueBoundArg::Type(ty) => OpaqueBoundArg::Type(map(ty)?),
                    })
                })
                .collect::<Result<Vec<_>, E>>()?
                .into(),
            self.equalities
                .iter()
                .map(|(item, ty)| Ok((*item, map(ty)?)))
                .collect::<Result<Vec<_>, E>>()?
                .into(),
        ))
    }
    pub fn types(&self) -> impl Iterator<Item = &Ty> {
        self.args
            .iter()
            .filter_map(|arg| match arg {
                OpaqueBoundArg::SelfType => None,
                OpaqueBoundArg::Type(ty) => Some(ty),
            })
            .chain(self.equalities.iter().map(|(_, ty)| ty))
    }
}

/// Canonical abstract identity of one return-position occurrence in a function.
/// Ordered owner arguments retain both enclosing impl and callable parameters.
/// Bound metadata is authenticated against that declaration's source schema.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OpaqueTy {
    owner: DeclId,
    owner_args: Arc<[Ty]>,
    site: u32,
    bounds: Arc<[OpaqueBound]>,
    captured_regions: Arc<[SourceRegionId]>,
}

impl OpaqueTy {
    pub(in crate::compiler) fn new(
        owner: DeclId,
        owner_args: Arc<[Ty]>,
        site: u32,
        bounds: Arc<[OpaqueBound]>,
        captured_regions: Arc<[SourceRegionId]>,
    ) -> Self {
        Self {
            owner,
            owner_args,
            site,
            bounds,
            captured_regions,
        }
    }
    pub fn owner(&self) -> DeclId {
        self.owner
    }
    pub fn owner_args(&self) -> &[Ty] {
        &self.owner_args
    }
    pub fn site(&self) -> u32 {
        self.site
    }
    pub fn bounds(&self) -> &[OpaqueBound] {
        &self.bounds
    }
    pub fn captured_regions(&self) -> &[SourceRegionId] {
        &self.captured_regions
    }
    pub fn types(&self) -> impl Iterator<Item = &Ty> {
        self.owner_args
            .iter()
            .chain(self.bounds.iter().flat_map(OpaqueBound::types))
    }
    pub(in crate::compiler) fn try_map<E>(
        &self,
        mut map: impl FnMut(&Ty) -> Result<Ty, E>,
    ) -> Result<Self, E> {
        Ok(Self::new(
            self.owner,
            self.owner_args
                .iter()
                .map(&mut map)
                .collect::<Result<Vec<_>, _>>()?
                .into(),
            self.site,
            self.bounds
                .iter()
                .map(|bound| bound.try_map(&mut map))
                .collect::<Result<Vec<_>, _>>()?
                .into(),
            self.captured_regions.clone(),
        ))
    }
    pub(in crate::compiler) fn try_eq<'a, E>(
        &'a self,
        other: &'a Self,
        mut equal: impl FnMut(&'a Ty, &'a Ty) -> Result<bool, E>,
    ) -> Result<bool, E> {
        if self.owner != other.owner
            || self.site != other.site
            || self.owner_args.len() != other.owner_args.len()
            || self.bounds.len() != other.bounds.len()
            || self.captured_regions != other.captured_regions
        {
            return Ok(false);
        }
        for (left, right) in self.bounds.iter().zip(other.bounds.iter()) {
            if left.declaration != right.declaration
                || left.args.len() != right.args.len()
                || left.equalities.len() != right.equalities.len()
                || left.args.iter().zip(right.args.iter()).any(|(a, b)| {
                    matches!(a, OpaqueBoundArg::SelfType) != matches!(b, OpaqueBoundArg::SelfType)
                })
                || left
                    .equalities
                    .iter()
                    .zip(right.equalities.iter())
                    .any(|(a, b)| a.0 != b.0)
            {
                return Ok(false);
            }
        }
        for (left, right) in self.types().zip(other.types()) {
            if !equal(left, right)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn pack_capture_sources(
    opaque: &OpaqueTy,
    hidden: &Ty,
    budget: &PlanBudget,
) -> Result<Arc<[CarrierLocation]>, String> {
    let locations = carrier_locations(hidden, budget)?;
    if locations.is_empty() {
        return Ok(locations);
    }
    let Ty::Closure(closure) = hidden else {
        return Err("unsupported: opaque pack requires explicit captured carrier mapping for non-closure hidden types".into());
    };
    if closure.owner() != opaque.owner() || closure.owner_args() != opaque.owner_args() {
        return Err(
            "opaque capture mapping changed its canonical owner or ordered arguments".into(),
        );
    }
    for (index, location) in locations.iter().enumerate() {
        budget.charge_work(
            index
                .saturating_add(opaque.owner_args().len())
                .saturating_add(1),
        )?;
        let [CarrierStep::Field(field)] = location.path.0.as_ref() else {
            return Err("unsupported: opaque pack requires explicit captured carrier mapping for nested capture channels".into());
        };
        let CarrierChannel::ParamTail {
            parameter,
            prefix: 0,
        } = &location.channel
        else {
            return Err("unsupported: opaque pack requires explicit captured carrier mapping for fixed or nested references".into());
        };
        if parameter.owner.declaration != opaque.owner()
            || !opaque.owner_args().contains(&Ty::Param(*parameter))
            || closure.captures().get(*field) != Some(&Ty::Param(*parameter))
            || !closure
                .sources()
                .get(*field)
                .is_some_and(|source| source.kind == CaptureKind::Value)
        {
            return Err("opaque capture mapping changed its defining parameter, field or value-capture authority".into());
        }
        if locations[..index]
            .iter()
            .any(|prior| prior.channel == location.channel)
        {
            return Err("unsupported: opaque repeated-family capture fields require a complete multi-field capture contract".into());
        }
    }
    Ok(locations)
}

/// A defining-body pack retaining exact ordered symbolic capture origins.
/// An arbitrary public opaque value still retains its abstract carrier family.
#[derive(Clone, Debug)]
pub struct OpaquePackProof {
    opaque: Arc<OpaqueTy>,
    hidden: Ty,
    publication: super::super::source_replay::OriginalBodyId,
    occurrence: ExprId,
    sources: Arc<[CarrierLocation]>,
}
impl PartialEq for OpaquePackProof {
    fn eq(&self, other: &Self) -> bool {
        self.publication == other.publication
            && self.occurrence == other.occurrence
            && self.opaque == other.opaque
            && self.hidden == other.hidden
            && self.sources == other.sources
    }
}
impl Eq for OpaquePackProof {}
impl OpaquePackProof {
    pub(in crate::compiler) fn seal(
        publication: Arc<super::super::source_replay::CanonicalSourcePublication>,
        occurrence: ExprId,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        publication
            .validate_source(publication.source(), publication.facts(), budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let source = publication.source();
        let expression = source
            .exprs
            .get(occurrence.0)
            .ok_or("opaque pack has no defining source occurrence")?;
        let ExprKind::OpaqueWrap { opaque, value } = &expression.kind else {
            return Err("opaque pack occurrence is not a defining wrapper".into());
        };
        let hidden = source
            .exprs
            .get(value.0)
            .ok_or("opaque wrapper has no source operand")?
            .ty
            .clone();
        let facts = FactsCtx::from_validated(publication.facts());
        validate_wrap(opaque, &hidden, facts)?;
        if !identical_type(&expression.ty, &Ty::Opaque(opaque.clone()), budget)? {
            return Err("opaque wrapper changed its defining result identity".into());
        }
        let sources = pack_capture_sources(opaque, &hidden, budget)?;
        Ok(Self {
            opaque: opaque.clone(),
            hidden,
            publication: publication.identity().clone(),
            occurrence,
            sources,
        })
    }
    pub(in crate::compiler) fn validate_occurrence(
        &self,
        publication: &Arc<super::super::source_replay::CanonicalSourcePublication>,
        occurrence: Option<ExprId>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if publication.identity() != &self.publication || occurrence != Some(self.occurrence) {
            return Err("opaque pack changed its canonical defining body occurrence".into());
        }
        publication
            .validate_source(publication.source(), publication.facts(), budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let replay = Self::seal(publication.clone(), self.occurrence, budget)?;
        if replay != *self {
            return Err("opaque pack changed its sealed defining occurrence".into());
        }
        self.validate_shape(
            &self.hidden,
            FactsCtx::from_validated(publication.facts()),
            budget,
        )
    }
    pub fn opaque(&self) -> &Arc<OpaqueTy> {
        &self.opaque
    }
    pub fn hidden(&self) -> &Ty {
        &self.hidden
    }
    /// Type/shape preflight only. This does not authorize ownership transport:
    /// validate_occurrence must replay the graph-read publication at the CFG gate.
    pub(in crate::compiler) fn validate_shape(
        &self,
        hidden: &Ty,
        facts: FactsCtx<'_>,
        budget: &PlanBudget,
    ) -> Result<(), String> {
        if !identical_type(&self.hidden, hidden, budget)? {
            return Err("opaque pack changed its checked hidden operand type".into());
        }
        validate_wrap(&self.opaque, hidden, facts)?;
        if pack_capture_sources(&self.opaque, hidden, budget)? != self.sources {
            return Err("opaque pack changed its exact ordered captured carrier mapping".into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn captured_sources(&self) -> &[CarrierLocation] {
        &self.sources
    }
    pub(in crate::compiler) fn captured_destination(&self) -> CarrierLocation {
        CarrierLocation {
            path: CarrierPath::default(),
            channel: CarrierChannel::OpaqueTail {
                opaque: self.opaque.clone(),
                prefix: 0,
            },
        }
    }
}

#[cfg(test)]
mod captured_pack_tests {
    use super::*;
    use crate::compiler as c;

    #[test]
    fn captured_pack_rejects_order_omission_foreign_occurrence_and_family() {
        let snapshot = Arc::new(c::Snapshot {
            input: c::Input::Text("fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C{move |x:A|g(f(x))}fn main(){}".into()),
            source_options: c::source::SourceOptions::default(),
            budget: PlanBudget::default(),
        });
        let budget = snapshot.budget.clone();
        let engine = c::Engine::new(c::CompilerProvider(snapshot));
        let key = c::Facts::item(c::TEMPLATE_BODY, 0);
        let requested = engine.request(key.clone());
        let other_key = c::Facts::item(c::TEMPLATE_BODY, 1);
        let other_requested = engine.request(other_key.clone());
        engine.run(1).unwrap();
        requested.completion().unwrap();
        other_requested.completion().unwrap();
        let other_body = engine
            .fact_of::<c::TemplateBodyFact>(&other_key, 0)
            .unwrap();
        let body = engine.fact_of::<c::TemplateBodyFact>(&key, 0).unwrap();
        let publication = body.publication.as_ref().unwrap();
        let occurrence = body
            .checked
            .exprs
            .iter()
            .position(|expr| matches!(expr.kind, ExprKind::OpaqueWrap { .. }))
            .unwrap();
        let proof =
            OpaquePackProof::seal(publication.clone(), ExprId(occurrence), &budget).unwrap();
        assert_eq!(proof.sources.len(), 2);
        proof
            .validate_occurrence(publication, Some(ExprId(occurrence)), &budget)
            .unwrap();
        let facts = FactsCtx::from_validated(publication.facts());
        assert!(
            proof
                .validate_occurrence(
                    other_body.publication.as_ref().unwrap(),
                    Some(ExprId(occurrence)),
                    &budget
                )
                .is_err()
        );
        let mut changed = proof.clone();
        let mut rows = changed.sources.to_vec();
        rows.reverse();
        changed.sources = rows.into();
        assert!(
            changed
                .validate_occurrence(publication, Some(ExprId(occurrence)), &budget)
                .is_err()
        );
        let mut changed = proof.clone();
        changed.sources = Arc::from([proof.sources[0].clone()]);
        assert!(
            changed
                .validate_shape(&proof.hidden, facts, &budget)
                .is_err()
        );
        let mut changed = proof.clone();
        changed.publication = c::source_replay::OriginalBodyId::new(&budget).unwrap();
        assert!(
            changed
                .validate_occurrence(publication, Some(ExprId(occurrence)), &budget)
                .is_err()
        );
        assert!(
            proof
                .validate_occurrence(publication, Some(ExprId(occurrence + 1)), &budget)
                .is_err()
        );
        let mut changed = proof.clone();
        let mut rows = changed.sources.to_vec();
        let CarrierChannel::ParamTail { parameter, .. } = &mut rows[0].channel else {
            panic!("expected symbolic family")
        };
        parameter.index = u16::MAX;
        changed.sources = rows.into();
        assert!(
            changed
                .validate_shape(&proof.hidden, facts, &budget)
                .is_err()
        );
        for rows in [
            Arc::from([proof.sources[0].clone(), proof.sources[0].clone()]),
            {
                let mut rows = proof.sources.to_vec();
                let CarrierChannel::ParamTail { prefix, .. } = &mut rows[0].channel else {
                    unreachable!()
                };
                *prefix = 1;
                Arc::from(rows)
            },
            {
                let mut rows = proof.sources.to_vec();
                rows[0].path = CarrierPath(Arc::from([CarrierStep::Field(usize::MAX)]));
                Arc::from(rows)
            },
        ] {
            let mut changed = proof.clone();
            changed.sources = rows;
            assert!(
                changed
                    .validate_occurrence(publication, Some(ExprId(occurrence)), &budget)
                    .is_err()
            );
        }
        for opaque in [
            OpaqueTy::new(
                proof.opaque.owner(),
                proof.opaque.owner_args().into(),
                proof.opaque.site() + 1,
                proof.opaque.bounds().into(),
                proof.opaque.captured_regions().into(),
            ),
            OpaqueTy::new(
                proof.opaque.owner(),
                proof.opaque.owner_args().into(),
                proof.opaque.site(),
                proof.opaque.bounds().into(),
                Arc::from([SourceRegionId {
                    declaration: proof.opaque.owner(),
                    index: u16::MAX,
                }]),
            ),
        ] {
            let mut changed = proof.clone();
            changed.opaque = Arc::new(opaque);
            assert!(
                changed
                    .validate_occurrence(publication, Some(ExprId(occurrence)), &budget)
                    .is_err()
            );
        }
        let template = c::cfg::lower_source_template(publication, &budget).unwrap();
        c::ownership::verify_template(template).unwrap();
    }
}

/// Exact canonical defining body and all of its chosen return wrappers.
/// Neither a raw hidden record nor satisfaction of the declared bounds can mint it.
#[derive(Clone, Debug)]
pub struct OpaqueDefinitionProof {
    publication: super::super::source_replay::OriginalBodyId,
    record: OpaqueHiddenType,
    occurrences: Arc<[ExprId]>,
}
impl OpaqueDefinitionProof {
    pub(in crate::compiler) fn seal(
        publication: Arc<super::super::source_replay::CanonicalSourcePublication>,
        record: &OpaqueHiddenType,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        publication
            .validate_source(publication.source(), publication.facts(), budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        if publication.facts().declaration() != record.opaque.owner() {
            return Err("hidden representation changed its canonical defining body".into());
        }
        let mut occurrences = Vec::new();
        budget.charge_work(publication.source().exprs.len())?;
        for (index, expression) in publication.source().exprs.iter().enumerate() {
            let ExprKind::OpaqueWrap { opaque, value } = &expression.kind else {
                continue;
            };
            if opaque.owner() != record.opaque.owner() || opaque.site() != record.opaque.site() {
                continue;
            }
            if !identical_type(
                &Ty::Opaque(opaque.clone()),
                &Ty::Opaque(record.opaque.clone()),
                budget,
            )? || !identical_type(&expression.ty, &Ty::Opaque(record.opaque.clone()), budget)?
            {
                return Err("hidden record changed defining wrapper identity, bounds, arguments, or captures".into());
            }
            let operand = publication
                .source()
                .exprs
                .get(value.0)
                .ok_or("defining opaque wrapper lost its operand")?;
            if !identical_type(&operand.ty, &record.hidden, budget)? {
                return Err("hidden record replaced its canonical defining operand type".into());
            }
            occurrences.push(ExprId(index));
        }
        if occurrences.is_empty() {
            return Err("hidden record has no canonical defining wrapper".into());
        }
        validate_wrap(
            &record.opaque,
            &record.hidden,
            FactsCtx::from_validated(publication.facts()),
        )?;
        Ok(Self {
            publication: publication.identity().clone(),
            record: record.clone(),
            occurrences: occurrences.into(),
        })
    }
    pub(in crate::compiler) fn instantiate(
        &self,
        expected: &Arc<OpaqueTy>,
        publication: &Arc<super::super::source_replay::CanonicalSourcePublication>,
        context: &mut super::super::frontend::PlanCtx,
        budget: &PlanBudget,
    ) -> Result<OpaqueHiddenType, String> {
        if publication.identity() != &self.publication {
            return Err("opaque defining witness changed its canonical publication seal".into());
        }
        if !Arc::ptr_eq(publication.facts().world(), &context.trait_world()?) {
            return Err("opaque defining witness belongs to a foreign snapshot or world".into());
        }
        let replay = Self::seal(publication.clone(), &self.record, budget)?;
        if replay.occurrences != self.occurrences {
            return Err("opaque defining occurrence witness changed".into());
        }
        self.record.instantiate(expected, context, budget)
    }
}

/// One final hidden representation, published by its defining body operation.
/// Consumers must read the body's typed graph output before using this value.
#[derive(Clone, Debug)]
pub struct OpaqueHiddenType {
    opaque: Arc<OpaqueTy>,
    hidden: Ty,
}

impl OpaqueHiddenType {
    pub(in crate::compiler) fn new(opaque: Arc<OpaqueTy>, hidden: Ty) -> Self {
        Self { opaque, hidden }
    }
    pub fn opaque(&self) -> &Arc<OpaqueTy> {
        &self.opaque
    }
    pub fn hidden(&self) -> &Ty {
        &self.hidden
    }
    pub(in crate::compiler) fn instantiate(
        &self,
        expected: &Arc<OpaqueTy>,
        context: &mut super::super::frontend::PlanCtx,
        budget: &PlanBudget,
    ) -> Result<Self, String> {
        if self.opaque.owner() != expected.owner() || self.opaque.site() != expected.site() {
            return Err("hidden template changed its defining opaque occurrence".into());
        }
        let template_facts = context.callable_facts(expected.owner())?;
        let target = CallTarget {
            declaration: expected.owner(),
            type_args: Arc::from(expected.owner_args()),
        };
        let closed = context.instance_facts(&target)?;
        let facts = FactsCtx::from_validated(&closed);
        let instantiated = substitute_ty(
            &Ty::Opaque(self.opaque.clone()),
            template_facts.env(),
            expected.owner_args(),
            facts,
            context,
            budget,
        )?;
        if !context
            .trait_world()?
            .types_equal(&[instantiated], &[Ty::Opaque(expected.clone())])
            .map_err(|e| e.into_legacy_diagnostic(budget))?
        {
            return Err("hidden template changed owner arguments, bounds, or captures".into());
        }
        let hidden = substitute_ty(
            &self.hidden,
            template_facts.env(),
            expected.owner_args(),
            facts,
            context,
            budget,
        )?;
        validate_wrap(expected, &hidden, facts)?;
        Ok(Self::new(expected.clone(), hidden))
    }
}

pub(super) fn validate_wrap(
    opaque: &Arc<OpaqueTy>,
    hidden: &Ty,
    context: super::FactsCtx<'_>,
) -> Result<(), String> {
    let facts = context
        .validated()
        .ok_or("opaque wrapper requires canonical callable facts")?;
    if facts.declaration() != opaque.owner() {
        return Err("opaque wrapper is outside its defining callable".into());
    }
    facts
        .validate_ty_metadata(&Ty::Opaque(opaque.clone()))
        .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
    facts
        .require_sized(hidden)
        .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?;
    let prove = |goal: &super::Predicate| -> Result<(), String> {
        match facts
            .service()
            .prove_goal(goal, facts)
            .map_err(|e| e.into_legacy_diagnostic(facts.budget()))?
        {
            super::super::traits::ProofOutcome::Proved(_) => Ok(()),
            super::super::traits::ProofOutcome::NotProven => {
                Err("opaque hidden representation does not satisfy its public bounds".into())
            }
        }
    };
    for bound in opaque.bounds() {
        let application = bound.application(hidden.clone());
        prove(&super::Predicate::Implements(application.clone()))?;
        for (item, ty) in bound.equalities() {
            prove(&super::Predicate::ProjectionEq {
                projection: super::AssociatedTypeProjection {
                    trait_ref: application.clone(),
                    item: *item,
                },
                ty: ty.clone(),
            })?;
        }
    }
    Ok(())
}
