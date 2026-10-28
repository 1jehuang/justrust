//! Required method authority for a rigid universal receiver.
//!
//! A caller bound proves membership, not an implementation assignment. Keep
//! bound-only associated types attached to their exact declaration and ordered
//! application. An exact implementation witness may reduce its source RHS in
//! the caller context, with immutable evidence replayed by executable consumers.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct UniversalMethodEvidence {
    pub(super) call: TraitCall,
    pub(super) arguments: Arc<[Ty]>,
    pub(super) selection: ProofEvidence,
    pub(super) signature: Signature,
    pub(super) source_regions: Arc<ir::SourceRegionEnv>,
    pub(super) dependencies: Arc<[(ir::AssociatedTypeProjection, equality::ProjectionReduction)]>,
    implementation_dependencies: Arc<[(ir::AssociatedTypeProjection, ProofEvidence, Ty)]>,
}

impl ValidatedFacts {
    /// Admission-only reuse for one immutable original caller. The RefCell
    /// borrow ends before any recursive normalization/equality/dependency work.
    pub(in crate::compiler) fn normalize_substituted_projection_type_scratch(
        &self,
        ty: &Ty,
        budget: &PlanBudget,
        original: &ValidatedFacts,
        scratch: &std::cell::RefCell<TraitScratch>,
    ) -> Result<Ty> {
        self.world.check_facts(self, false)?;
        self.world.check_nominal_replay_issuer()?;
        if !std::ptr::eq(self, original) || !self.budget().same_authority(budget) {
            return Err(FactsError::invalid(
                budget,
                "solver metadata changed original caller or planning authority",
            ));
        }
        if !self.world.type_contains_projection(ty, 0)? {
            return Ok(ty.clone());
        }
        {
            let mut admission = scratch.try_borrow_mut().map_err(|_| {
                FactsError::invalid(budget, "recursive solver metadata admission borrow")
            })?;
            self.validate_ty_metadata_scratch(ty, budget, &mut admission)?;
        }
        self.normalize_substituted_projection_type_after_metadata(ty, budget)
    }

    /// Structural substitution may close an opaque associated family. Resolve
    /// it before executable consumers such as cleanup planning inspect it.
    /// Schema substitution deliberately does not call this entry point.
    pub(in crate::compiler) fn normalize_substituted_projection_type(
        &self,
        ty: &Ty,
        budget: &PlanBudget,
    ) -> Result<Ty> {
        self.world.check_facts(self, false)?;
        if !self.world.type_contains_projection(ty, 0)? {
            return Ok(ty.clone());
        }
        self.validate_ty_metadata_with_budget(ty, budget)?;
        self.normalize_substituted_projection_type_after_metadata(ty, budget)
    }

    fn normalize_substituted_projection_type_after_metadata(
        &self,
        ty: &Ty,
        budget: &PlanBudget,
    ) -> Result<Ty> {
        let context = ProofContext::from_facts(self);
        self.world
            .map_universal_projection_type(ty, self.env(), 0, &mut |projection| {
                let raw = Ty::AssociatedType(Arc::new(projection.clone()));
                if self
                    .world
                    .checked_closure_projection(projection, self.env())?
                {
                    return self.world.normalize_equality_type(&raw, self);
                }
                if !self.world.raw_associated_ty_is_closed(&raw, 0)? {
                    if associated_types::nominal_application_self(&projection.trait_ref.self_ty) {
                        let (normal, dependencies) = self
                            .world
                            .normalize_closed_projected_type(&raw, context, false)?;
                        for dependency in dependencies.iter() {
                            self.world
                                .validate_projected_dependency(dependency, context)?;
                        }
                        return Ok(normal);
                    }
                    self.world
                        .validate_universal_projection(projection, self.env())?;
                    for row in self.predicates.predicates.iter() {
                        charged(budget, budget.charge_work(1))?;
                        if let Predicate::ProjectionEq {
                            projection: declared,
                            ..
                        } = row
                            && declared.item == projection.item
                            && self.world.eq_predicate(
                                &Predicate::Implements(declared.trait_ref.clone()),
                                &Predicate::Implements(projection.trait_ref.clone()),
                            )?
                        {
                            return self.world.normalize_equality_type(&raw, self);
                        }
                    }
                    return Ok(raw);
                }
                let (normal, dependencies) = self
                    .world
                    .normalize_closed_projected_type(&raw, context, false)?;
                for dependency in dependencies.iter() {
                    self.world
                        .validate_projected_dependency(dependency, context)?;
                }
                Ok(normal)
            })
    }
}

