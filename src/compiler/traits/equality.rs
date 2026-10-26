//! Declared projection equalities are assumptions, never implementation assignments.
//! Staging contexts deliberately have no conversion to facts or final evidence.
use super::*;

mod reduction_rows;
use reduction_rows::ReductionRows;

#[cfg(test)]
mod freeze_tests;
#[cfg(test)]
mod owner_requirements_tests;

impl ValidatedFacts {
    /// True only for finalized rigid projected calls using a required equality view.
    pub(in crate::compiler) fn uses_equality_required_call(
        &self,
        call: &TraitCall,
    ) -> Result<bool> {
        self.world.check_facts(self, true)?;
        self.validate_callee_metadata(&Callee::TraitItem(call.clone()))?;
        Ok(self.phase() == FactsPhase::Universal
            && self
                .world
                .schema_contains_projection(&self.world.item_header(call.item)?.schema)?
            && !self.world.goal_is_closed(&call.trait_ref, self.env())?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DeclaredStageId(WorldId);
pub(super) struct DeclaredCallableContext {
    pub(super) world: WorldId,
    pub(super) stage: DeclaredStageId,
    pub(super) declaration: DeclId,
    pub(super) env: Arc<GenericEnv>,
    pub(super) predicates: Arc<[Predicate]>,
    pub(super) raw: GenericSchema,
    pub(super) origins: Arc<[super::parameter_equality::EqualityObligationOrigin]>,
    reductions: ReductionRows,
}

#[derive(Clone)]
struct DeclaredEqualityReduction {
    world: WorldId,
    stage: DeclaredStageId,
    declaration: DeclId,
    projection: ir::AssociatedTypeProjection,
    rhs: Ty,
}

/// Private construction result, never independently published.
pub(super) struct DeclaredCallableNormalization {
    schema: GenericSchema,
    world: WorldId,
    stage: DeclaredStageId,
    declaration: DeclId,
    reductions: ReductionRows,
}
impl DeclaredCallableNormalization {
    pub(super) fn finish(
        self,
        context: &mut DeclaredCallableContext,
        budget: &PlanBudget,
    ) -> Result<GenericSchema> {
        if self.world != context.world
            || self.stage != context.stage
            || self.declaration != context.declaration
        {
            return Err(FactsError::invalid(
                budget,
                "declared reduction result changed stage or owner",
            ));
        }
        self.reductions.check(budget)?;
        charged(budget, budget.charge_work(self.reductions.values.len()))?;
        for row in &self.reductions.values {
            if row.world != context.world
                || row.stage != context.stage
                || row.declaration != context.declaration
            {
                return Err(FactsError::invalid(
                    budget,
                    "declared reduction row changed stage or owner",
                ));
            }
        }
        context.reductions.append(self.reductions, budget)?;
        Ok(self.schema)
    }
}

fn projection_formation_limits(budget: &PlanBudget) -> [usize; 6] {
    let limits = budget.limits();
    [
        limits.params,
        limits.applications,
        limits.depth,
        limits.nodes,
        limits.work,
        limits.bytes,
    ]
}

#[derive(Clone, Debug)]
struct ProjectionEqualityEvidence {
    world: WorldId,
    environment: usize,
    phase: FactsPhase,
    declaration: Option<DeclId>,
    projection: ir::AssociatedTypeProjection,
    rhs: Ty,
    rows: Arc<[Predicate]>,
    membership: ProofEvidence,
    validated_env: Arc<GenericEnv>,
    validated_registry: SnapshotId,
    validated_ready: bool,
    validated_finalized: bool,
    validated_limits: [usize; 6],
}

#[derive(Clone, Debug)]
pub struct NormalizedProjectionEquality(Arc<ProjectionEqualityEvidence>);
impl NormalizedProjectionEquality {
    pub fn ty(&self) -> &Ty {
        &self.0.rhs
    }
    pub fn validate_for(
        &self,
        projection: &ir::AssociatedTypeProjection,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        let world = facts.world();
        world.check_facts(facts, false)?;
        world.check_normalization_budget()?;
        world.validate_final_equality_context(facts)?;
        let evidence = &self.0;
        if evidence.world != world.id
            || evidence.environment != facts.predicates.index
            || evidence.phase != facts.phase()
            || evidence.declaration != facts.predicates.declaration
            || !world.same_projection(&evidence.projection, projection)?
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "projection equality replay changed world, environment, phase or projection",
            ));
        }
        world.validate_equality_domain_in(
            projection,
            &evidence.rhs,
            ProofContext::from_facts(facts),
            &mut ProofScratch::default(),
        )?;
        world.compare_predicate_rows(&evidence.rows, &facts.predicates.predicates)?;
        // Only the private producer below can seal these exact immutable rows.
        // A foreign allocation or changed formation context retains the full
        // validation path. Domain, membership, poison and caller checks above
        // and below remain mandatory even on this narrow formation reuse hit.
        let reuse_formation = Arc::ptr_eq(&evidence.rows, &facts.predicates.predicates)
            && evidence.validated_registry == world.registry
            && evidence.validated_ready == world.owner_normalization_ready
            && evidence.validated_finalized == world.owner_normalization_finalized
            && evidence.validated_limits == projection_formation_limits(&world.budget)
            && Arc::ptr_eq(&evidence.validated_env, &facts.predicates.env);
        let actual = if reuse_formation {
            world.reduce_declared_equality_rhs(
                projection,
                &facts.predicates.predicates,
                facts.env(),
                &mut Vec::new(),
            )?
        } else {
            world.declared_equality_rhs(projection, &facts.predicates.predicates, facts.env())?
        }
        .ok_or_else(|| {
            FactsError::invalid(facts.budget(), "projection equality lost its declared row")
        })?;
        if !world.eq_ty(&actual, &evidence.rhs, 0)?
            || evidence.membership.0.world != world.id
            || evidence.membership.0.environment != facts.predicates.index
            || evidence.membership.0.phase != facts.phase()
            || !world.eq_predicate(
                &evidence.membership.0.goal,
                &Predicate::Implements(projection.trait_ref.clone()),
            )?
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "projection equality replay changed RHS or membership",
            ));
        }
        world.require_in(
            &Predicate::Implements(projection.trait_ref.clone()),
            ProofContext::from_facts(facts),
            &mut ProofScratch::default(),
        )?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(super) enum ProjectionReduction {
    Equality(NormalizedProjectionEquality),
    Implementation(NormalizedAssociatedType),
}
impl ProjectionReduction {
    pub(super) fn ty(&self) -> &Ty {
        match self {
            Self::Equality(p) => p.ty(),
            Self::Implementation(p) => p.ty(),
        }
    }
}

#[derive(Debug)]
pub(super) struct EqualityRequiredMethodEvidence {
    world: WorldId,
    environment: usize,
    phase: FactsPhase,
    pub(super) call: TraitCall,
    pub(super) raw: GenericSchema,
    pub(super) arguments: Arc<[Ty]>,
    pub(super) signature: Signature,
    pub(super) source_regions: Arc<ir::SourceRegionEnv>,
    pub(super) dependencies: Arc<[(ir::AssociatedTypeProjection, ProjectionReduction)]>,
}
pub(super) struct EqualityRequiredApplication {
    pub(super) instantiated: InstantiatedSchema,
    pub(super) evidence: Arc<EqualityRequiredMethodEvidence>,
}

