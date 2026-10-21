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
        let actual = world
            .declared_equality_rhs(projection, &facts.predicates.predicates, facts.env())?
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
        for ty in predicate.metadata_types() {
            self.validate_ty_metadata_in(ty, env, 0)?;
        }
        if let Predicate::ProjectionEq { projection, .. } = predicate {
            self.validate_ty_metadata_in(&self.projection_type(projection)?, env, 0)?;
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
            *ty = self.map_equality_type(ty, 0, &mut reduce)?;
        }
        instantiated.signature.result =
            self.map_equality_type(&instantiated.signature.result, 0, &mut reduce)?;
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
    pub(super) fn same_projection(
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
        self.check_normalization_budget()?;
        self.validate_ty_metadata_in(&self.projection_type(projection)?, env, 0)?;
        self.validate_ty_metadata_in(rhs, env, 0)?;
        // Raw admission is solver-free. Metadata, including rejected children,
        // has already been validated above.
        if matches!(rhs, Ty::Param(_)) {
            if self.goal_is_closed(&projection.trait_ref, env)? {
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
        self.validate_equality_reference_domain(rhs, 0)?;
        if matches!(rhs, Ty::AssociatedType(_)) {
            // A closed projection RHS (the `Item = <IntoIter as Iterator>::Item`
            // bound of a concrete `IntoIterator` impl) is metadata here, like a
            // symbolic RHS: it reduces through the closed impl set when the
            // equality is consumed, and executable validation still rejects
            // any closed projection that never normalizes.
            return Ok(());
        }
        self.validate_ty(rhs, env, None, true, 0)
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
            } => self.reject_projected_references(referent, depth + 1),
            Ty::Ref { .. } | Ty::Slice { .. } => Err(FactsError::unsupported(
                &self.budget,
                "unsupported: mutable or nested reference associated equality",
            )),
            Ty::Array { element, .. } => {
                self.validate_equality_reference_domain(element, depth + 1)
            }
            Ty::Struct(value) => {
                for field in value.def().fields.iter() {
                    self.validate_equality_reference_domain(&field.ty, depth + 1)?;
                }
                for argument in type_arguments(ty) {
                    self.validate_equality_reference_domain(argument, depth + 1)?;
                }
                Ok(())
            }
            Ty::Enum(value) => {
                for variant in value.def().variants.iter() {
                    for field in variant.fields.iter() {
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

    pub(super) fn validate_equality_domain_in(
        &self,
        projection: &ir::AssociatedTypeProjection,
        rhs: &Ty,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        self.validate_equality_domain(projection, rhs, context.env())?;
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
            || self.goal_is_closed(&projection.trait_ref, context.env())?
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
    /// Equality-only selection for a rigid source nominal subject such as
    /// `slice::Iter<T>`. The impl is chosen by canonical head matching in the
    /// genuine caller context and its RHS is reminted from the immutable
    /// declaration callback. This never grants a carrier, layout or origin.
    pub(super) fn rigid_nominal_implementation_rhs(
        &self,
        projection: &ir::AssociatedTypeProjection,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Option<(ProofEvidence, Ty)>> {
        if !matches!(projection.trait_ref.self_ty, Ty::Struct(_) | Ty::Enum(_))
            || self.goal_is_closed(&projection.trait_ref, context.env())?
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
        let normal = self.map_equality_type(&instantiated.signature.result, 0, &mut |nested| {
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
        })?;
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
        self.map_equality_type(ty, depth, &mut |projection| {
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
                        self.map_equality_type(part, depth + 1, reduce)?,
                    )?;
                }
                Ty::Tuple(values.into())
            }
            Ty::Array { element, len } => {
                retain::<Ty>(&self.budget, 1)?;
                Ty::Array {
                    element: Arc::new(self.map_equality_type(element, depth + 1, reduce)?),
                    len: *len,
                }
            }
            Ty::Ref { kind, referent } => {
                retain::<Ty>(&self.budget, 1)?;
                Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(self.map_equality_type(referent, depth + 1, reduce)?),
                }
            }
            Ty::Slice { element } => {
                retain::<Ty>(&self.budget, 1)?;
                Ty::Slice {
                    element: Arc::new(self.map_equality_type(element, depth + 1, reduce)?),
                }
            }
            Ty::Struct(_) | Ty::Enum(_) if self.type_contains_projection(ty, depth)? => {
                return Err(FactsError::unsupported(
                    &self.budget,
                    "unsupported: projection in nominal metadata",
                ));
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
            let self_ty = self.map_equality_type(&application.self_ty, 0, &mut reduce)?;
            let mut args = Vec::new();
            for argument in application.args.iter() {
                push(
                    &self.budget,
                    &mut args,
                    self.map_equality_type(argument, 0, &mut reduce)?,
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
                    ty: self.map_equality_type(ty, 0, &mut reduce)?,
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