impl ValidatedTraitWorld {
    pub(super) fn map_universal_projection_type(
        &self,
        ty: &Ty,
        env: &GenericEnv,
        depth: usize,
        reduce: &mut impl FnMut(&ir::AssociatedTypeProjection) -> Result<Ty>,
    ) -> Result<Ty> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        Ok(match ty {
            Ty::AssociatedType(projection) => {
                let receiver = self.map_universal_projection_type(
                    &projection.trait_ref.self_ty,
                    env,
                    depth + 1,
                    reduce,
                )?;
                let mut arguments = Vec::new();
                for argument in projection.trait_ref.args.iter() {
                    push(
                        &self.budget,
                        &mut arguments,
                        self.map_universal_projection_type(argument, env, depth + 1, reduce)?,
                    )?;
                }
                reduce(&ir::AssociatedTypeProjection {
                    trait_ref: TraitRef {
                        declaration: projection.trait_ref.declaration,
                        self_ty: receiver,
                        args: arguments.into(),
                    },
                    item: projection.item,
                })?
            }
            Ty::Closure(closure) => Ty::Closure(Arc::new(closure.try_map(|child| {
                self.map_universal_projection_type(child, env, depth + 1, reduce)
            })?)),
            Ty::Dyn(object) => {
                let mut args = Vec::new();
                for argument in object.args.iter() {
                    push(
                        &self.budget,
                        &mut args,
                        self.map_universal_projection_type(argument, env, depth + 1, reduce)?,
                    )?;
                }
                let mut associated_types = Vec::new();
                for (item, value) in object.associated_types.iter() {
                    push(
                        &self.budget,
                        &mut associated_types,
                        (
                            *item,
                            self.map_universal_projection_type(value, env, depth + 1, reduce)?,
                        ),
                    )?;
                }
                Ty::Dyn(Arc::new(ir::DynTy {
                    principal: object.principal,
                    args: args.into(),
                    associated_types: associated_types.into(),
                }))
            }
            Ty::Ref { kind, referent } => Ty::Ref {
                kind: *kind,
                referent: Arc::new(self.map_universal_projection_type(
                    referent,
                    env,
                    depth + 1,
                    reduce,
                )?),
            },
            Ty::RawPtr { kind, pointee } => Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(self.map_universal_projection_type(
                    pointee,
                    env,
                    depth + 1,
                    reduce,
                )?),
            },
            Ty::Array { element, len } => Ty::Array {
                element: Arc::new(self.map_universal_projection_type(
                    element,
                    env,
                    depth + 1,
                    reduce,
                )?),
                len: *len,
            },
            Ty::Slice { element } => Ty::Slice {
                element: Arc::new(self.map_universal_projection_type(
                    element,
                    env,
                    depth + 1,
                    reduce,
                )?),
            },
            Ty::Tuple(parts) => {
                let mut mapped = Vec::new();
                for part in parts.iter() {
                    push(
                        &self.budget,
                        &mut mapped,
                        self.map_universal_projection_type(part, env, depth + 1, reduce)?,
                    )?;
                }
                Ty::Tuple(mapped.into())
            }
            Ty::Struct(_) | Ty::Enum(_) => {
                self.normalize_nominal_actuals(ty, env, depth, |actual| {
                    self.map_universal_projection_type(actual, env, depth + 1, reduce)
                })?
            }
            _ => ty.clone(),
        })
    }

    pub(super) fn normalize_instantiated_projection_signature(
        &self,
        signature: &mut Signature,
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<()> {
        self.check_context(context)?;
        if !self.signature_contains_projection(signature)? {
            return Ok(());
        }
        let mut normalize = |projection: &ir::AssociatedTypeProjection| -> Result<Ty> {
            let raw = Ty::AssociatedType(Arc::new(projection.clone()));
            if let Some(ty) = self.declared_equality_rhs(
                projection,
                &context.predicates.predicates,
                context.env(),
            )? {
                self.require_in(
                    &Predicate::ProjectionEq {
                        projection: projection.clone(),
                        ty: ty.clone(),
                    },
                    context,
                    active,
                )?;
                return Ok(ty);
            }
            if !self.raw_associated_ty_is_closed(&raw, 0)?
                && !self.checked_closure_projection(projection, context.env())?
                && !associated_types::nominal_application_self(&projection.trait_ref.self_ty)
            {
                return Ok(raw);
            }
            // normalize_closed_projected_type validates every returned
            // dependency under this exact context before returning.
            let (normal, _) =
                self.normalize_closed_projected_type_with(&raw, context, false, active)?;
            Ok(normal)
        };
        for parameter in &mut signature.params {
            *parameter =
                self.map_universal_projection_type(parameter, context.env(), 0, &mut normalize)?;
        }
        signature.result = self.map_universal_projection_type(
            &signature.result,
            context.env(),
            0,
            &mut normalize,
        )?;
        Ok(())
    }

    pub(super) fn normalize_instantiated_projection_predicates(
        &self,
        predicates: &[Predicate],
        context: ProofContext<'_>,
        active: &mut ProofScratch,
    ) -> Result<Arc<[Predicate]>> {
        self.check_context(context)?;
        let mut normalize = |projection: &ir::AssociatedTypeProjection| -> Result<Ty> {
            let raw = Ty::AssociatedType(Arc::new(projection.clone()));
            if let Some(ty) = self.declared_equality_rhs(
                projection,
                &context.predicates.predicates,
                context.env(),
            )? {
                self.require_in(
                    &Predicate::ProjectionEq {
                        projection: projection.clone(),
                        ty: ty.clone(),
                    },
                    context,
                    active,
                )?;
                return Ok(ty);
            }
            if !self.raw_associated_ty_is_closed(&raw, 0)?
                && !self.checked_closure_projection(projection, context.env())?
                && !associated_types::nominal_application_self(&projection.trait_ref.self_ty)
            {
                return Ok(raw);
            }
            // normalize_closed_projected_type validates every returned
            // dependency under this exact context before returning.
            let (normal, _) =
                self.normalize_closed_projected_type_with(&raw, context, false, active)?;
            Ok(normal)
        };
        let mut rows = Vec::new();
        for predicate in predicates {
            let goal = predicate.trait_ref();
            let mut args = Vec::new();
            for argument in goal.args.iter() {
                push(
                    &self.budget,
                    &mut args,
                    self.map_universal_projection_type(argument, context.env(), 0, &mut normalize)?,
                )?;
            }
            let goal = TraitRef {
                declaration: goal.declaration,
                self_ty: self.map_universal_projection_type(
                    &goal.self_ty,
                    context.env(),
                    0,
                    &mut normalize,
                )?,
                args: args.into(),
            };
            let row = match predicate {
                Predicate::Implements(_) => Predicate::Implements(goal),
                Predicate::ProjectionEq { projection, ty } => Predicate::ProjectionEq {
                    projection: ir::AssociatedTypeProjection {
                        trait_ref: goal,
                        item: projection.item,
                    },
                    ty: self.map_universal_projection_type(ty, context.env(), 0, &mut normalize)?,
                },
            };
            push(&self.budget, &mut rows, row)?;
        }
        Ok(rows.into())
    }

    pub(super) fn retain_universal_owner_projection(
        &self,
        projection: &ir::AssociatedTypeProjection,
        context: ProofContext<'_>,
    ) -> Result<Option<Ty>> {
        self.check_context(context)?;
        if context.phase != FactsPhase::Universal
            || self.goal_is_closed(&projection.trait_ref, context.env())?
        {
            return Ok(None);
        }
        // This is a symbolic RHS authorized by the owning declaration's
        // predicates, not an associated assignment owned by another impl.
        self.require_in(
            &Predicate::Implements(projection.trait_ref.clone()),
            context,
            &mut ProofScratch::default(),
        )?;
        retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
        Ok(Some(Ty::AssociatedType(Arc::new(projection.clone()))))
    }

    pub(super) fn instantiate_universal_required_item(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<(InstantiatedSchema, Arc<UniversalMethodEvidence>)> {
        self.check_facts(facts, true)?;
        facts.validate_callee_metadata(&Callee::TraitItem(call.clone()))?;
        if facts.phase() != FactsPhase::Universal || self.trait_call_is_closed(call, facts.env())? {
            return Err(FactsError::invalid(
                &self.budget,
                "universal required method needs a rigid universal application",
            ));
        }
        let item = self.item_header(call.item)?;
        self.validate_projected_required_schema(item)?;
        if item.trait_decl != call.trait_ref.declaration
            || self.info(item.declaration)?.own_env.arity() != call.method_args.len()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "universal required method changed owner or method arity",
            ));
        }
        let context = ProofContext::from_facts(facts);
        let selection = self.require_in(
            &Predicate::Implements(call.trait_ref.clone()),
            context,
            &mut ProofScratch::default(),
        )?;
        // The evidence is tied to this caller, never the empty closed context.
        if selection.0.world != self.id
            || selection.0.environment != facts.predicates.index
            || selection.0.phase != FactsPhase::Universal
            || !self.eq_predicate(
                &selection.0.goal,
                &Predicate::Implements(call.trait_ref.clone()),
            )?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "universal required method lost exact caller membership proof",
            ));
        }
        let mut arguments = self.trait_arguments(&call.trait_ref)?;
        for actual in call.method_args.iter() {
            self.validate_ty_metadata_in(actual, facts.env(), 0)?;
            push(&self.budget, &mut arguments, actual.clone())?;
        }
        let mut instantiated =
            self.substitute_projected_schema(&item.schema, &arguments, facts.env())?;
        // A reference Self actual (`IntoIterator for &'a Vec<K>` in a generic
        // caller) expands the raw Self tail into exact input regions, derived
        // from the original schema as in `instantiate_projected_item`.
        instantiated.signature.ref_contract = self.realize_ref_contract(
            &item.schema,
            &arguments,
            &instantiated.signature,
            Some(item.declaration),
        )?;
        let mut dependencies = Vec::new();
        let mut implementation_dependencies = Vec::new();
        let mut reduce = |projection: &ir::AssociatedTypeProjection| -> Result<Ty> {
            if self.checked_closure_projection(projection, facts.env())?
                || self
                    .opaque_associated_binding(projection, facts.env())?
                    .is_some()
                || self.goal_is_closed(&projection.trait_ref, facts.env())?
                || associated_types::nominal_application_self(&projection.trait_ref.self_ty)
                || self
                    .declared_equality_rhs(projection, &facts.predicates.predicates, facts.env())?
                    .is_some()
            {
                let proof = self.normalize_projection_for_facts(projection, facts)?;
                self.validate_equality_domain_in(
                    projection,
                    proof.ty(),
                    context,
                    &mut ProofScratch::default(),
                )?;
                let ty = proof.ty().clone();
                push(&self.budget, &mut dependencies, (projection.clone(), proof))?;
                return Ok(ty);
            }
            let membership = self.require_in(
                &Predicate::Implements(projection.trait_ref.clone()),
                context,
                &mut ProofScratch::default(),
            )?;
            if matches!(membership.0.origin, ProofOrigin::Implementation { .. }) {
                let ty = self.implementation_equality_rhs(projection, &membership, context)?;
                self.validate_equality_domain_in(
                    projection,
                    &ty,
                    context,
                    &mut ProofScratch::default(),
                )?;
                retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
                retain::<Ty>(&self.budget, 1)?;
                push(
                    &self.budget,
                    &mut implementation_dependencies,
                    (projection.clone(), membership, ty.clone()),
                )?;
                return Ok(ty);
            }
            retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
            Ok(Ty::AssociatedType(Arc::new(projection.clone())))
        };
        for parameter in &mut instantiated.signature.params {
            *parameter =
                self.map_universal_projection_type(parameter, facts.env(), 0, &mut reduce)?;
        }
        let projected_result = charged(
            &self.budget,
            ir::contains_associated_types(&instantiated.signature.result, &self.budget),
        )?;
        instantiated.signature.result = self.map_universal_projection_type(
            &instantiated.signature.result,
            facts.env(),
            0,
            &mut reduce,
        )?;
        // A declaration result written as a projection has no reference
        // positions in its schema contract. A reference-valued equality RHS
        // would need region authority that the schema never declared.
        if projected_result
            && charged(
                &self.budget,
                ir::root_reference_layers(&instantiated.signature.result),
            )? > instantiated.signature.ref_contract.results.len()
        {
            return Err(FactsError::unsupported(
                &self.budget,
                "unsupported: equality RHS must be closed or one authoritative parameter",
            ));
        }
        instantiated.predicates = self.normalize_instantiated_projection_predicates(
            &instantiated.predicates,
            context,
            &mut ProofScratch::default(),
        )?;
        for predicate in instantiated.predicates.iter() {
            self.require_in(predicate, context, &mut ProofScratch::default())?;
        }
        self.validate_schema_signature(call.item, &instantiated.signature, facts.env(), None)?;
        let source_regions = self.validate_source_regions(call.item, &instantiated.signature)?;
        retain::<UniversalMethodEvidence>(&self.budget, 1)?;
        retain::<Ty>(&self.budget, instantiated.signature.params.len())?;
        let evidence = Arc::new(UniversalMethodEvidence {
            call: call.clone(),
            arguments: arguments.into(),
            selection,
            signature: instantiated.signature.clone(),
            source_regions,
            dependencies: dependencies.into(),
            implementation_dependencies: implementation_dependencies.into(),
        });
        Ok((instantiated, evidence))
    }

    pub(super) fn universal_required_carrier_contract(
        &self,
        call: &TraitCall,
        facts: &ValidatedFacts,
    ) -> Result<Arc<CarrierContractView>> {
        let (application, authority) = self.instantiate_universal_required_item(call, facts)?;
        let mut view = self.universal_required_carrier_parts(application, authority, facts)?;
        self.seal_equality_results(&mut view, facts)?;
        view.validate_for(facts)?;
        Ok(Arc::new(view))
    }

    /// Carriers of a universal required application. An associated type is
    /// fixed by the trait reference alone, so a required item whose declared
    /// result names no reference of its own cannot return through the
    /// method's own parameter borrows (`fn next(&mut self) -> Option<Self::Item>`).
    /// Only carriers reached through a parameter's referent stay eligible.
    pub(super) fn derive_universal_required_carriers(
        &self,
        item: DeclId,
        signature: &Signature,
        source: &ir::SourceRegionEnv,
    ) -> Result<(ir::CarrierContract, ir::CarrierRegionSchema)> {
        let (mut contract, regions) = self.derive_carriers(signature, source)?;
        let raw = &self.item_header(item)?.schema.signature;
        let declared_reference = charged(
            &self.budget,
            ir::carrier_locations(&raw.result, &self.budget),
        )?
        .iter()
        .any(|location| matches!(location.channel, ir::CarrierChannel::Fixed(_)));
        if declared_reference {
            return Ok((contract, regions));
        }
        let mut results = Vec::new();
        for result in contract.results.iter() {
            let mut sources = Vec::new();
            for input in result.sources.iter() {
                charged(&self.budget, self.budget.charge_work(1))?;
                let own_borrow = input.location.path.0.is_empty()
                    && matches!(input.location.channel, ir::CarrierChannel::Fixed(_))
                    && matches!(raw.params.get(input.param), Some(Ty::Ref { .. }));
                if !own_borrow {
                    push(&self.budget, &mut sources, input.clone())?;
                }
            }
            push(
                &self.budget,
                &mut results,
                ir::CarrierReturn {
                    location: result.location.clone(),
                    sources: sources.into(),
                },
            )?;
        }
        contract.results = results.into();
        Ok((contract, regions))
    }

    fn universal_required_carrier_parts(
        &self,
        application: InstantiatedSchema,
        authority: Arc<UniversalMethodEvidence>,
        facts: &ValidatedFacts,
    ) -> Result<CarrierContractView> {
        let signature = application.signature;
        let source_regions = authority.source_regions.clone();
        let origin = charged(
            &self.budget,
            ir::derive_signature_origin_schema(&signature, facts.env(), &self.budget),
        )?;
        let (contract, regions) = self.derive_universal_required_carriers(
            authority.call.item,
            &signature,
            &source_regions,
        )?;
        let mut arguments = Vec::new();
        let mut components = Vec::new();
        for parameter in facts.env().ordered_params() {
            let ty = Ty::Param(parameter);
            for location in charged(&self.budget, ir::carrier_locations(&ty, &self.budget))?.iter()
            {
                push(
                    &self.budget,
                    &mut components,
                    ir::CarrierComponentMapping {
                        original_family: ir::CarrierFamily::Parameter(parameter),
                        original: location.clone(),
                        substituted: location.clone(),
                    },
                )?;
            }
            push(&self.budget, &mut arguments, ty)?;
        }
        retain::<projected_methods::ProjectedMethodEvidence>(&self.budget, 1)?;
        let projected = Arc::new(projected_methods::ProjectedMethodEvidence::universal(
            self.id,
            facts,
            authority.clone(),
        ));
        retain::<CarrierContractView>(&self.budget, 1)?;
        charged(&self.budget, self.budget.charge_nodes(1))?;
        retain::<NativeHeapInvariant>(&self.budget, 0)?;
        Ok(CarrierContractView {
            native_heap_invariants: Arc::from([]),
            nominal: None,
            physical: None,
            closure_result: None,
            world: self.id,
            registry: self.registry,
            environment: facts.predicates.index,
            phase: facts.phase(),
            declaration: authority.call.item,
            arguments: arguments.into(),
            original_env: facts.env().clone(),
            original_contract: contract.clone(),
            contract,
            origin,
            fixed: signature.ref_contract.clone(),
            regions,
            source_regions,
            components: components.into(),
            nominals: Arc::from([]),
            projected: Some(projected),
            projected_subject: Some(authority.call.trait_ref.clone()),
            equality_required: None,
            equality_results: None,
            result_subject: parameter_equality::EqualityResultSubject::Required(
                authority.call.clone(),
            ),
            result_signature: signature,
            budget: self.budget.clone(),
        })
    }

    pub(super) fn validate_universal_required_carrier(
        &self,
        view: &CarrierContractView,
        authority: &UniversalMethodEvidence,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        let (application, replay) = self.validate_universal_required_evidence(authority, facts)?;
        let parameter_equality::EqualityResultSubject::Required(subject) = &view.result_subject
        else {
            return Err(FactsError::invalid(
                &self.budget,
                "universal required carrier lost call subject",
            ));
        };
        if subject.item != authority.call.item
            || !self.eq_predicate(
                &Predicate::Implements(subject.trait_ref.clone()),
                &Predicate::Implements(authority.call.trait_ref.clone()),
            )?
            || !self.eq_types(&subject.method_args, &authority.call.method_args, 0)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "universal required carrier changed method actuals",
            ));
        }
        let expected = self.universal_required_carrier_parts(application, replay, facts)?;
        if view.declaration != authority.call.item
            || !self.same_env(&view.original_env, facts.env())?
            || !self.eq_types(&view.arguments, &expected.arguments, 0)?
            || !self.eq_signature(&view.result_signature, &authority.signature)?
            || !self.eq_contract(&view.fixed, &expected.fixed)?
            || view.original_contract != expected.original_contract
            || view.contract != expected.contract
            || view.origin != expected.origin
            || view.regions != expected.regions
            || view.source_regions != expected.source_regions
            || view.components != expected.components
            || view.nominals != expected.nominals
            || view.equality_required.is_some()
        {
            return Err(FactsError::invalid(
                &self.budget,
                "universal required carrier changed exact projected topology",
            ));
        }
        Ok(())
    }

    fn same_universal_required_selection(
        &self,
        left: &ProofEvidence,
        right: &ProofEvidence,
        depth: usize,
    ) -> Result<bool> {
        charged(&self.budget, self.budget.check_depth(depth))?;
        charged(&self.budget, self.budget.charge_work(1))?;
        Ok(match (&left.0.origin, &right.0.origin) {
            (ProofOrigin::Assumption, ProofOrigin::Assumption)
            | (ProofOrigin::TraitObject, ProofOrigin::TraitObject) => true,
            (ProofOrigin::OpaqueDeclared, ProofOrigin::OpaqueDeclared) => {
                self.eq_predicate(&left.0.goal, &right.0.goal)?
            }
            (ProofOrigin::Builtin { item: a }, ProofOrigin::Builtin { item: b }) => a == b,
            (ProofOrigin::FnItem { kind: a }, ProofOrigin::FnItem { kind: b }) => {
                a == b && self.eq_predicate(&left.0.goal, &right.0.goal)?
            }
            (ProofOrigin::Closure { kind: a }, ProofOrigin::Closure { kind: b }) => {
                a == b && self.eq_predicate(&left.0.goal, &right.0.goal)?
            }
            (
                ProofOrigin::Implementation {
                    declaration: a,
                    arguments: aa,
                },
                ProofOrigin::Implementation {
                    declaration: b,
                    arguments: ba,
                },
            ) => a == b && self.eq_types(aa, ba, 0)?,
            (
                ProofOrigin::ProjectedSubject {
                    evidence: a,
                    reductions: ar,
                },
                ProofOrigin::ProjectedSubject {
                    evidence: b,
                    reductions: br,
                },
            ) => {
                if ar.len() != br.len()
                    || !self.eq_predicate(&a.0.goal, &b.0.goal)?
                    || !self.same_universal_required_selection(a, b, depth + 1)?
                {
                    return Ok(false);
                }
                for (a, b) in ar.iter().zip(br.iter()) {
                    if !self.eq_ty(a.ty(), b.ty(), 0)? {
                        return Ok(false);
                    }
                }
                true
            }
            _ => false,
        })
    }

    /// Return this invocation's freshly authenticated replay to the carrier
    /// validator rather than discarding it and repeating the same instantiation.
    /// No cache, proof refund, or cross-context reuse is involved.
    pub(super) fn validate_universal_required_evidence(
        &self,
        evidence: &UniversalMethodEvidence,
        facts: &ValidatedFacts,
    ) -> Result<(InstantiatedSchema, Arc<UniversalMethodEvidence>)> {
        let (application, replay) =
            self.instantiate_universal_required_item(&evidence.call, facts)?;
        if evidence.dependencies.len() != replay.dependencies.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "universal required reductions changed",
            ));
        }
        for ((projection, proof), (expected, _)) in
            evidence.dependencies.iter().zip(replay.dependencies.iter())
        {
            if !self.same_projection(projection, expected)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "universal required projection changed",
                ));
            }
            match proof {
                equality::ProjectionReduction::Equality(proof) => {
                    proof.validate_for(projection, facts)?
                }
                equality::ProjectionReduction::Implementation(proof) => {
                    proof.validate_for(projection, facts)?
                }
            }
        }
        if evidence.implementation_dependencies.len() != replay.implementation_dependencies.len() {
            return Err(FactsError::invalid(
                &self.budget,
                "universal implementation reductions changed",
            ));
        }
        let context = ProofContext::from_facts(facts);
        for ((projection, proof, rhs), (expected, expected_proof, expected_rhs)) in evidence
            .implementation_dependencies
            .iter()
            .zip(replay.implementation_dependencies.iter())
        {
            if !self.same_projection(projection, expected)?
                || proof.0.world != expected_proof.0.world
                || proof.0.environment != expected_proof.0.environment
                || proof.0.phase != expected_proof.0.phase
                || !self.eq_predicate(&proof.0.goal, &expected_proof.0.goal)?
                || !self.same_universal_required_selection(proof, expected_proof, 0)?
            {
                return Err(FactsError::invalid(
                    &self.budget,
                    "universal implementation projection authority changed",
                ));
            }
            let reminted = self.implementation_equality_rhs(projection, proof, context)?;
            self.validate_equality_domain_in(
                projection,
                &reminted,
                context,
                &mut ProofScratch::default(),
            )?;
            if !self.eq_ty(rhs, &reminted, 0)? || !self.eq_ty(rhs, expected_rhs, 0)? {
                return Err(FactsError::invalid(
                    &self.budget,
                    "universal implementation projection RHS changed",
                ));
            }
        }
        if !self.eq_types(&evidence.arguments, &replay.arguments, 0)?
            || !self.eq_signature(&evidence.signature, &replay.signature)?
            || evidence.source_regions != replay.source_regions
            || evidence.selection.0.world != replay.selection.0.world
            || evidence.selection.0.environment != replay.selection.0.environment
            || evidence.selection.0.phase != replay.selection.0.phase
            || !self.same_universal_required_selection(&evidence.selection, &replay.selection, 0)?
            || !self.eq_predicate(&evidence.selection.0.goal, &replay.selection.0.goal)?
        {
            return Err(FactsError::invalid(
                &self.budget,
                "universal required method changed application or caller authority",
            ));
        }
        Ok((application, replay))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::frontend::{self, PlanCtx};

    fn fixture() -> (Arc<ValidatedTraitWorld>, Arc<ValidatedFacts>, TraitCall) {
        let program = frontend::parse(
            "trait Has<A,B>{type Out;fn get(self)->Self::Out;}fn caller<T:Has<i32,bool>>(x:T)-><T as Has<i32,bool>>::Out{x.get()}fn main(){}",
        ).unwrap();
        let ctx = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = ctx.trait_world().unwrap();
        // The fixture's first user function is `caller`; library and recipe
        // providers must not be mistaken for it by generic arity alone.
        let caller = ctx.function_decl(0).unwrap();
        let facts = world.callable_facts(caller).unwrap();
        let goal = facts
            .predicates
            .predicates
            .iter()
            .find_map(|predicate| match predicate {
                Predicate::Implements(goal) if goal.args.len() == 2 => Some(goal.clone()),
                _ => None,
            })
            .unwrap();
        let item = world
            .headers
            .items
            .iter()
            .find(|h| h.trait_decl == goal.declaration)
            .unwrap()
            .declaration;
        (
            world,
            facts,
            TraitCall {
                trait_ref: goal,
                item,
                method_args: Arc::from([]),
            },
        )
    }

    fn admission_projection(facts: &ValidatedFacts, call: &TraitCall) -> Ty {
        facts
            .world()
            .instantiate_universal_required_item(call, facts)
            .unwrap()
            .0
            .signature
            .result
    }

    #[test]
    fn solver_metadata_admission_warm_rejects_scope_source_and_borrow_changes() {
        let (world, facts, call) = fixture();
        let ty = admission_projection(&facts, &call);
        assert!(matches!(ty, Ty::AssociatedType(_)));
        let scratch = std::cell::RefCell::new(TraitScratch::default());
        for _ in 0..2 {
            let normal = facts
                .normalize_substituted_projection_type_scratch(
                    &ty,
                    facts.budget(),
                    &facts,
                    &scratch,
                )
                .unwrap();
            assert!(world.eq_ty(&normal, &ty, 0).unwrap());
            assert!(scratch.try_borrow_mut().is_ok());
        }
        let nested = Ty::Tuple(Arc::from([ty.clone(), Ty::Bool]));
        let nested_normal = facts
            .normalize_substituted_projection_type_scratch(
                &nested,
                facts.budget(),
                &facts,
                &scratch,
            )
            .unwrap();
        assert!(world.eq_ty(&nested_normal, &nested, 0).unwrap());
        let rows = scratch.borrow().proofs.replays.replay_counts();
        assert!(rows.2 > 0);
        // Even an equivalent cloned facts value is not this solver's original.
        // Ordered environment, subject, phase and registry are inseparable from it.
        for mode in 0..4 {
            let mut changed = facts.as_ref().clone();
            match mode {
                0 => {}
                1 => changed.subject = FactsSubject::SchemaItem(facts.declaration()),
                2 => changed.predicates = world.empty.clone(),
                _ => changed.closure_mode = Some(ir::ClosureKind::FnOnce),
            }
            assert!(
                changed
                    .normalize_substituted_projection_type_scratch(
                        &ty,
                        facts.budget(),
                        &facts,
                        &scratch,
                    )
                    .is_err()
            );
            assert_eq!(scratch.borrow().proofs.replays.replay_counts(), rows);
        }
        let (_, foreign, _) = fixture();
        assert!(
            foreign
                .normalize_substituted_projection_type_scratch(
                    &ty,
                    foreign.budget(),
                    &facts,
                    &scratch,
                )
                .is_err()
        );
        let foreign_budget = PlanBudget::default();
        assert!(
            facts
                .normalize_substituted_projection_type_scratch(
                    &ty,
                    &foreign_budget,
                    &facts,
                    &scratch,
                )
                .is_err()
        );
        let Ty::AssociatedType(projection) = &ty else {
            unreachable!()
        };
        let mut forged = projection.as_ref().clone();
        // A real method declaration cannot be forged into an associated item.
        forged.item = call.item;
        assert!(
            facts
                .normalize_substituted_projection_type_scratch(
                    &Ty::AssociatedType(Arc::new(forged)),
                    facts.budget(),
                    &facts,
                    &scratch,
                )
                .is_err()
        );
        assert_eq!(scratch.borrow().proofs.replays.replay_counts(), rows);
        let held = scratch.borrow_mut();
        assert!(
            facts
                .normalize_substituted_projection_type_scratch(
                    &ty,
                    facts.budget(),
                    &facts,
                    &scratch,
                )
                .is_err()
        );
        drop(held);
        facts
            .normalize_substituted_projection_type_scratch(&ty, facts.budget(), &facts, &scratch)
            .unwrap();
    }

    #[test]
    fn solver_metadata_admission_warm_depth_remains_checked() {
        let (_, facts, call) = fixture();
        let ty = admission_projection(&facts, &call);
        let scratch = std::cell::RefCell::new(TraitScratch::default());
        facts
            .normalize_substituted_projection_type_scratch(&ty, facts.budget(), &facts, &scratch)
            .unwrap();
        let rows = scratch.borrow().proofs.replays.replay_counts();
        let mut nested = ty.clone();
        for _ in 0..=facts.budget().limits().depth {
            nested = Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(nested),
            };
        }
        let failure = facts.budget().first_resource_failure();
        assert_eq!(failure, None);
        let error = facts
            .normalize_substituted_projection_type_scratch(
                &nested,
                facts.budget(),
                &facts,
                &scratch,
            )
            .unwrap_err();
        assert!(error.is_resource());
        assert_eq!(
            error.to_string(),
            "associated type traversal depth limit exceeded"
        );
        assert_eq!(facts.budget().first_resource_failure(), failure);
        assert_eq!(scratch.borrow().proofs.replays.replay_counts(), rows);
        // Static traversal exhaustion rejects this input without poisoning the ledger.
        facts
            .normalize_substituted_projection_type_scratch(&ty, facts.budget(), &facts, &scratch)
            .unwrap();
        assert_eq!(scratch.borrow().proofs.replays.replay_counts(), rows);
        assert_eq!(facts.budget().first_resource_failure(), failure);
    }

    #[test]
    fn solver_metadata_admission_warm_exact_work_short_and_live_poison() {
        for short in [false, true] {
            let (_, facts, call) = fixture();
            let ty = admission_projection(&facts, &call);
            let scratch = std::cell::RefCell::new(TraitScratch::default());
            facts
                .normalize_substituted_projection_type_scratch(
                    &ty,
                    facts.budget(),
                    &facts,
                    &scratch,
                )
                .unwrap();
            let before = facts.budget().usage_snapshot().work;
            facts
                .normalize_substituted_projection_type_scratch(
                    &ty,
                    facts.budget(),
                    &facts,
                    &scratch,
                )
                .unwrap();
            let cost = facts
                .budget()
                .usage_snapshot()
                .work
                .checked_sub(before)
                .unwrap();
            assert!(cost > 0);
            let rows = scratch.borrow().proofs.replays.replay_counts();
            let filler = facts
                .budget()
                .limits()
                .work
                .checked_sub(facts.budget().usage_snapshot().work)
                .and_then(|remaining| remaining.checked_sub(cost))
                .and_then(|remaining| remaining.checked_add(usize::from(short)))
                .unwrap();
            facts.budget().charge_work(filler).unwrap();
            let result = facts.normalize_substituted_projection_type_scratch(
                &ty,
                facts.budget(),
                &facts,
                &scratch,
            );
            assert_eq!(result.is_err(), short);
            assert_eq!(scratch.borrow().proofs.replays.replay_counts(), rows);
            if short {
                assert_eq!(
                    facts.budget().first_resource_failure().unwrap().kind,
                    ir::ResourceBudgetKind::Work
                );
                let failure = facts.budget().first_resource_failure();
                assert!(
                    facts
                        .normalize_substituted_projection_type_scratch(
                            &ty,
                            facts.budget(),
                            &facts,
                            &scratch,
                        )
                        .is_err()
                );
                assert_eq!(facts.budget().first_resource_failure(), failure);
            }
        }
        for planner in [false, true] {
            let (world, facts, call) = fixture();
            let ty = admission_projection(&facts, &call);
            let scratch = std::cell::RefCell::new(TraitScratch::default());
            facts
                .normalize_substituted_projection_type_scratch(
                    &ty,
                    facts.budget(),
                    &facts,
                    &scratch,
                )
                .unwrap();
            let rows = scratch.borrow().proofs.replays.replay_counts();
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
            for _ in 0..2 {
                assert!(
                    matches!(facts.normalize_substituted_projection_type_scratch(
                    &ty, facts.budget(), &facts, &scratch,
                ), Err(error) if error.is_resource())
                );
                assert_eq!(scratch.borrow().proofs.replays.replay_counts(), rows);
            }
        }
    }

    #[test]
    fn rigid_slice_implementation_pointer_rejects_changed_selection() {
        let program = frontend::parse(
            "trait Data{type Element;type Other;fn data(&mut self,value:Self::Other)->*mut Self::Element;}impl<T> Data for [T]{type Element=T;type Other=bool;fn data(&mut self,value:bool)->*mut T{0usize as *mut T}}fn caller<T>(s:&mut[T])->*mut T{s.data(true)}fn main(){}",
        ).unwrap();
        let ctx = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
        let world = ctx.trait_world().unwrap();
        let facts = world.callable_facts(ctx.function_decl(0).unwrap()).unwrap();
        let parameter = facts.env().ordered_params().next().unwrap();
        let implementation = world
            .headers
            .impls
            .iter()
            .find(|header| matches!(header.trait_ref.self_ty, Ty::Slice { .. }))
            .unwrap();
        let item = world
            .headers
            .items
            .iter()
            .find(|item| item.trait_decl == implementation.trait_ref.declaration)
            .unwrap();
        let call = TraitCall {
            trait_ref: TraitRef {
                declaration: implementation.trait_ref.declaration,
                self_ty: Ty::Slice {
                    element: Arc::new(Ty::Param(parameter)),
                },
                args: Arc::from([]),
            },
            item: item.declaration,
            method_args: Arc::from([]),
        };
        let context = ProofContext::from_facts(&facts);
        let membership = world
            .require_in(
                &Predicate::Implements(call.trait_ref.clone()),
                context,
                &mut ProofScratch::default(),
            )
            .unwrap();
        let projection = ir::AssociatedTypeProjection {
            trait_ref: call.trait_ref.clone(),
            item: implementation.associated_types[0].associated,
        };
        let rhs = world
            .implementation_equality_rhs(&projection, &membership, context)
            .unwrap();
        assert!(world.eq_ty(&rhs, &Ty::Param(parameter), 0).unwrap());
        let (application, evidence) = world
            .instantiate_universal_required_item(&call, &facts)
            .unwrap();
        assert!(
            world
                .eq_ty(
                    &application.signature.result,
                    &Ty::RawPtr {
                        kind: ir::BorrowKind::Mut,
                        pointee: Arc::new(Ty::Param(parameter)),
                    },
                    0
                )
                .unwrap()
        );
        world
            .validate_universal_required_evidence(&evidence, &facts)
            .unwrap();
        for result in [
            Ty::RawPtr {
                kind: ir::BorrowKind::Shared,
                pointee: Arc::new(Ty::Param(parameter)),
            },
            Ty::RawPtr {
                kind: ir::BorrowKind::Mut,
                pointee: Arc::new(Ty::Bool),
            },
        ] {
            let mut forged = (*evidence).clone();
            forged.signature.result = result;
            assert!(
                world
                    .validate_universal_required_evidence(&forged, &facts)
                    .is_err()
            );
        }
        let ProofOrigin::Implementation {
            declaration,
            arguments,
        } = &evidence.selection.0.origin
        else {
            panic!("expected exact source implementation")
        };
        for origin in [
            ProofOrigin::Implementation {
                declaration: call.item,
                arguments: arguments.clone(),
            },
            ProofOrigin::Implementation {
                declaration: *declaration,
                arguments: Arc::from([Ty::Bool]),
            },
        ] {
            let mut forged = (*evidence).clone();
            forged.selection = ProofEvidence(Arc::new(ProofRecord {
                world: evidence.selection.0.world,
                environment: evidence.selection.0.environment,
                phase: evidence.selection.0.phase,
                goal: evidence.selection.0.goal.clone(),
                origin,
            }));
            assert!(
                world
                    .validate_universal_required_evidence(&forged, &facts)
                    .is_err()
            );
        }
        assert_eq!(evidence.implementation_dependencies.len(), 2);
        let dependency = evidence.implementation_dependencies[0].clone();
        let reject_dependency = |changed: (ir::AssociatedTypeProjection, ProofEvidence, Ty)| {
            let mut forged = (*evidence).clone();
            let mut dependencies = evidence.implementation_dependencies.to_vec();
            dependencies[0] = changed;
            forged.implementation_dependencies = dependencies.into();
            assert!(
                world
                    .validate_universal_required_evidence(&forged, &facts)
                    .is_err()
            );
        };
        let mut changed = dependency.clone();
        changed.0.item = call.item;
        reject_dependency(changed);
        let mut changed = dependency.clone();
        changed.0.trait_ref.self_ty = Ty::Bool;
        reject_dependency(changed);
        for rhs in [
            Ty::Unit,
            Ty::RawPtr {
                kind: ir::BorrowKind::Shared,
                pointee: Arc::new(Ty::Param(parameter)),
            },
        ] {
            let mut changed = dependency.clone();
            changed.2 = rhs;
            reject_dependency(changed);
        }
        for which in 0..6 {
            let mut changed = dependency.clone();
            let mut proof = (*changed.1.0).clone();
            match which {
                0 => proof.environment += 1,
                1 => proof.phase = FactsPhase::Closed,
                2 => {
                    proof.goal = Predicate::Implements(TraitRef {
                        self_ty: Ty::Bool,
                        ..call.trait_ref.clone()
                    })
                }
                3 => {
                    proof.origin = ProofOrigin::Implementation {
                        declaration: call.item,
                        arguments: Arc::from([Ty::Param(parameter)]),
                    }
                }
                4 => {
                    proof.origin = ProofOrigin::Implementation {
                        declaration: *declaration,
                        arguments: Arc::from([Ty::Bool]),
                    }
                }
                _ => proof.world = fixture().0.id,
            }
            changed.1 = ProofEvidence(Arc::new(proof));
            reject_dependency(changed);
        }
        let mut reversed = evidence.implementation_dependencies.to_vec();
        reversed.reverse();
        for dependencies in [Vec::new(), vec![dependency], reversed] {
            let mut forged = (*evidence).clone();
            forged.implementation_dependencies = dependencies.into();
            assert!(
                world
                    .validate_universal_required_evidence(&forged, &facts)
                    .is_err()
            );
        }
    }

    #[test]
    fn rigid_required_method_keeps_projection_without_inventing_equality() {
        let (world, facts, call) = fixture();
        let (application, evidence) = world
            .instantiate_universal_required_item(&call, &facts)
            .unwrap();
        assert!(matches!(
            application.signature.result,
            Ty::AssociatedType(_)
        ));
        assert!(evidence.dependencies.is_empty());
        assert!(evidence.implementation_dependencies.is_empty());
        assert!(matches!(
            evidence.selection.0.origin,
            ProofOrigin::Assumption
        ));
        world
            .validate_universal_required_evidence(&evidence, &facts)
            .unwrap();
        world
            .universal_required_carrier_contract(&call, &facts)
            .unwrap()
            .validate_for(&facts)
            .unwrap();
    }

    #[test]
    fn projection_assignment_normal_form_requires_exact_universal_authority() {
        let (world, facts, call) = fixture();
        let (application, _) = world
            .instantiate_universal_required_item(&call, &facts)
            .unwrap();
        let normal = &application.signature.result;
        let context = ProofContext::from_facts(&facts);
        world
            .validate_associated_normal_form(
                None,
                normal,
                facts.env(),
                context,
                &mut ProofScratch::default(),
            )
            .unwrap();
        for phase in [FactsPhase::Schema, FactsPhase::Closed] {
            assert!(
                world
                    .validate_associated_normal_form(
                        None,
                        normal,
                        facts.env(),
                        ProofContext {
                            original: None,
                            predicates: &facts.predicates,
                            phase
                        },
                        &mut ProofScratch::default(),
                    )
                    .is_err()
            );
        }
        let (_, foreign, _) = fixture();
        assert!(
            world
                .validate_associated_normal_form(
                    None,
                    normal,
                    foreign.env(),
                    ProofContext::from_facts(&foreign),
                    &mut ProofScratch::default(),
                )
                .is_err()
        );
        let Ty::AssociatedType(projection) = normal else {
            panic!("rigid projection required")
        };
        let mut forged = projection.as_ref().clone();
        forged.trait_ref.args = Arc::from([Ty::Bool, Ty::Int(ir::IntTy::I32)]);
        assert!(
            world
                .validate_associated_normal_form(
                    None,
                    &Ty::AssociatedType(Arc::new(forged)),
                    facts.env(),
                    context,
                    &mut ProofScratch::default(),
                )
                .is_err()
        );
    }

    #[test]
    fn authenticated_required_replay_keeps_payload_and_rejects_live_short_or_poison() {
        for control in 0..3 {
            let (world, facts, call) = fixture();
            let (_, evidence) = world
                .instantiate_universal_required_item(&call, &facts)
                .unwrap();
            let budget = facts.budget();
            let before = budget.usage_snapshot();
            assert!(before.first_failure.is_none());
            assert!(!before.poisoned);
            let mut held = None;
            if control == 1 {
                held = Some(
                    budget
                        .reserve_bytes(budget.limits().bytes - before.bytes)
                        .unwrap(),
                );
                let full = budget.usage_snapshot();
                assert!(full.first_failure.is_none());
                assert!(!full.poisoned);
                assert_eq!(full.bytes, budget.limits().bytes);
            } else if control == 2 {
                budget.poison_ledger_for_tests();
                assert!(budget.usage_snapshot().poisoned);
            }
            let result = world.validate_universal_required_evidence(&evidence, &facts);
            if control == 0 {
                let (application, replay) = result.unwrap();
                assert!(
                    world
                        .eq_signature(&application.signature, &evidence.signature)
                        .unwrap()
                );
                assert!(
                    world
                        .eq_signature(&replay.signature, &evidence.signature)
                        .unwrap()
                );
                assert_eq!(replay.dependencies.len(), evidence.dependencies.len());
                assert_eq!(
                    replay.implementation_dependencies.len(),
                    evidence.implementation_dependencies.len()
                );
            } else {
                assert!(result.is_err());
                if control == 1 {
                    let failure = budget.first_resource_failure().unwrap();
                    assert_eq!(failure.kind, ir::ResourceBudgetKind::Bytes);
                    assert_eq!(failure.used, failure.limit);
                }
            }
            drop(held);
            assert!(budget.usage_snapshot().bytes >= before.bytes);
            assert_eq!(evidence.selection.0.world, world.id);
        }
    }

    #[test]
    fn universal_required_method_rejects_forged_arguments_signature_and_caller() {
        let (world, facts, call) = fixture();
        let (_, evidence) = world
            .instantiate_universal_required_item(&call, &facts)
            .unwrap();
        let mut forged = (*evidence).clone();
        forged.signature.result = Ty::Bool;
        assert!(
            world
                .validate_universal_required_evidence(&forged, &facts)
                .is_err()
        );
        let mut forged = (*evidence).clone();
        let mut actuals = forged.call.trait_ref.args.to_vec();
        actuals.reverse();
        forged.call.trait_ref.args = actuals.into();
        assert!(
            world
                .validate_universal_required_evidence(&forged, &facts)
                .is_err()
        );
        let mut wrong_arity = call.clone();
        wrong_arity.method_args = Arc::from([Ty::Bool]);
        assert!(
            world
                .instantiate_universal_required_item(&wrong_arity, &facts)
                .is_err()
        );
        let (_, foreign, _) = fixture();
        assert!(
            world
                .validate_universal_required_evidence(&evidence, &foreign)
                .is_err()
        );
    }
}