impl ValidatedTraitWorld {
    fn validate_final_equality_context(&self, facts: &ValidatedFacts) -> Result<()> {
        if !self.owner_normalization_finalized {
            return Err(FactsError::invalid(
                &self.budget,
                "final equality evidence requested from a private stage",
            ));
        }
        let declaration = facts.predicates.declaration.ok_or_else(|| {
            FactsError::invalid(&self.budget, "final equality has no declaring callable")
        })?;
        let context = self.declared_callable_context(declaration)?;
        self.validate_declaration_schema(declaration, &context.raw)?;
        if !self.same_env(&context.env, facts.env())? {
            return Err(FactsError::invalid(
                &self.budget,
                "final equality changed declaring binder provenance",
            ));
        }
        self.validate_predicates(&facts.predicates.predicates, facts.env(), None)?;
        self.validate_declared_equality_domains(&facts.predicates.predicates, facts.env())
    }
    pub(super) fn validate_predicate_metadata(
        &self,
        predicate: &Predicate,
        env: &GenericEnv,
    ) -> Result<()> {
        self.validate_predicate_metadata_with(predicate, env, &mut NominalReplays::default())
    }
    pub(super) fn validate_predicate_metadata_with(
        &self,
        predicate: &Predicate,
        env: &GenericEnv,
        replays: &mut NominalReplays,
    ) -> Result<()> {
        self.validate_predicate_metadata_with_facts(predicate, env, replays, None)
    }
    pub(super) fn validate_predicate_metadata_with_facts(
        &self,
        predicate: &Predicate,
        env: &GenericEnv,
        replays: &mut NominalReplays,
        caller: Option<&ValidatedFacts>,
    ) -> Result<()> {
        for ty in predicate.metadata_types() {
            self.validate_ty_metadata_with_facts(ty, env, 0, replays, caller)?;
        }
        if let Predicate::ProjectionEq { projection, .. } = predicate {
            self.validate_ty_metadata_with_facts(
                &self.projection_type(projection)?,
                env,
                0,
                replays,
                caller,
            )?;
        }
        Ok(())
    }
    fn projection_type(&self, projection: &ir::AssociatedTypeProjection) -> Result<Ty> {
        retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
        Ok(Ty::AssociatedType(Arc::new(projection.clone())))
    }
    pub(super) fn validate_final_declared_contexts(&self) -> Result<()> {
        for context in &self.declared_callables {
            let env = self.predicate_env(context.declaration)?;
            self.validate_predicates(&env.predicates, &env.env, None)?;
            self.validate_declared_equality_domains(&env.predicates, &env.env)?;
            let mut canonical = Vec::new();
            for row in context.predicates.iter() {
                self.append_predicate(&mut canonical, row)?;
            }
            if canonical.len() != env.predicates.len() {
                return Err(FactsError::invalid(
                    &self.budget,
                    "final callable rows differ from staged declaration",
                ));
            }
            for row in &canonical {
                let mut found = false;
                for actual in env.predicates.iter() {
                    found |= self.eq_predicate(row, actual)?;
                }
                if !found {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "final callable lost staged predicate provenance",
                    ));
                }
            }
            context.reductions.check(&self.budget)?;
            for reduction in &context.reductions.values {
                if reduction.world != self.id
                    || reduction.stage != context.stage
                    || reduction.declaration != context.declaration
                {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "declared equality dependency changed its private stage",
                    ));
                }
                let rhs = self
                    .declared_equality_rhs(&reduction.projection, &env.predicates, &env.env)?
                    .ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "staged equality absent from finalized rows",
                        )
                    })?;
                if !self.eq_ty(&rhs, &reduction.rhs, 0)? {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "staged equality differs from finalized rows",
                    ));
                }
            }
        }
        Ok(())
    }
    pub(super) fn has_declared_callable_equalities(&self, declaration: DeclId) -> Result<bool> {
        Ok(self
            .declared_callable_context(declaration)?
            .predicates
            .iter()
            .any(|row| matches!(row, Predicate::ProjectionEq { .. })))
    }
    pub(super) fn validate_predicate_substitution(
        &self,
        raw: &Predicate,
        actual: &Predicate,
        env: &GenericEnv,
        arguments: &[Ty],
    ) -> Result<()> {
        match (raw, actual) {
            (Predicate::Implements(_), Predicate::Implements(_)) => {}
            (
                Predicate::ProjectionEq {
                    projection: left, ..
                },
                Predicate::ProjectionEq {
                    projection: right, ..
                },
            ) if left.item == right.item => {}
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "predicate substitution changed kind or associated item",
                ));
            }
        }
        if raw.trait_ref().declaration != actual.trait_ref().declaration
            || raw.trait_ref().args.len() != actual.trait_ref().args.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "predicate substitution changed complete trait application",
            ));
        }
        for (left, right) in raw.metadata_types().zip(actual.metadata_types()) {
            self.validate_rhs_substitution(left, right, env, arguments, 0)?;
        }
        Ok(())
    }
    pub(super) fn instantiate_equality_required_item(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<EqualityRequiredApplication> {
        self.check_facts(facts, true)?;
        facts.validate_callee_metadata(&Callee::TraitItem(call.clone()))?;
        if facts.phase() != FactsPhase::Universal
            || self.goal_is_closed(&call.trait_ref, facts.env())?
        {
            return Err(FactsError::unsupported(
                &self.budget,
                "equality required view needs a rigid universal caller",
            ));
        }
        let item = self.item_header(call.item)?;
        self.validate_projected_required_schema(item)?;
        if !call.method_args.is_empty() {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: generic equality required method",
            ));
        }
        let context = ProofContext::from_facts(facts);
        self.require_in(
            &Predicate::Implements(call.trait_ref.clone()),
            context,
            &mut ProofScratch::default(),
        )?;
        let raw = clone_schema(&item.schema, &self.budget)?;
        let source_regions = self.validate_source_regions(call.item, &raw.signature)?;
        let arguments = self.trait_arguments(&call.trait_ref)?;
        let mut instantiated = self.substitute_projected_schema(&raw, &arguments, facts.env())?;
        let mut dependencies = Vec::new();
        let mut reduce = |projection: &ir::AssociatedTypeProjection| -> Result<Ty> {
            let proof = self.normalize_projection_for_facts(projection, facts)?;
            self.validate_equality_domain_in(
                projection,
                proof.ty(),
                context,
                &mut ProofScratch::default(),
            )?;
            let normal = proof.ty().clone();
            push(&self.budget, &mut dependencies, (projection.clone(), proof))?;
            Ok(normal)
        };
        for ty in &mut instantiated.signature.params {
            *ty = self.map_equality_type(ty, facts.env(), 0, &mut reduce)?;
        }
        instantiated.signature.result =
            self.map_equality_type(&instantiated.signature.result, facts.env(), 0, &mut reduce)?;
        if !dependencies
            .iter()
            .any(|(_, proof)| matches!(proof, ProjectionReduction::Equality(_)))
        {
            return Err(FactsError::invalid(
                &self.budget,
                "rigid required view has no equality reduction",
            ));
        }
        for row in instantiated.predicates.iter() {
            self.require_in(row, context, &mut ProofScratch::default())?;
        }
        self.validate_schema_signature(call.item, &instantiated.signature, facts.env(), None)?;
        let normalized_regions =
            self.validate_source_regions(call.item, &instantiated.signature)?;
        if *normalized_regions != *source_regions {
            return Err(FactsError::invalid(
                &self.budget,
                "required source regions changed during equality reduction",
            ));
        }
        retain::<EqualityRequiredMethodEvidence>(&self.budget, 1)?;
        retain::<Ty>(&self.budget, instantiated.signature.params.len())?;
        charged(
            &self.budget,
            self.budget.charge_work(instantiated.signature.params.len()),
        )?;
        let evidence = Arc::new(EqualityRequiredMethodEvidence {
            world: self.id,
            environment: facts.predicates.index,
            phase: facts.phase(),
            call: call.clone(),
            raw,
            arguments: arguments.into(),
            signature: instantiated.signature.clone(),
            source_regions,
            dependencies: dependencies.into(),
        });
        Ok(EqualityRequiredApplication {
            instantiated,
            evidence,
        })
    }
    pub(super) fn validate_equality_required_carrier(
        &self,
        view: &CarrierContractView,
        evidence: &EqualityRequiredMethodEvidence,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        self.check_normalization_budget()?;
        if evidence.world != self.id
            || evidence.environment != facts.predicates.index
            || evidence.phase != facts.phase()
            || view.declaration != evidence.call.item
            || view.projected.is_some()
            || !self.same_env(&view.original_env, facts.env())?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "required equality carrier changed exact caller authority",
            ));
        }
        let subject = view.projected_subject.as_ref().ok_or_else(|| {
            FactsError::invalid(&self.budget, "required equality carrier lost subject")
        })?;
        if !self.eq_predicate(
            &Predicate::Implements(subject.clone()),
            &Predicate::Implements(evidence.call.trait_ref.clone()),
        )? {
            return Err(FactsError::invalid(
                &self.budget,
                "required equality carrier changed trait application",
            ));
        }
        self.validate_declaration_schema(evidence.call.item, &evidence.raw)?;
        let source = self.validate_source_regions(evidence.call.item, &evidence.raw.signature)?;
        if *source != *evidence.source_regions || *source != *view.source_regions {
            return Err(FactsError::invalid(
                &self.budget,
                "required equality carrier changed original source regions",
            ));
        }
        let replay = self.instantiate_equality_required_item(&evidence.call, facts)?;
        if !self.eq_signature(&evidence.signature, &replay.instantiated.signature)?
            || !self.eq_types(&evidence.arguments, &replay.evidence.arguments, 0)?
            || evidence.dependencies.len() != replay.evidence.dependencies.len()
            || !self.eq_contract(&view.fixed, &evidence.signature.ref_contract)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "required equality carrier changed required substitution",
            ));
        }
        for ((projection, proof), (other, _)) in evidence
            .dependencies
            .iter()
            .zip(replay.evidence.dependencies.iter())
        {
            if !self.same_projection(projection, other)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "required equality occurrence changed",
                ));
            }
            match proof {
                ProjectionReduction::Equality(proof) => proof.validate_for(projection, facts)?,
                ProjectionReduction::Implementation(proof) => {
                    proof.validate_for(projection, facts)?
                }
            }
        }
        let origin = charged(
            &self.budget,
            ir::derive_signature_origin_schema(&evidence.signature, facts.env(), &self.budget),
        )?;
        let (contract, regions) = self.derive_carriers(&evidence.signature, &source)?;
        if *origin != *view.origin
            || contract != view.contract
            || contract != view.original_contract
            || regions != view.regions
        {
            return Err(FactsError::invalid(
                &self.budget,
                "required equality view changed caller carrier coordinates",
            ));
        }
        Ok(())
    }
    pub(in crate::compiler) fn same_projection(
        &self,
        a: &ir::AssociatedTypeProjection,
        b: &ir::AssociatedTypeProjection,
    ) -> Result<bool> {
        Ok(a.item == b.item
            && self.eq_predicate(
                &Predicate::Implements(a.trait_ref.clone()),
                &Predicate::Implements(b.trait_ref.clone()),
            )?)
    }
    fn compare_predicate_rows(&self, left: &[Predicate], right: &[Predicate]) -> Result<()> {
        if left.len() != right.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "declared predicate row count changed",
            ));
        }
        for (left, right) in left.iter().zip(right) {
            if !self.eq_predicate(left, right)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "declared predicate row provenance changed",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn validate_equality_domain(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        env: &GenericEnv,
    ) -> Result<()> {
        self.validate_equality_domain_with(projection, rhs, env, &mut NominalReplays::default())
    }
    pub(super) fn validate_equality_domain_with(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        env: &GenericEnv,
        replays: &mut NominalReplays,
    ) -> Result<()> {
        self.validate_equality_domain_with_facts(projection, rhs, env, replays, None)
    }
    pub(super) fn validate_equality_domain_with_facts(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        env: &GenericEnv,
        replays: &mut NominalReplays,
        caller: Option<&ValidatedFacts>,
    ) -> Result<()> {
        self.validate_equality_domain_policy(projection, rhs, env, replays, caller, |goal, env| {
            self.goal_is_closed(goal, env)
        })
    }
    // Only callers already owning a proof arena use its existing completed-root
    // inventory. Metadata replays and closedness have disjoint mutable borrows;
    // no arena, authority, or retained allocation is created by this adapter.
    pub(super) fn validate_equality_domain_with_active(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        env: &GenericEnv,
        active: &mut ProofScratch,
        caller: Option<&ValidatedFacts>,
    ) -> Result<()> {
        let ProofScratch {
            replays,
            closedness,
            closedness_storage,
            ..
        } = active;
        self.validate_equality_domain_with_storage(
            projection,
            rhs,
            env,
            replays,
            caller,
            closedness,
            closedness_storage,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate_equality_domain_with_storage(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        env: &GenericEnv,
        replays: &mut NominalReplays,
        caller: Option<&ValidatedFacts>,
        closedness: &mut Vec<ClosednessValidation>,
        closedness_storage: &mut Vec<ir::PlanReservation>,
    ) -> Result<()> {
        self.validate_equality_domain_policy(projection, rhs, env, replays, caller, |goal, env| {
            self.goal_is_closed_with_storage(goal, env, closedness, closedness_storage)
        })
    }
    fn validate_equality_domain_policy(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        env: &GenericEnv,
        replays: &mut NominalReplays,
        caller: Option<&ValidatedFacts>,
        mut goal_is_closed: impl FnMut(&TraitRef, &GenericEnv) -> Result<bool>,
    ) -> Result<()> {
        self.check_normalization_budget()?;
        self.validate_ty_metadata_with_facts(
            &self.projection_type(projection)?,
            env,
            0,
            replays,
            caller,
        )?;
        self.validate_ty_metadata_with_facts(rhs, env, 0, replays, caller)?;
        // Raw admission is solver-free. Metadata, including rejected children,
        // has already been validated above.
        if matches!(rhs, Ty::Param(_)) {
            if goal_is_closed(&projection.trait_ref, env)? {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: closed equality cannot infer a rigid RHS parameter",
                ));
            }
            return Ok(());
        }
        if !self.raw_associated_ty_is_closed(rhs, 0)? {
            // Symbolic RHS terms are metadata, not executable or concrete
            // assignments. Their contextual membership/Sized proof is checked
            // when the immutable predicate environment is formed or consumed.
            self.validate_equality_reference_domain(rhs, 0)?;
            return Ok(());
        }
        if !self.shared_assignment_metadata(projection, rhs, env)? {
            self.validate_equality_reference_domain(rhs, 0)?;
        }
        if matches!(rhs, Ty::AssociatedType(_)) {
            // A closed projection RHS (the `Item = <IntoIter as Iterator>::Item`
            // bound of a concrete `IntoIterator` impl) is metadata here, like a
            // symbolic RHS: it reduces through the closed impl set when the
            // equality is consumed, and executable validation still rejects
            // any closed projection that never normalizes.
            return Ok(());
        }
        if caller.is_none() {
            self.validate_ty(rhs, env, None, true, 0)
        } else {
            self.validate_predicate_formation(rhs, env, TypeUse::Root, None, replays, caller)
        }
    }

    // Equality rows describe types, not loans. In particular a shared reference
    // RHS does not authorize a projected executable return or assign an origin:
    // those still require the exact callable source regions and carrier proof.
    // Keep mutable and nested references outside this domain until their
    // associated equality carrier contract is implemented.
    fn validate_equality_reference_domain(&self, ty: &Ty, depth: usize) -> Result<()> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match ty {
            Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } => {
                // A shared slice equality describes a descriptor type, not a loan.
                // Executable origins still come from the callable carrier proof.
                let referent = match referent.as_ref() {
                    Ty::Slice { element } => element.as_ref(),
                    other => other,
                };
                self.reject_projected_references(referent, depth + 1)
            }
            Ty::Ref { .. } | Ty::Slice { .. } => Err(FactsError::unsupported(
                &self.budget,
                "unsupported: mutable or nested reference associated equality",
            )),
            Ty::Array { element, .. } => {
                self.validate_equality_reference_domain(element, depth + 1)
            }
            Ty::Struct(value) => {
                for field in nominal_fields(&value.def().fields, &self.budget)?.iter() {
                    self.validate_equality_reference_domain(&field.ty, depth + 1)?;
                }
                for argument in type_arguments(ty) {
                    self.validate_equality_reference_domain(argument, depth + 1)?;
                }
                Ok(())
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in nominal_fields(&variant.fields, &self.budget)?.iter() {
                        self.validate_equality_reference_domain(&field.ty, depth + 1)?;
                    }
                }
                for argument in type_arguments(ty) {
                    self.validate_equality_reference_domain(argument, depth + 1)?;
                }
                Ok(())
            }
            _ => {
                for argument in type_arguments(ty) {
                    self.validate_equality_reference_domain(argument, depth + 1)?;
                }
                Ok(())
            }
        }
    }

    fn checked_assignment_source(
        &self,
        value: &AssociatedTypeValue,
    ) -> Result<Arc<AssociatedTypeSource>> {
        let source = self
            .schema
            .associated_type_source(value.implementation, value.associated)?;
        let retained = &value.source;
        if source.implementation != value.implementation
            || source.associated != value.associated
            || !self.same_env(&source.env, &self.info(value.implementation)?.env)?
            || source.region_count as usize > ir::MAX_REGION_PARAMS
            || source.implementation != retained.implementation
            || source.associated != retained.associated
            || !self.same_env(&source.env, &retained.env)?
            || source.region_count != retained.region_count
            || source.outlives != retained.outlives
            || source.references != retained.references
            || source.parameter_tail != retained.parameter_tail
        {
            return Err(FactsError::invalid(
                &self.budget,
                "associated assignment source authority changed",
            ));
        }
        let check_region = |region: &AssociatedSourceRegion| -> Result<()> {
            charged(&self.budget, self.budget.charge_work(1))?;
            if let AssociatedSourceRegion::Named(id) = region
                && (id.declaration != value.implementation || id.index >= source.region_count)
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated assignment has foreign source lifetime",
                ));
            }
            Ok(())
        };
        let mut tail = &value.rhs;
        for (kind, region) in source.references.iter() {
            check_region(region)?;
            let Ty::Ref {
                kind: actual,
                referent,
            } = tail
            else {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated source reference shape changed",
                ));
            };
            if actual != kind {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated source reference kind changed",
                ));
            }
            tail = referent;
        }
        for (longer, shorter) in source.outlives.iter() {
            check_region(longer)?;
            check_region(shorter)?;
        }
        if let Some(parameter) = source.parameter_tail
            && (tail != &Ty::Param(parameter)
                || !source.env.ordered_params().any(|p| p == parameter))
        {
            return Err(FactsError::invalid(
                &self.budget,
                "associated source generic binder changed",
            ));
        }
        Ok(source)
    }

    /// Domain admission is not membership: candidate predicates are still
    /// discharged by the enclosing selection, and equality is rechecked from
    /// that selection's witness. This stage must not recursively select itself.
    fn shared_assignment_metadata(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        env: &GenericEnv,
    ) -> Result<bool> {
        let tuple = matches!(rhs, Ty::Tuple(fields) if !fields.is_empty());
        if tuple {
            if !self.shared_assignment_payload(rhs, 0)? {
                return Ok(false);
            }
        } else {
            let Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: inner,
            } = rhs
            else {
                return Ok(false);
            };
            if matches!(
                inner.as_ref(),
                Ty::Tuple(_) | Ty::Struct(_) | Ty::Enum(_) | Ty::Array { .. }
            ) {
                if !self.shared_assignment_payload(inner, 0)? {
                    return Ok(false);
                }
            } else {
                let Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: leaf,
                } = inner.as_ref()
                else {
                    return Ok(false);
                };
                // Keep the original deeper-root boundary. A shared aggregate
                // actual above instead replays the same original &'a T tail.
                match self.reject_projected_references(leaf, 0) {
                    Ok(()) => {}
                    Err(error) if error.kind() == FactsErrorKind::Unsupported => return Ok(false),
                    Err(error) => return Err(error),
                }
            }
        }
        let mut selected = false;
        let Some(candidates) = self.trait_impl_index.get(&projection.trait_ref.declaration) else {
            return Ok(false);
        };
        for &index in candidates {
            charged(&self.budget, self.budget.charge_work(1))?;
            let header = self.headers.impls.get(index).ok_or_else(|| {
                FactsError::invalid(
                    &self.budget,
                    "associated metadata implementation index changed",
                )
            })?;
            let mut slots = Vec::new();
            for _ in header.schema_env.ordered_params() {
                push(&self.budget, &mut slots, None)?;
            }
            if !self.match_head(
                &header.trait_ref.self_ty,
                &projection.trait_ref.self_ty,
                &header.schema_env,
                &mut slots,
                0,
            )? || header.trait_ref.args.len() != projection.trait_ref.args.len()
            {
                continue;
            }
            let mut matches = true;
            for (raw, actual) in header
                .trait_ref
                .args
                .iter()
                .zip(projection.trait_ref.args.iter())
            {
                if !self.match_head(raw, actual, &header.schema_env, &mut slots, 0)? {
                    matches = false;
                    break;
                }
            }
            if !matches || slots.iter().any(Option::is_none) {
                continue;
            }
            if selected {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated metadata has ambiguous implementation heads",
                ));
            }
            let value = self.associated_value(header.declaration, projection.item)?;
            let source = self.checked_assignment_source(value)?;
            if tuple {
                // A source tuple of shared parameter tails retains its original
                // declaration and every ordered substitution. This is metadata
                // admission only, never a return-origin or executable loan proof.
                let Ty::Tuple(fields) = &value.rhs else {
                    return Ok(false);
                };
                if fields.is_empty()
                    || !source.references.is_empty()
                    || source.parameter_tail.is_some()
                {
                    return Ok(false);
                }
                for field in fields.iter() {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if !matches!(field, Ty::Ref { kind: ir::BorrowKind::Shared, referent }
                        if matches!(referent.as_ref(), Ty::Param(p) if header.schema_env.contains(*p)))
                    {
                        return Ok(false);
                    }
                }
            } else {
                let Some(parameter) = source.parameter_tail else {
                    return Ok(false);
                };
                if source.references.len() != 1
                    || !matches!(
                        source.references[0],
                        (ir::BorrowKind::Shared, AssociatedSourceRegion::Named(_))
                    )
                    || !matches!(&value.rhs, Ty::Ref { kind: ir::BorrowKind::Shared, referent } if referent.as_ref() == &Ty::Param(parameter))
                {
                    return Ok(false);
                }
            }
            let mut args = Vec::new();
            for slot in slots {
                push(
                    &self.budget,
                    &mut args,
                    slot.ok_or_else(|| {
                        FactsError::invalid(
                            &self.budget,
                            "associated metadata lost a matched argument",
                        )
                    })?,
                )?;
            }
            let raw = self
                .schema
                .associated_type_rhs(header.declaration, projection.item)?;
            if !self.eq_ty(&raw, &value.rhs, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "associated metadata RHS source changed",
                ));
            }
            retain::<ir::RefContract>(&self.budget, 1)?;
            let schema = GenericSchema {
                env: header.schema_env.clone(),
                signature: Signature {
                    params: Vec::new(),
                    result: raw.clone(),
                    ref_contract: Arc::new(ir::RefContract::default()),
                },
                predicates: Arc::from([]),
            };
            let instance = self
                .schema
                .instantiate_schema(&schema, &args, env, &self.budget)?;
            self.validate_rhs_substitution(
                &raw,
                &instance.signature.result,
                &schema.env,
                &args,
                0,
            )?;
            if !self.eq_ty(&instance.signature.result, rhs, 0)? {
                return Ok(false);
            }
            selected = true;
        }
        Ok(selected)
    }

    /// Closed shared payloads carried by a source-owned tuple substitution.
    /// Mutable/slice and unresolved carriers keep their existing boundaries.
    /// The enclosing metadata replay authenticates the full nominal definitions.
    fn shared_assignment_payload(&self, ty: &Ty, depth: usize) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        match ty {
            Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent,
            } => self.shared_assignment_payload(referent, depth + 1),
            Ty::Ref { .. }
            | Ty::Slice { .. }
            | Ty::Param(_)
            | Ty::AssociatedType(_)
            | Ty::Closure(_)
            | Ty::Dyn(_)
            | Ty::Opaque(_) => Ok(false),
            Ty::Array { element, .. } => self.shared_assignment_payload(element, depth + 1),
            Ty::Struct(value) => {
                for field in nominal_fields(&value.def().fields, &self.budget)?.iter() {
                    if !self.shared_assignment_payload(&field.ty, depth + 1)? {
                        return Ok(false);
                    }
                }
                for argument in type_arguments(ty) {
                    if !self.shared_assignment_payload(argument, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in nominal_fields(&variant.fields, &self.budget)?.iter() {
                        if !self.shared_assignment_payload(&field.ty, depth + 1)? {
                            return Ok(false);
                        }
                    }
                }
                for argument in type_arguments(ty) {
                    if !self.shared_assignment_payload(argument, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => {
                for argument in type_arguments(ty) {
                    if !self.shared_assignment_payload(argument, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }

    pub(super) fn validate_equality_domain_in(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        self.check_context(context)?;
        self.validate_equality_domain_with_active(projection, rhs, context.env(), active, None)?;
        self.require_equality_rhs_sized(rhs, context, active)
    }

    fn require_equality_rhs_sized(
        &self,
        rhs: &Ty,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        let sized = Predicate::Implements(TraitRef {
            declaration: self.catalog.id(CatalogItem::Sized),
            self_ty: rhs.clone(),
            args: Arc::from([]),
        });
        self.require_in(&sized, context, active)?;
        Ok(())
    }

    pub(super) fn validate_declared_equality_domains(
        &self,
        rows: &[Predicate],
        env: &GenericEnv,
    ) -> Result<()> {
        for row in rows {
            let Predicate::ProjectionEq { projection, ty } = row else {
                continue;
            };
            self.validate_equality_domain(projection, ty, env)?;
            if !self.raw_associated_ty_is_closed(ty, 0)? {
                if !self.declared_rhs_proves_sized(ty, rows, env, 0)? {
                    return Err(FactsError::unproved(
                        &self.budget,
                        "declared equality RHS lacks genuine contextual Sized evidence",
                    ));
                }
            } else {
                self.require_equality_rhs_sized(
                    ty,
                    ProofContext {
                        original: None,
                        predicates: &self.empty,
                        phase: FactsPhase::Closed,
                    },
                    &mut ProofScratch::default(),
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn validate_equality_conflicts(&self, rows: &[Predicate]) -> Result<()> {
        for (index, row) in rows.iter().enumerate() {
            if let Predicate::ProjectionEq { projection, ty } = row {
                for other in &rows[..index] {
                    charged(&self.budget, self.budget.charge_work(1))?;
                    if let Predicate::ProjectionEq {
                        projection: old,
                        ty: rhs,
                    } = other
                        && self.same_projection(projection, old)?
                        && !self.eq_ty(ty, rhs, 0)?
                    {
                        return Err(FactsError::unsupported(
                            &self.budget,
                            "unsupported: conflicting declared projection equalities",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn declared_equality_rhs(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rows: &[Predicate],
        env: &GenericEnv,
    ) -> Result<Option<Ty>> {
        // Validate every written row before either conflict detection or selection.
        self.validate_predicates(rows, env, None)?;
        self.reduce_declared_equality_rhs(projection, rows, env, &mut Vec::new())
    }
    pub fn normalize_projection_equality(
        &self,
        projection: &ir::AssociatedTypeProjection,
        facts: &ValidatedFacts,
    ) -> Result<NormalizedProjectionEquality> {
        self.check_facts(facts, false)?;
        self.check_normalization_budget()?;
        if !self.owner_normalization_finalized
            || facts.phase() != FactsPhase::Universal
            || self.goal_is_closed(&projection.trait_ref, facts.env())?
        {
            return Err(FactsError::unsupported(
                &self.budget,
                "declared equality normalization requires a finalized rigid universal caller",
            ));
        }
        self.validate_final_equality_context(facts)?;
        let rhs = self
            .declared_equality_rhs(projection, &facts.predicates.predicates, facts.env())?
            .ok_or_else(|| {
                FactsError::unsupported(
                    &self.budget,
                    "unsupported: rigid projection has no exact declared equality",
                )
            })?;
        let membership = self.require_in(
            &Predicate::Implements(projection.trait_ref.clone()),
            ProofContext::from_facts(facts),
            &mut ProofScratch::default(),
        )?;
        retain::<ProjectionEqualityEvidence>(&self.budget, 1)?;
        let proof = NormalizedProjectionEquality(Arc::new(ProjectionEqualityEvidence {
            world: self.id,
            environment: facts.predicates.index,
            phase: facts.phase(),
            declaration: facts.predicates.declaration,
            projection: projection.clone(),
            rhs,
            rows: facts.predicates.predicates.clone(),
            membership,
            validated_env: facts.predicates.env.clone(),
            validated_registry: self.registry,
            validated_ready: self.owner_normalization_ready,
            validated_finalized: self.owner_normalization_finalized,
            validated_limits: projection_formation_limits(&self.budget),
        }));
        proof.validate_for(projection, facts)?;
        Ok(proof)
    }
    pub(super) fn normalize_projection_for_facts(
        &self,
        projection: &ir::AssociatedTypeProjection,
        facts: &ValidatedFacts,
    ) -> Result<ProjectionReduction> {
        if self.checked_closure_projection(projection, facts.env())?
            || self
                .opaque_associated_binding(projection, facts.env())?
                .is_some()
            || self.goal_is_closed(&projection.trait_ref, facts.env())?
            || matches!(projection.trait_ref.self_ty, Ty::Struct(_) | Ty::Enum(_))
        {
            Ok(ProjectionReduction::Implementation(
                self.normalize_associated_type(projection, facts)?,
            ))
        } else {
            Ok(ProjectionReduction::Equality(
                self.normalize_projection_equality(projection, facts)?,
            ))
        }
    }
    pub(super) fn prove_projection_equality(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<ProofOutcome> {
        self.check_context(context)?;
        self.validate_equality_domain_in(projection, rhs, context, active)?;
        let goal = Predicate::ProjectionEq {
            projection: projection.clone(),
            ty: rhs.clone(),
        };
        if let Ty::AssociatedType(rhs_projection) = rhs
            && self.same_projection(projection, rhs_projection)?
        {
            let membership = self.require_in(
                &Predicate::Implements(projection.trait_ref.clone()),
                context,
                active,
            )?;
            return self.evidence(
                &goal,
                context,
                ProofOrigin::ReflexiveEquality { membership },
            );
        }
        if self.checked_closure_projection(projection, context.env())?
            || self
                .opaque_associated_binding(projection, context.env())?
                .is_some()
            || self.goal_is_closed_with(&projection.trait_ref, context.env(), active)?
            || matches!(projection.trait_ref.self_ty, Ty::Struct(_) | Ty::Enum(_))
        {
            let normalization =
                self.normalize_closed_equality_projection(projection, context, active)?;
            let normal = normalization.ty();
            self.validate_equality_domain_in(projection, normal, context, active)?;
            if !self.eq_ty(normal, rhs, 0)? {
                return Ok(ProofOutcome::NotProven);
            }
            return self.evidence(
                &goal,
                context,
                ProofOrigin::ClosedEquality { normalization },
            );
        }
        let Some(normal) =
            self.declared_equality_rhs(projection, &context.predicates.predicates, context.env())?
        else {
            let Some((membership, normal)) =
                self.rigid_nominal_implementation_rhs(projection, context, active)?
            else {
                return Ok(ProofOutcome::NotProven);
            };
            if !self.eq_ty(&normal, rhs, 0)? {
                return Ok(ProofOutcome::NotProven);
            }
            return self.evidence(
                &goal,
                context,
                ProofOrigin::ImplementationEquality { membership },
            );
        };
        if !self.eq_ty(&normal, rhs, 0)? {
            return Ok(ProofOutcome::NotProven);
        }
        self.require_in(
            &Predicate::Implements(projection.trait_ref.clone()),
            context,
            active,
        )?;
        self.evidence(&goal, context, ProofOrigin::DeclaredEquality)
    }
    /// Equality-only selection for a rigid source nominal or reference subject,
    /// such as `slice::Iter<T>` or a borrowed opaque callable. The impl is chosen
    /// by canonical head matching in the genuine caller context, and its RHS
    /// is reminted from the immutable declaration callback. This never grants
    /// a carrier, layout or origin.
    pub(super) fn rigid_nominal_implementation_rhs(
        &self,
        projection: &ir::AssociatedTypeProjection,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<(ProofEvidence, Ty)>> {
        if !matches!(
            projection.trait_ref.self_ty,
            Ty::Struct(_) | Ty::Enum(_) | Ty::Ref { .. }
        ) || self.goal_is_closed_with(&projection.trait_ref, context.env(), active)?
            || self.type_contains_projection(&projection.trait_ref.self_ty, 0)?
        {
            return Ok(None);
        }
        for argument in projection.trait_ref.args.iter() {
            if self.type_contains_projection(argument, 0)? {
                return Ok(None);
            }
        }
        let membership = match self.prove_implementation_in(
            &Predicate::Implements(projection.trait_ref.clone()),
            context,
            active,
        )? {
            ProofOutcome::Proved(evidence) => evidence,
            ProofOutcome::NotProven => return Ok(None),
        };
        let normal = self.implementation_equality_rhs(projection, &membership, context)?;
        Ok(Some((membership, normal)))
    }

    /// Replays an implementation-selected equality from its membership record.
    pub(super) fn implementation_equality_rhs(
        &self,
        projection: &ir::AssociatedTypeProjection,
        membership: &ProofEvidence,
        context: ProofContext<'_>,
    ) -> Result<Ty> {
        let record = &membership.0;
        let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &record.origin
        else {
            return Err(FactsError::invalid(
                &self.budget,
                "implementation equality lacks an implementation witness",
            ));
        };
        if record.world != self.id
            || record.environment != context.predicates.index
            || record.phase != context.phase
            || !self.eq_predicate(
                &record.goal,
                &Predicate::Implements(projection.trait_ref.clone()),
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "implementation equality changed world, caller context or application",
            ));
        }
        let header = self
            .headers
            .impls
            .iter()
            .find(|header| header.declaration == *declaration)
            .ok_or_else(|| {
                FactsError::invalid(&self.budget, "implementation equality lost its impl")
            })?;
        if header.trait_ref.declaration != projection.trait_ref.declaration
            || arguments.len() != header.schema_env.ordered_params().count()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "implementation equality changed trait or binder arity",
            ));
        }
        let value = self.associated_value(*declaration, projection.item)?;
        self.checked_assignment_source(value)?;
        let raw_rhs = self
            .schema
            .associated_type_rhs(*declaration, projection.item)?;
        if !self.eq_ty(&raw_rhs, &value.rhs, 0)? {
            return Err(FactsError::invalid(
                &self.budget,
                "implementation equality RHS callback changed",
            ));
        }
        retain::<ir::RefContract>(&self.budget, 1)?;
        let schema = GenericSchema {
            env: header.schema_env.clone(),
            signature: Signature {
                params: Vec::new(),
                result: raw_rhs.clone(),
                ref_contract: Arc::new(ir::RefContract::default()),
            },
            predicates: Arc::from([]),
        };
        let instantiated =
            self.schema
                .instantiate_schema(&schema, arguments, context.env(), &self.budget)?;
        self.validate_rhs_substitution(
            &raw_rhs,
            &instantiated.signature.result,
            &header.schema_env,
            arguments,
            0,
        )?;
        if !instantiated.signature.params.is_empty() || !instantiated.predicates.is_empty() {
            return Err(FactsError::invalid(
                &self.budget,
                "implementation equality substitution invented obligations",
            ));
        }
        // Residual projections stay symbolic only for rigid parameter subjects
        // whose membership the genuine caller context proves. Anything else
        // (closed or nominal nested projection) stays outside this path.
        let normal = self.map_equality_type(
            &instantiated.signature.result,
            context.env(),
            0,
            &mut |nested| {
                if !matches!(nested.trait_ref.self_ty, Ty::Param(_))
                    || self.goal_is_closed(&nested.trait_ref, context.env())?
                {
                    return Err(FactsError::unsupported(
                        &self.budget,
                        "unsupported: projected RHS in rigid implementation equality",
                    ));
                }
                self.require_in(
                    &Predicate::Implements(nested.trait_ref.clone()),
                    context,
                    &mut ProofScratch::default(),
                )?;
                self.projection_type(nested)
            },
        )?;
        self.validate_ty_metadata_in(&normal, context.env(), 0)?;
        Ok(normal)
    }
    pub(super) fn build_declared_callable_contexts(&mut self) -> Result<()> {
        for callable in self.headers.callables.iter() {
            self.validate_declaration_schema(callable.declaration, &callable.schema)?;
            let mut rows = Vec::new();
            let mut origins = Vec::new();
            self.retain_equality_origins(
                callable.declaration,
                &callable.schema.predicates,
                &mut origins,
            )?;
            // Retain occurrences until all supplied rows have been validated.
            for row in callable.schema.predicates.iter() {
                push(&self.budget, &mut rows, row.clone())?;
            }
            let owner = self.info(callable.declaration)?.owner;
            for implementation in self.headers.impls.iter() {
                if Some(implementation.declaration) == owner {
                    self.retain_equality_origins(
                        implementation.declaration,
                        &implementation.predicates,
                        &mut origins,
                    )?;
                    for row in implementation.predicates.iter() {
                        push(&self.budget, &mut rows, row.clone())?;
                    }
                    self.append_predicate(
                        &mut rows,
                        &Predicate::Implements(implementation.trait_ref.clone()),
                    )?;
                }
            }
            if let Some(item) = self.source_default_item(callable.declaration)? {
                self.append_predicate(
                    &mut rows,
                    &Predicate::Implements(self.symbolic_trait_ref(item.trait_decl)?),
                )?;
            }
            self.implicit_sized(&callable.schema.env, &mut rows)?;
            self.validate_predicates(&rows, &callable.schema.env, None)?;
            self.close_predicates(&mut rows, &callable.schema.env)?;
            self.validate_declared_equality_domains(&rows, &callable.schema.env)?;
            retain::<DeclaredCallableContext>(&self.budget, 1)?;
            self.declared_callable_index.note(
                &self.budget,
                callable.declaration,
                self.declared_callables.len(),
            )?;
            push(
                &self.budget,
                &mut self.declared_callables,
                DeclaredCallableContext {
                    world: self.id,
                    stage: DeclaredStageId(self.id),
                    declaration: callable.declaration,
                    env: callable.schema.env.clone(),
                    predicates: rows.into(),
                    raw: clone_schema(&callable.schema, &self.budget)?,
                    origins: super::parameter_equality::result_slice(&self.budget, origins)?,
                    reductions: ReductionRows::default(),
                },
            )?;
        }
        Ok(())
    }
    pub(super) fn declared_callable_context(
        &self,
        declaration: DeclId,
    ) -> Result<&DeclaredCallableContext> {
        if let Some(position) = self
            .declared_callable_index
            .get(&self.budget, declaration)?
        {
            return Ok(&self.declared_callables[position]);
        }
        Err(FactsError::invalid(
            &self.budget,
            "missing private declared callable context",
        ))
    }
    pub(super) fn normalize_declared_callable_schema(
        &self,
        declaration: DeclId,
        raw: &GenericSchema,
    ) -> Result<DeclaredCallableNormalization> {
        let context = self.declared_callable_context(declaration)?;
        if context.world != self.id
            || context.stage != DeclaredStageId(self.id)
            || self.owner_normalization_finalized
            || !self.same_env(&context.env, &raw.env)?
            || !self.eq_signature(&context.raw.signature, &raw.signature)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "declared normalization changed stage or raw callable",
            ));
        }
        self.compare_predicate_rows(&context.raw.predicates, &raw.predicates)?;
        let mut schema = clone_schema(raw, &self.budget)?;
        let mut reductions = ReductionRows::default();
        for ty in &mut schema.signature.params {
            *ty = self.normalize_declared_type(ty, context, &mut reductions, 0)?;
        }
        schema.signature.result =
            self.normalize_declared_type(&schema.signature.result, context, &mut reductions, 0)?;
        self.validate_schema_signature(declaration, &schema.signature, &schema.env, None)?;
        Ok(DeclaredCallableNormalization {
            schema,
            world: self.id,
            stage: context.stage,
            declaration,
            reductions,
        })
    }
    fn normalize_declared_type(
        &self,
        ty: &Ty,
        context: &DeclaredCallableContext,
        reductions: &mut ReductionRows,
        depth: usize,
    ) -> Result<Ty> {
        self.map_equality_type(ty, &context.env, depth, &mut |projection| {
            if let Some(normal) =
                self.normalize_exact_owner_projection(context.declaration, projection)?
            {
                return Ok(normal);
            }
            if !self.goal_is_closed(&projection.trait_ref, &context.env)? {
                let rhs =
                    self.declared_equality_rhs(projection, &context.predicates, &context.env)?;
                // Membership is structural here, not a final proof. Closure includes only genuine declared memberships.
                let membership = Predicate::Implements(projection.trait_ref.clone());
                let mut found = false;
                for row in context.predicates.iter() {
                    found |= self.eq_predicate(row, &membership)?;
                }
                if !found {
                    return Err(FactsError::unproved(
                        &self.budget,
                        "declared equality lacks exact owner membership",
                    ));
                }
                let Some(rhs) = rhs else {
                    // An unrelated equality must not force this exact, independently
                    // declared family to acquire a concrete assignment. Retain its
                    // authenticated rigid identity, without recording a reduction.
                    self.validate_universal_projection(projection, &context.env)?;
                    return self.projection_type(projection);
                };
                reductions.push(
                    DeclaredEqualityReduction {
                        world: self.id,
                        stage: context.stage,
                        declaration: context.declaration,
                        projection: projection.clone(),
                        rhs: rhs.clone(),
                    },
                    &self.budget,
                )?;
                Ok(rhs)
            } else {
                self.normalize_closed_projected_type(
                    &self.projection_type(projection)?,
                    ProofContext {
                        original: None,
                        predicates: &self.empty,
                        phase: FactsPhase::Schema,
                    },
                    true,
                )
                .map(|value| value.0)
            }
        })
    }
    pub(super) fn map_equality_type(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
        reduce: &mut impl FnMut(&ir::AssociatedTypeProjection) -> Result<Ty>,
    ) -> Result<Ty> {
        self.check_normalization_budget()?;
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        Ok(match ty {
            Ty::AssociatedType(projection) => reduce(projection)?,
            Ty::Tuple(parts) => {
                let mut values = Vec::new();
                for part in parts.iter() {
                    push(
                        &self.budget,
                        &mut values,
                        self.map_equality_type(part, env, depth + 1, reduce)?,
                    )?;
                }
                Ty::Tuple(values.into())
            }
            Ty::Array { element, len } => {
                retain::<Ty>(&self.budget, 1)?;
                Ty::Array {
                    element: Arc::new(self.map_equality_type(element, env, depth + 1, reduce)?),
                    len: *len,
                }
            }
            Ty::Ref { kind, referent } => {
                retain::<Ty>(&self.budget, 1)?;
                Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(self.map_equality_type(referent, env, depth + 1, reduce)?),
                }
            }
            Ty::Slice { element } => {
                retain::<Ty>(&self.budget, 1)?;
                Ty::Slice {
                    element: Arc::new(self.map_equality_type(element, env, depth + 1, reduce)?),
                }
            }
            Ty::Struct(_) | Ty::Enum(_) => {
                self.normalize_nominal_actuals(ty, env, depth, |argument| {
                    self.map_equality_type(argument, env, depth + 1, reduce)
                })?
            }
            _ => ty.clone(),
        })
    }
    /// Alpha-renaming may close an owning associated type inside a method
    /// bound. Reduce only that exact staged owner's assignment before the
    /// ordinary predicate validator sees it. This grants no new membership.
    pub(super) fn normalize_required_owner_predicates(
        &self,
        declaration: DeclId,
        predicates: &[Predicate],
    ) -> Result<Arc<[Predicate]>> {
        let env = &self.info(declaration)?.env;
        let mut rows = Vec::new();
        for predicate in predicates {
            charged(&self.budget, self.budget.charge_work(1))?;
            let mut reduce = |projection: &ir::AssociatedTypeProjection| match self
                .normalize_exact_owner_projection(declaration, projection)?
            {
                Some(normal) => Ok(normal),
                None => self.projection_type(projection),
            };
            let application = predicate.trait_ref();
            let self_ty = self.map_equality_type(&application.self_ty, env, 0, &mut reduce)?;
            let mut args = Vec::new();
            for argument in application.args.iter() {
                push(
                    &self.budget,
                    &mut args,
                    self.map_equality_type(argument, env, 0, &mut reduce)?,
                )?;
            }
            let application = TraitRef {
                declaration: application.declaration,
                self_ty,
                args: args.into(),
            };
            let row = match predicate {
                Predicate::Implements(_) => Predicate::Implements(application),
                Predicate::ProjectionEq { projection, ty } => Predicate::ProjectionEq {
                    // Normalize operands, never discharge this outer equality.
                    // Its original associated item and obligation remain.
                    projection: ir::AssociatedTypeProjection {
                        trait_ref: application,
                        item: projection.item,
                    },
                    ty: self.map_equality_type(ty, env, 0, &mut reduce)?,
                },
            };
            push(&self.budget, &mut rows, row)?;
        }
        super::parameter_equality::result_slice(&self.budget, rows)
    }

    pub(super) fn normalize_equality_type(&self, ty: &Ty, facts: &ValidatedFacts) -> Result<Ty> {
        self.check_facts(facts, false)?;
        self.validate_ty_metadata_in(ty, facts.env(), 0)?;
        self.map_universal_projection_type(ty, facts.env(), 0, &mut |projection| {
            if let FactsSubject::UniversalCallable(declaration) = facts.subject
                && let Some(normal) =
                    self.normalize_exact_owner_projection(declaration, projection)?
            {
                return Ok(normal);
            }
            if !self.checked_closure_projection(projection, facts.env())?
                && !self.goal_is_closed(&projection.trait_ref, facts.env())?
                && self
                    .declared_equality_rhs(projection, &facts.predicates.predicates, facts.env())?
                    .is_none()
            {
                if facts.phase() != FactsPhase::Universal {
                    return Err(FactsError::invalid(
                        &self.budget,
                        "unreduced projection requires a universal caller",
                    ));
                }
                self.validate_universal_projection(projection, facts.env())?;
                self.require_in(
                    &Predicate::Implements(projection.trait_ref.clone()),
                    ProofContext::from_facts(facts),
                    &mut ProofScratch::default(),
                )?;
                return Ok(Ty::AssociatedType(Arc::new(projection.clone())));
            }
            Ok(self
                .normalize_projection_for_facts(projection, facts)?
                .ty()
                .clone())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};

    // Genuine source-owned caller, projection, RHS parameter and opaque return.
    struct ActiveEqualityCase {
        world: Arc<ValidatedTraitWorld>,
        facts: Arc<ValidatedFacts>,
        projection: ir::AssociatedTypeProjection,
        rhs: Ty,
        opaque: Ty,
    }
    fn active_equality_fixture() -> ActiveEqualityCase {
        let source = frontend::parse(
            "trait Has{type Out;}struct Number{value:i32}impl Has for Number{type Out=i32;}fn probe<T:Has<Out=U>,U>(x:T,y:U){}fn make()->impl Has<Out=i32>{Number{value:0}}fn main(){}",
        ).unwrap();
        let plan = PlanCtx::from_program(&source, &PlanBudget::default()).unwrap();
        let world = plan.trait_world().unwrap();
        let facts = world
            .callable_facts(plan.function_decl(0).unwrap())
            .unwrap();
        let (projection, rhs) = facts
            .predicates
            .predicates
            .iter()
            .find_map(|row| match row {
                Predicate::ProjectionEq { projection, ty } => {
                    Some((projection.clone(), ty.clone()))
                }
                _ => None,
            })
            .unwrap();
        assert!(matches!(rhs, Ty::Param(_)));
        let opaque = world
            .schema
            .declaration_schema(plan.function_decl(1).unwrap())
            .unwrap()
            .signature
            .result
            .clone();
        assert!(matches!(opaque, Ty::Opaque(_)));
        ActiveEqualityCase {
            world,
            facts,
            projection,
            rhs,
            opaque,
        }
    }

    // Substitute a genuine declaration into its original ordered caller binder.
    // This is a predicate batch, not copied synthetic source metadata.
    fn active_predicate_batch(case: &ActiveEqualityCase) -> Vec<Predicate> {
        let schema = case
            .world
            .schema
            .declaration_schema(case.facts.declaration())
            .unwrap();
        let args = case
            .facts
            .env()
            .ordered_params()
            .map(Ty::Param)
            .collect::<Vec<_>>();
        case.world
            .schema
            .instantiate_schema(&schema, &args, case.facts.env(), case.facts.budget())
            .unwrap()
            .predicates
            .to_vec()
    }

    #[test]
    fn active_predicate_batch_warm_context_metadata_and_conflicts() {
        let case = active_equality_fixture();
        let rows = active_predicate_batch(&case);
        let mut active = ProofScratch::default();
        for _ in 0..2 {
            case.world
                .validate_predicates_with_active(
                    &rows,
                    case.facts.env(),
                    None,
                    &mut active,
                    Some(&case.facts),
                )
                .unwrap();
        }
        let completed = active.closedness.len();
        assert!(completed > 0);
        let schema = case
            .world
            .schema
            .declaration_schema(case.facts.declaration())
            .unwrap();
        let arguments = case
            .facts
            .env()
            .ordered_params()
            .map(Ty::Param)
            .collect::<Vec<_>>();
        let mut instantiated = ProofScratch::default();
        for _ in 0..2 {
            case.world
                .instantiate_scratch_with_facts(
                    &schema,
                    &arguments,
                    ProofContext::from_facts(&case.facts),
                    Some(case.facts.declaration()),
                    &mut instantiated,
                    Some(&case.facts),
                )
                .unwrap();
        }
        assert!(!instantiated.checked.as_ref().unwrap().closedness.is_empty());
        let bound = active.replays.predicate_formations.caller_context;
        let formations = active.replays.predicate_formations.complete.len();
        // Raw admission cannot rebind the original caller-aware formation cache.
        case.world
            .validate_predicates_with_active(&rows, case.facts.env(), None, &mut active, None)
            .unwrap();
        assert_eq!(active.replays.predicate_formations.caller_context, bound);
        assert_eq!(
            active.replays.predicate_formations.complete.len(),
            formations
        );
        let foreign = active_equality_fixture();
        assert!(
            foreign
                .world
                .validate_predicates_with_active(
                    &rows,
                    case.facts.env(),
                    None,
                    &mut active,
                    Some(&case.facts)
                )
                .is_err()
        );
        assert!(
            case.world
                .validate_predicates_with_active(
                    &rows,
                    foreign.facts.env(),
                    None,
                    &mut active,
                    Some(&case.facts)
                )
                .is_err()
        );
        let mut changed = case.facts.as_ref().clone();
        changed.predicates = case.world.empty.clone();
        assert!(
            case.world
                .validate_predicates_with_active(
                    &rows,
                    case.facts.env(),
                    None,
                    &mut active,
                    Some(&changed)
                )
                .is_err()
        );
        assert_eq!(active.closedness.len(), completed);
        // A valid early equality must not run its domain check before late metadata.
        let mut invalid = rows.clone();
        let mut forged = case.projection.clone();
        forged.item = case.facts.declaration();
        invalid.push(Predicate::ProjectionEq {
            projection: forged,
            ty: case.rhs.clone(),
        });
        let mut cold = ProofScratch::default();
        assert!(
            case.world
                .validate_predicates_with_active(
                    &invalid,
                    case.facts.env(),
                    None,
                    &mut cold,
                    Some(&case.facts)
                )
                .is_err()
        );
        assert!(cold.closedness.is_empty());
        assert!(
            case.world
                .validate_predicates_with_active(
                    &invalid,
                    case.facts.env(),
                    None,
                    &mut active,
                    Some(&case.facts)
                )
                .is_err()
        );
        assert_eq!(active.closedness.len(), completed);
        // Duplicates keep canonical insertion policy. Contradictory RHS rows keep
        // conflict rejection even after genuine successes have warmed the arena.
        let mut duplicate = rows.clone();
        duplicate.extend(rows.iter().cloned());
        case.world
            .validate_predicates_with_active(
                &duplicate,
                case.facts.env(),
                None,
                &mut active,
                Some(&case.facts),
            )
            .unwrap();
        let mut conflict = rows.clone();
        conflict.push(Predicate::ProjectionEq {
            projection: case.projection.clone(),
            ty: Ty::Bool,
        });
        let fresh = case
            .world
            .validate_predicates_with_facts(
                &conflict,
                case.facts.env(),
                None,
                &mut NominalReplays::default(),
                Some(&case.facts),
            )
            .unwrap_err();
        let warm = case
            .world
            .validate_predicates_with_active(
                &conflict,
                case.facts.env(),
                None,
                &mut active,
                Some(&case.facts),
            )
            .unwrap_err();
        assert_eq!(fresh.kind(), warm.kind());
        // Root cache entries do not collapse the distinct Actual usage key.
        let prior = active.closedness.len();
        case.world
            .closedness_of_root_with_storage(
                &case.projection.trait_ref.self_ty,
                case.facts.env(),
                TypeUse::Actual,
                0,
                &mut active.closedness,
                &mut active.closedness_storage,
            )
            .unwrap();
        assert_eq!(active.closedness.len(), prior + 1);
    }

    #[test]
    fn active_predicate_batch_exact_warm_work_and_poison() {
        for short in [false, true] {
            let case = active_equality_fixture();
            let rows = active_predicate_batch(&case);
            let mut active = ProofScratch::default();
            for _ in 0..2 {
                case.world
                    .validate_predicates_with_active(
                        &rows,
                        case.facts.env(),
                        None,
                        &mut active,
                        Some(&case.facts),
                    )
                    .unwrap();
            }
            let count = active.closedness.len();
            let before = case.facts.budget().usage_snapshot().work;
            case.world
                .validate_predicates_with_active(
                    &rows,
                    case.facts.env(),
                    None,
                    &mut active,
                    Some(&case.facts),
                )
                .unwrap();
            let cost = case
                .facts
                .budget()
                .usage_snapshot()
                .work
                .checked_sub(before)
                .unwrap();
            assert!(cost > 0);
            let filler = case
                .facts
                .budget()
                .limits()
                .work
                .checked_sub(case.facts.budget().usage_snapshot().work)
                .and_then(|remaining| remaining.checked_sub(cost))
                .and_then(|remaining| remaining.checked_add(usize::from(short)))
                .unwrap();
            case.facts.budget().charge_work(filler).unwrap();
            let result = case.world.validate_predicates_with_active(
                &rows,
                case.facts.env(),
                None,
                &mut active,
                Some(&case.facts),
            );
            assert_eq!(result.is_err(), short);
            assert_eq!(active.closedness.len(), count);
            if short {
                let failure = case.facts.budget().first_resource_failure();
                assert_eq!(failure.unwrap().kind, ir::ResourceBudgetKind::Work);
                assert!(
                    case.world
                        .validate_predicates_with_active(
                            &rows,
                            case.facts.env(),
                            None,
                            &mut active,
                            Some(&case.facts)
                        )
                        .is_err()
                );
                assert_eq!(case.facts.budget().first_resource_failure(), failure);
            }
        }
        for planner in [false, true] {
            let case = active_equality_fixture();
            let rows = active_predicate_batch(&case);
            let mut active = ProofScratch::default();
            case.world
                .validate_predicates_with_active(
                    &rows,
                    case.facts.env(),
                    None,
                    &mut active,
                    Some(&case.facts),
                )
                .unwrap();
            let count = active.closedness.len();
            if planner {
                case.facts.budget().poison_ledger_for_tests();
            } else {
                assert!(
                    case.world
                        .catalog
                        .modules()
                        .budget()
                        .charge(crate::compiler::expansion_budget::Counter::Work, u64::MAX)
                        .is_err()
                );
            }
            let failure = case.facts.budget().first_resource_failure();
            for _ in 0..2 {
                assert!(
                    matches!(case.world.validate_predicates_with_active(&rows, case.facts.env(), None, &mut active, Some(&case.facts)), Err(error) if error.is_resource())
                );
                assert_eq!(active.closedness.len(), count);
                assert_eq!(case.facts.budget().first_resource_failure(), failure);
            }
        }
    }

    #[test]
    fn active_predicate_batch_actual_peak_exact_and_short() {
        let case = active_equality_fixture();
        let rows = active_predicate_batch(&case);
        let before = case.facts.budget().usage_snapshot();
        assert_eq!(before.bytes, before.reservations.current_bytes);
        assert!(!before.reservations.counters_saturated);
        let lift = case
            .facts
            .budget()
            .reserve_bytes(
                before
                    .reservations
                    .peak_bytes
                    .checked_sub(before.bytes)
                    .unwrap(),
            )
            .unwrap();
        let window = case.facts.budget().usage_snapshot();
        assert_eq!(window.bytes, window.reservations.peak_bytes);
        let mut measured = ProofScratch::default();
        case.world
            .validate_predicates_with_active(
                &rows,
                case.facts.env(),
                None,
                &mut measured,
                Some(&case.facts),
            )
            .unwrap();
        let after = case.facts.budget().usage_snapshot();
        assert_eq!(after.bytes, after.reservations.current_bytes);
        assert!(!after.reservations.counters_saturated);
        let peak = after
            .reservations
            .peak_bytes
            .checked_sub(window.bytes)
            .unwrap();
        let net = after.bytes.checked_sub(window.bytes).unwrap();
        assert!(peak >= net && net > 0);
        drop(measured);
        drop(lift);
        for short in [false, true] {
            let current = case.facts.budget().usage_snapshot().bytes;
            let headroom = peak.checked_sub(usize::from(short)).unwrap();
            let filler_bytes = case
                .facts
                .budget()
                .limits()
                .bytes
                .checked_sub(current)
                .and_then(|available| available.checked_sub(headroom))
                .unwrap();
            let filler = case.facts.budget().reserve_bytes(filler_bytes).unwrap();
            let mut active = ProofScratch::default();
            let result = case.world.validate_predicates_with_active(
                &rows,
                case.facts.env(),
                None,
                &mut active,
                Some(&case.facts),
            );
            assert_eq!(result.is_err(), short);
            if short {
                assert_eq!(
                    case.facts.budget().first_resource_failure().unwrap().kind,
                    ir::ResourceBudgetKind::Bytes
                );
            }
            let failure = case.facts.budget().first_resource_failure();
            drop(active);
            let owner_dropped = case.facts.budget().usage_snapshot().bytes;
            drop(filler);
            assert_eq!(
                case.facts.budget().usage_snapshot().bytes,
                owner_dropped - filler_bytes
            );
            assert_eq!(case.facts.budget().first_resource_failure(), failure);
        }
    }

    #[test]
    fn active_predicate_batch_owned_closedness_drop_and_explicit_poison() {
        for poison in [false, true] {
            let case = active_equality_fixture();
            let rows = active_predicate_batch(&case);
            let parent = case.facts.budget().reserve_bytes(17).unwrap();
            let mut active = ProofScratch::default();
            case.world
                .validate_predicates_with_active(
                    &rows,
                    case.facts.env(),
                    None,
                    &mut active,
                    Some(&case.facts),
                )
                .unwrap();
            let count = active.closedness.len();
            assert!(count > 0);
            assert_eq!(active.closedness_storage.len(), count);
            let paid = scratch_row_bytes::<ClosednessValidation>()
                .unwrap()
                .checked_mul(count)
                .unwrap();
            // Measure only these moved fields. The remaining arena fields are
            // unchanged and drop at the end of their original scope.
            let ProofScratch {
                closedness,
                closedness_storage,
                ..
            } = active;
            let before = case.facts.budget().usage_snapshot().bytes;
            if poison {
                case.facts.budget().poison_ledger_for_tests();
            }
            drop(closedness);
            assert_eq!(case.facts.budget().usage_snapshot().bytes, before);
            drop(closedness_storage);
            assert_eq!(
                case.facts.budget().usage_snapshot().bytes,
                if poison { before } else { before - paid }
            );
            let before_parent = case.facts.budget().usage_snapshot().bytes;
            drop(parent);
            assert_eq!(
                case.facts.budget().usage_snapshot().bytes,
                if poison {
                    before_parent
                } else {
                    before_parent - 17
                }
            );
        }
    }

    #[test]
    fn active_equality_closedness_warm_preserves_context_source_and_rejection() {
        let ActiveEqualityCase {
            world,
            facts,
            projection,
            rhs,
            opaque,
        } = active_equality_fixture();
        let context = ProofContext::from_facts(&facts);
        let mut active = ProofScratch::default();
        let before = facts.budget().usage_snapshot().work;
        world
            .validate_equality_domain_in(&projection, &rhs, context, &mut active)
            .unwrap();
        let cold_work = facts.budget().usage_snapshot().work - before;
        let rows = active.closedness.len();
        assert!(rows > 0);
        let before = facts.budget().usage_snapshot().work;
        world
            .validate_equality_domain_in(&projection, &rhs, context, &mut active)
            .unwrap();
        let warm_work = facts.budget().usage_snapshot().work - before;
        assert!(warm_work > 0 && warm_work < cold_work);
        assert_eq!(active.closedness.len(), rows);
        // The second production caller keeps its ordinary proof/cycle policy.
        let predicate = Predicate::ProjectionEq {
            projection: projection.clone(),
            ty: rhs.clone(),
        };
        let mut proving = ProofScratch::default();
        for _ in 0..2 {
            world.require_in(&predicate, context, &mut proving).unwrap();
        }
        assert!(proving.checked.as_ref().unwrap().closedness.len() > 0);
        let ActiveEqualityCase { facts: foreign, .. } = active_equality_fixture();
        assert!(
            foreign
                .world()
                .validate_equality_domain_in(&projection, &rhs, context, &mut active,)
                .is_err()
        );
        let mut changed = facts.as_ref().clone();
        changed.predicates = world.empty.clone();
        assert!(
            world
                .validate_equality_domain_in(
                    &projection,
                    &rhs,
                    ProofContext::from_facts(&changed),
                    &mut active,
                )
                .is_err()
        );
        assert!(
            world
                .validate_equality_domain_in(
                    &projection,
                    &rhs,
                    ProofContext {
                        original: None,
                        predicates: &world.empty,
                        phase: FactsPhase::Closed
                    },
                    &mut active,
                )
                .is_err()
        );
        let mut forged = projection.clone();
        forged.item = facts.declaration();
        assert!(
            world
                .validate_equality_domain_in(&forged, &rhs, context, &mut active)
                .is_err()
        );
        assert_eq!(active.closedness.len(), rows);
        // Exact source occurrence is still checked before any warm decision.
        let Ty::Opaque(original) = opaque else {
            unreachable!()
        };
        let forged_opaque = Ty::Opaque(Arc::new(ir::OpaqueTy::new(
            original.owner(),
            original.owner_args().into(),
            original.site() + 1,
            original.bounds().into(),
            original.captured_regions().into(),
        )));
        let mut late = projection.clone();
        late.trait_ref.self_ty = Ty::Tuple(Arc::from([
            projection.trait_ref.self_ty.clone(),
            forged_opaque,
        ]));
        assert!(
            world
                .validate_equality_domain_in(&late, &rhs, context, &mut active)
                .is_err()
        );
        assert_eq!(active.closedness.len(), rows);
        let owner = world
            .headers
            .impls
            .iter()
            .find(|header| {
                header
                    .associated_types
                    .iter()
                    .any(|value| value.associated == projection.item)
            })
            .unwrap();
        let Ty::Struct(nominal) = &owner.trait_ref.self_ty else {
            panic!("source Number implementation")
        };
        let mut forged_definition = nominal.def().clone();
        forged_definition.module = foreign.world().catalog.modules().core_root();
        late.trait_ref.self_ty = Ty::Struct(ir::StructTy::new(forged_definition));
        assert!(
            world
                .validate_equality_domain_in(&late, &rhs, context, &mut active)
                .is_err()
        );
        assert_eq!(active.closedness.len(), rows);
        // A valid closed root is not permission to infer a rigid RHS, even warm.
        let mut closed = projection.clone();
        closed.trait_ref.self_ty = Ty::Bool;
        for _ in 0..2 {
            let error = world
                .validate_equality_domain_in(&closed, &rhs, context, &mut active)
                .unwrap_err();
            assert_eq!(error.kind(), FactsErrorKind::Unsupported);
        }
    }

    #[test]
    fn active_equality_closedness_complete_flags_usage_and_identity() {
        let ActiveEqualityCase {
            world,
            facts,
            projection,
            opaque,
            ..
        } = active_equality_fixture();
        let parameter = projection.trait_ref.self_ty.clone();
        let mut active = ProofScratch::default();
        for (ty, flags) in [
            (
                Ty::Tuple(Arc::from([Ty::Bool, parameter.clone()])),
                (true, false),
            ),
            (
                Ty::Tuple(Arc::from([Ty::Bool, opaque.clone()])),
                (false, true),
            ),
            (Ty::Tuple(Arc::from([parameter, opaque])), (true, true)),
            (Ty::Tuple(Arc::from([Ty::Bool, Ty::Unit])), (false, false)),
        ] {
            let mut walk = TypeWalk::default();
            let height = world
                .validate_ty_inner(&ty, facts.env(), 0, TypeUse::Root, &mut walk)
                .unwrap();
            assert_eq!((walk.has_parameters, walk.has_opaque), flags);
            let expected = (height, walk.has_parameters, walk.has_opaque);
            for _ in 0..2 {
                let result = world
                    .closedness_of_root_with_storage(
                        &ty,
                        facts.env(),
                        TypeUse::Root,
                        0,
                        &mut active.closedness,
                        &mut active.closedness_storage,
                    )
                    .unwrap();
                assert_eq!(
                    (result.height, result.has_parameters, result.has_opaque),
                    expected
                );
            }
            let rows = active.closedness.len();
            let actual = world
                .closedness_of_root_with_storage(
                    &ty,
                    facts.env(),
                    TypeUse::Actual,
                    0,
                    &mut active.closedness,
                    &mut active.closedness_storage,
                )
                .unwrap();
            assert_eq!(
                (actual.height, actual.has_parameters, actual.has_opaque),
                expected
            );
            assert_eq!(active.closedness.len(), rows + 1);
            // Same display/structure is not the retained allocation identity.
            let Ty::Tuple(parts) = &ty else {
                unreachable!()
            };
            let separate = Ty::Tuple(parts.iter().cloned().collect::<Vec<_>>().into());
            assert!(!same_closedness_input(&ty, &separate));
            let collision = world
                .closedness_of_root_with_storage(
                    &separate,
                    facts.env(),
                    TypeUse::Root,
                    0,
                    &mut active.closedness,
                    &mut active.closedness_storage,
                )
                .unwrap();
            assert_eq!(
                (
                    collision.height,
                    collision.has_parameters,
                    collision.has_opaque
                ),
                expected
            );
            assert_eq!(active.closedness.len(), rows + 2);
            let separate_env = GenericEnv::new(facts.env().scopes(), facts.budget()).unwrap();
            let result = world
                .closedness_of_root_with_storage(
                    &ty,
                    &separate_env,
                    TypeUse::Root,
                    0,
                    &mut active.closedness,
                    &mut active.closedness_storage,
                )
                .unwrap();
            assert_eq!(
                (result.height, result.has_parameters, result.has_opaque),
                expected
            );
            assert_eq!(active.closedness.len(), rows + 3);
        }
        // An invalid late child is never retained as a completed root.
        let ActiveEqualityCase { facts: foreign, .. } = active_equality_fixture();
        let foreign_parameter = Ty::Param(foreign.env().ordered_params().next().unwrap());
        let invalid = Ty::Tuple(Arc::from([Ty::Bool, foreign_parameter]));
        let rows = active.closedness.len();
        assert!(
            world
                .closedness_of_root_with_storage(
                    &invalid,
                    facts.env(),
                    TypeUse::Root,
                    0,
                    &mut active.closedness,
                    &mut active.closedness_storage,
                )
                .is_err()
        );
        assert_eq!(active.closedness.len(), rows);
        // A world change misses the completed-root key even for a universally
        // valid primitive. This raw helper grants no caller/context authority.
        world
            .closedness_of_root_with_storage(
                &Ty::Bool,
                facts.env(),
                TypeUse::Root,
                0,
                &mut active.closedness,
                &mut active.closedness_storage,
            )
            .unwrap();
        let rows = active.closedness.len();
        let foreign_result = foreign
            .world()
            .closedness_of_root_with_storage(
                &Ty::Bool,
                facts.env(),
                TypeUse::Root,
                0,
                &mut active.closedness,
                &mut active.closedness_storage,
            )
            .unwrap();
        assert!(!foreign_result.has_parameters && !foreign_result.has_opaque);
        assert_eq!(active.closedness.len(), rows + 1);
    }

    #[test]
    fn active_equality_closedness_warm_exact_work_short_and_issuers() {
        for short in [false, true] {
            let ActiveEqualityCase {
                world,
                facts,
                projection,
                rhs,
                ..
            } = active_equality_fixture();
            let mut active = ProofScratch::default();
            let context = ProofContext::from_facts(&facts);
            for _ in 0..2 {
                world
                    .validate_equality_domain_in(&projection, &rhs, context, &mut active)
                    .unwrap();
            }
            let rows = active.closedness.len();
            let before = facts.budget().usage_snapshot().work;
            world
                .validate_equality_domain_in(&projection, &rhs, context, &mut active)
                .unwrap();
            let cost = facts
                .budget()
                .usage_snapshot()
                .work
                .checked_sub(before)
                .unwrap();
            assert!(cost > 0);
            let filler = facts
                .budget()
                .limits()
                .work
                .checked_sub(facts.budget().usage_snapshot().work)
                .and_then(|remaining| remaining.checked_sub(cost))
                .and_then(|remaining| remaining.checked_add(usize::from(short)))
                .unwrap();
            facts.budget().charge_work(filler).unwrap();
            let result = world.validate_equality_domain_in(&projection, &rhs, context, &mut active);
            assert_eq!(result.is_err(), short);
            assert_eq!(active.closedness.len(), rows);
            if short {
                assert_eq!(
                    facts.budget().first_resource_failure().unwrap().kind,
                    ir::ResourceBudgetKind::Work
                );
                let failure = facts.budget().first_resource_failure();
                assert!(
                    world
                        .validate_equality_domain_in(&projection, &rhs, context, &mut active)
                        .is_err()
                );
                assert_eq!(facts.budget().first_resource_failure(), failure);
            }
        }
        for planner in [false, true] {
            let ActiveEqualityCase {
                world,
                facts,
                projection,
                rhs,
                ..
            } = active_equality_fixture();
            let mut active = ProofScratch::default();
            let context = ProofContext::from_facts(&facts);
            world
                .validate_equality_domain_in(&projection, &rhs, context, &mut active)
                .unwrap();
            let rows = active.closedness.len();
            if planner {
                facts.budget().poison_ledger_for_tests();
            } else {
                assert!(
                    world
                        .catalog
                        .modules()
                        .budget()
                        .charge(crate::compiler::expansion_budget::Counter::Work, u64::MAX,)
                        .is_err()
                );
            }
            let failure = facts.budget().first_resource_failure();
            for _ in 0..2 {
                assert!(
                    matches!(world.validate_equality_domain_in(&projection, &rhs, context, &mut active),
                    Err(error) if error.is_resource())
                );
                assert_eq!(active.closedness.len(), rows);
                assert_eq!(facts.budget().first_resource_failure(), failure);
            }
        }
    }

    #[test]
    fn active_equality_closedness_warm_depth_rechecks_cached_height() {
        let ActiveEqualityCase {
            world,
            facts,
            projection,
            ..
        } = active_equality_fixture();
        let ty = Ty::Tuple(Arc::from([projection.trait_ref.self_ty]));
        let mut active = ProofScratch::default();
        let first = world
            .closedness_of_root_with_storage(
                &ty,
                facts.env(),
                TypeUse::Root,
                0,
                &mut active.closedness,
                &mut active.closedness_storage,
            )
            .unwrap();
        assert!(first.height > 0);
        let rows = active.closedness.len();
        let depth = facts
            .budget()
            .limits()
            .depth
            .checked_sub(first.height)
            .unwrap();
        world
            .closedness_of_root_with_storage(
                &ty,
                facts.env(),
                TypeUse::Root,
                depth,
                &mut active.closedness,
                &mut active.closedness_storage,
            )
            .unwrap();
        assert!(matches!(world.closedness_of_root_with_storage(
            &ty, facts.env(), TypeUse::Root, depth + 1,
            &mut active.closedness, &mut active.closedness_storage,
        ), Err(error) if error.is_resource()));
        assert_eq!(active.closedness.len(), rows);
    }

    #[test]
    fn active_equality_closedness_actual_peak_short_and_drop() {
        let ActiveEqualityCase {
            world,
            facts,
            projection,
            ..
        } = active_equality_fixture();
        let ty = Ty::Tuple(Arc::from([projection.trait_ref.self_ty]));
        let before = facts.budget().usage_snapshot();
        assert_eq!(before.bytes, before.reservations.current_bytes);
        assert!(!before.reservations.counters_saturated);
        let lift_bytes = before
            .reservations
            .peak_bytes
            .checked_sub(before.bytes)
            .unwrap();
        let lift = facts.budget().reserve_bytes(lift_bytes).unwrap();
        let window = facts.budget().usage_snapshot();
        assert_eq!(window.bytes, window.reservations.peak_bytes);
        let mut measured = ProofScratch::default();
        world
            .closedness_of_root_with_storage(
                &ty,
                facts.env(),
                TypeUse::Root,
                0,
                &mut measured.closedness,
                &mut measured.closedness_storage,
            )
            .unwrap();
        let after = facts.budget().usage_snapshot();
        assert_eq!(after.bytes, after.reservations.current_bytes);
        assert!(!after.reservations.counters_saturated);
        let peak = after
            .reservations
            .peak_bytes
            .checked_sub(window.bytes)
            .unwrap();
        let net = after.bytes.checked_sub(window.bytes).unwrap();
        assert!(peak >= net && net > 0);
        let paid = scratch_row_bytes::<ClosednessValidation>().unwrap();
        assert_eq!(measured.closedness.len(), 1);
        drop(measured);
        assert_eq!(facts.budget().usage_snapshot().bytes, after.bytes - paid);
        drop(lift);
        for short in [false, true] {
            let current = facts.budget().usage_snapshot().bytes;
            let headroom = peak.checked_sub(usize::from(short)).unwrap();
            let filler = facts
                .budget()
                .reserve_bytes(
                    facts
                        .budget()
                        .limits()
                        .bytes
                        .checked_sub(current)
                        .unwrap()
                        .checked_sub(headroom)
                        .unwrap(),
                )
                .unwrap();
            let mut active = ProofScratch::default();
            let result = world.closedness_of_root_with_storage(
                &ty,
                facts.env(),
                TypeUse::Root,
                0,
                &mut active.closedness,
                &mut active.closedness_storage,
            );
            assert_eq!(result.is_err(), short);
            if short {
                assert!(active.closedness.is_empty());
                assert_eq!(
                    facts.budget().first_resource_failure().unwrap().kind,
                    ir::ResourceBudgetKind::Bytes
                );
            }
            let failure = facts.budget().first_resource_failure();
            let before_drop = facts.budget().usage_snapshot().bytes;
            drop(active);
            let after_owner_drop = facts.budget().usage_snapshot().bytes;
            if !short {
                assert_eq!(after_owner_drop, before_drop - paid);
            }
            drop(filler);
            assert_eq!(facts.budget().first_resource_failure(), failure);
            assert_eq!(
                facts.budget().usage_snapshot().bytes,
                after_owner_drop
                    - facts
                        .budget()
                        .limits()
                        .bytes
                        .checked_sub(current)
                        .unwrap()
                        .checked_sub(headroom)
                        .unwrap()
            );
        }
        // Explicit ledger poison is distinct from an ordinary denied reserve:
        // admitted owner and parent leases conservatively cannot refund it.
        let ActiveEqualityCase {
            world,
            facts,
            projection,
            ..
        } = active_equality_fixture();
        let parent = facts.budget().reserve_bytes(17).unwrap();
        let mut active = ProofScratch::default();
        world
            .closedness_of_root_with_storage(
                &projection.trait_ref.self_ty,
                facts.env(),
                TypeUse::Root,
                0,
                &mut active.closedness,
                &mut active.closedness_storage,
            )
            .unwrap();
        assert_eq!(active.closedness.len(), 1);
        let live = facts.budget().usage_snapshot().bytes;
        facts.budget().poison_ledger_for_tests();
        drop(active);
        assert_eq!(facts.budget().usage_snapshot().bytes, live);
        drop(parent);
        assert_eq!(facts.budget().usage_snapshot().bytes, live);
    }

    #[test]
    fn associated_assignment_source_rejects_missing_foreign_and_rebound_rows() {
        let source = frontend::parse(
            "trait Has{type Out;}struct Hold<'a,T>{r:&'a T}impl<'a,T> Has for Hold<'a,T>{type Out=&'a T;}fn probe(x:Hold<&i32>){}fn main(){}",
        ).unwrap();
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let owner = world
            .headers
            .impls
            .iter()
            .find(|owner| !owner.associated_types.is_empty())
            .unwrap();
        let value = &owner.associated_types[0];
        let original = world.checked_assignment_source(value).unwrap();
        assert_eq!(original.region_count, 1);
        assert_eq!(original.references.len(), 1);
        assert!(original.parameter_tail.is_some());
        for mode in 0..8 {
            let mut forged = value.clone();
            let row = Arc::make_mut(&mut forged.source);
            match mode {
                0 => row.references = Arc::from([]),
                1 => row.implementation = value.associated,
                2 => row.associated = value.implementation,
                3 => row.region_count = 0,
                4 => row.parameter_tail = None,
                5 => row.env = Arc::new(GenericEnv::new(&[], &budget).unwrap()),
                6 => {
                    row.references = Arc::from([(
                        ir::BorrowKind::Shared,
                        AssociatedSourceRegion::Named(ir::SourceRegionId {
                            declaration: value.associated,
                            index: 0,
                        }),
                    )])
                }
                _ => {
                    row.references = Arc::from([(
                        ir::BorrowKind::Shared,
                        AssociatedSourceRegion::Named(ir::SourceRegionId {
                            declaration: value.implementation,
                            index: 1,
                        }),
                    )])
                }
            }
            assert!(
                world.checked_assignment_source(&forged).is_err(),
                "forgery {mode}"
            );
        }
        let probe = world
            .schema
            .declaration_schema(plan.function_decl(0).unwrap())
            .unwrap();
        let projection = ir::AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: owner.trait_ref.declaration,
                self_ty: probe.signature.params[0].clone(),
                args: Arc::from([]),
            },
            item: value.associated,
        };
        let shared = |ty| Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let rhs = shared(shared(Ty::Int(ir::IntTy::I32)));
        assert!(
            world
                .shared_assignment_metadata(&projection, &rhs, &probe.env)
                .unwrap()
        );
        let deeper = shared(rhs.clone());
        assert!(
            !world
                .shared_assignment_metadata(&projection, &deeper, &probe.env)
                .unwrap()
        );
        let mutable = shared(Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(Ty::Int(ir::IntTy::I32)),
        });
        assert!(
            !world
                .shared_assignment_metadata(&projection, &mutable, &probe.env)
                .unwrap()
        );
        assert!(world.reject_projected_references(&rhs, 0).is_err());
    }

    #[test]
    fn shared_parameter_assignment_replays_aggregate_actual() {
        let source = frontend::parse(
            "trait Has{type Out;}struct Wrap<T>{value:T}impl<'a,T> Has for &'a Wrap<T>{type Out=&'a T;}fn probe(x:&Wrap<(&str,usize)>){}fn main(){}",
        ).unwrap();
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let owner = world
            .headers
            .impls
            .iter()
            .find(|owner| !owner.associated_types.is_empty())
            .unwrap();
        let probe = world
            .schema
            .declaration_schema(plan.function_decl(0).unwrap())
            .unwrap();
        let projection = ir::AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: owner.trait_ref.declaration,
                self_ty: probe.signature.params[0].clone(),
                args: Arc::from([]),
            },
            item: owner.associated_types[0].associated,
        };
        let leaf = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(Ty::Str),
        };
        let rhs = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(Ty::Tuple(Arc::from([
                leaf.clone(),
                Ty::Int(ir::IntTy::Usize),
            ]))),
        };
        assert!(
            world
                .shared_assignment_metadata(&projection, &rhs, &probe.env)
                .unwrap()
        );
        world
            .validate_equality_domain(&projection, &rhs, &probe.env)
            .unwrap();
        let forged = Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(Ty::Tuple(Arc::from([Ty::Int(ir::IntTy::Usize), leaf]))),
        };
        assert!(
            !world
                .shared_assignment_metadata(&projection, &forged, &probe.env)
                .unwrap()
        );
        assert!(world.reject_projected_references(&rhs, 0).is_err());
    }

    #[test]
    fn shared_tuple_assignment_replays_ordered_generic_payloads() {
        let source = frontend::parse(
            "trait Has{type Out;}struct Pair<K,V>{a:K,b:V}impl<'a,K,V> Has for &'a Pair<K,V>{type Out=(&'a K,&'a V);}fn probe(x:&Pair<u32,&str>){}fn main(){}",
        ).unwrap();
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let owner = world
            .headers
            .impls
            .iter()
            .find(|owner| !owner.associated_types.is_empty())
            .unwrap();
        let value = &owner.associated_types[0];
        let probe = world
            .schema
            .declaration_schema(plan.function_decl(0).unwrap())
            .unwrap();
        let projection = ir::AssociatedTypeProjection {
            trait_ref: TraitRef {
                declaration: owner.trait_ref.declaration,
                self_ty: probe.signature.params[0].clone(),
                args: Arc::from([]),
            },
            item: value.associated,
        };
        let shared = |ty| Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ty),
        };
        let first = shared(Ty::Int(ir::IntTy::U32));
        let second = shared(shared(Ty::Str));
        let rhs = Ty::Tuple(Arc::from([first.clone(), second.clone()]));
        assert!(
            world
                .shared_assignment_metadata(&projection, &rhs, &probe.env)
                .unwrap()
        );
        world
            .validate_equality_domain(&projection, &rhs, &probe.env)
            .unwrap();
        for forged in [
            Ty::Tuple(Arc::from([second.clone(), first.clone()])),
            Ty::Tuple(Arc::from([first.clone()])),
            Ty::Tuple(Arc::from([first.clone(), shared(Ty::Int(ir::IntTy::U32))])),
            Ty::Tuple(Arc::from([
                first,
                shared(Ty::Ref {
                    kind: ir::BorrowKind::Mut,
                    referent: Arc::new(Ty::Str),
                }),
            ])),
        ] {
            assert!(
                !world
                    .shared_assignment_metadata(&projection, &forged, &probe.env)
                    .unwrap()
            );
        }
        for mode in 0..3 {
            let mut forged = value.clone();
            let row = Arc::make_mut(&mut forged.source);
            match mode {
                0 => row.associated = value.implementation,
                1 => row.implementation = value.associated,
                _ => row.region_count = 0,
            }
            assert!(world.checked_assignment_source(&forged).is_err());
        }
        assert!(world.reject_projected_references(&rhs, 0).is_err());
    }

    #[test]
    fn associated_assignment_rejects_undeclared_source_lifetime() {
        let source = frontend::parse("trait Has{type Out;}struct Hold<'a,T>{r:&'a T}impl<'a,T> Has for Hold<'a,T>{type Out=&'missing T;}fn main(){}").unwrap();
        let budget = PlanBudget::default();
        let error = match PlanCtx::from_program(&source, &budget) {
            Ok(_) => panic!("undeclared lifetime accepted"),
            Err(error) => error,
        };
        assert!(
            error.contains("undeclared associated assignment lifetime"),
            "{error}"
        );
    }

    #[test]
    fn inherited_equality_and_exact_owner_projection_coexist_in_signature_and_body() {
        let source = frontend::parse(
            "trait Other:Sized{type X;fn x(self)->Self::X;}\
             trait Source<A>{type Out;fn get(self,value:A)->Self::Out;}\
             struct Wrap<T>{value:T}\
             impl<T:Other<X=i32>> Source<i32> for Wrap<T>{\
               type Out=i32;\
               fn get(self,value:<T as Other>::X)->Self::Out{\
                 let owned:Self::Out=value;\
                 let other:<T as Other>::X=<T as Other>::x(self.value);\
                 owned+other\
               }\
             }\
             fn main(){}",
        )
        .unwrap();
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let owner = world
            .headers
            .impls
            .iter()
            .find(|header| !header.items.is_empty())
            .unwrap();
        let declaration = owner.items[0].1;
        let facts = world.callable_facts(declaration).unwrap();
        let raw = world.schema.declaration_schema(declaration).unwrap();
        assert!(matches!(raw.signature.result, Ty::AssociatedType(_)));
        assert!(matches!(raw.signature.params[1], Ty::AssociatedType(_)));
        let Ty::AssociatedType(projection) = &raw.signature.params[1] else {
            unreachable!()
        };
        let context = ProofContext::from_facts(&facts);
        let mut scratch = ProofScratch::default();
        let rhs = owner.trait_ref.self_ty.clone();
        let before = budget.usage_snapshot().work;
        world
            .validate_equality_domain_in(projection, &rhs, context, &mut scratch)
            .unwrap();
        let first_work = budget.usage_snapshot().work - before;
        let retained = scratch.replays.types.len();
        assert!(retained > 0);
        let before = budget.usage_snapshot().work;
        world
            .validate_equality_domain_in(projection, &rhs, context, &mut scratch)
            .unwrap();
        assert!(budget.usage_snapshot().work - before < first_work);
        assert_eq!(scratch.replays.types.len(), retained);
        assert!(
            world
                .validate_equality_domain_in(
                    projection,
                    &rhs,
                    ProofContext {
                        original: None,
                        predicates: &world.empty,
                        phase: FactsPhase::Closed
                    },
                    &mut scratch
                )
                .is_err()
        );
        let foreign_program = frontend::parse("fn main(){}").unwrap();
        let foreign_plan = PlanCtx::from_program(&foreign_program, &PlanBudget::default()).unwrap();
        let foreign_world = foreign_plan.trait_world().unwrap();
        assert!(
            foreign_world
                .validate_equality_domain_in(projection, &rhs, context, &mut scratch)
                .is_err()
        );
        let Ty::Struct(nominal) = &rhs else {
            panic!("fixture nominal")
        };
        let mut forged = nominal.def().clone();
        forged.module = foreign_world.catalog.modules().core_root();
        let forged = Ty::Struct(ir::StructTy::new(forged));
        assert!(
            world
                .validate_equality_domain_in(projection, &forged, context, &mut scratch)
                .is_err()
        );
        assert_eq!(scratch.replays.types.len(), retained);
        let normalized = world.normalized_callable_schema(declaration).unwrap();
        assert_eq!(normalized.signature.result, Ty::Int(ir::IntTy::I32));
        assert_eq!(normalized.signature.params[1], Ty::Int(ir::IntTy::I32));
        assert_eq!(
            world.normalize_ty(&raw.signature.result, &facts).unwrap(),
            Ty::Int(ir::IntTy::I32)
        );
        assert_eq!(
            world
                .normalize_ty(&raw.signature.params[1], &facts)
                .unwrap(),
            Ty::Int(ir::IntTy::I32)
        );
        let target = CallTarget {
            declaration,
            type_args: facts
                .env()
                .ordered_params()
                .map(Ty::Param)
                .collect::<Vec<_>>()
                .into(),
        };
        plan.checked_body(&target, &facts, &budget).unwrap();
        let context = world.declared_callable_context(declaration).unwrap();
        assert!(!context.reductions.values.is_empty());
        assert!(
            world
                .requirements
                .iter()
                .find(|(id, _)| *id == declaration)
                .unwrap()
                .1
                .iter()
                .any(|row| matches!(row, Predicate::ProjectionEq { .. }))
        );
    }
}
