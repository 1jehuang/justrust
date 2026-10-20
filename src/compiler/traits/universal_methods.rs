//! Required method authority for a rigid universal receiver.
//!
//! A caller bound proves membership, not an implementation assignment. Keep
//! unresolved associated types attached to their exact declaration and ordered
//! application until a closed instance can select and normalize an impl.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct UniversalMethodEvidence {
    pub(super) call: TraitCall,
    pub(super) arguments: Arc<[Ty]>,
    pub(super) selection: ProofEvidence,
    pub(super) signature: Signature,
    pub(super) source_regions: Arc<ir::SourceRegionEnv>,
    pub(super) dependencies: Arc<[(ir::AssociatedTypeProjection, equality::ProjectionReduction)]>,
}

impl ValidatedFacts {
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
        let context = ProofContext::from_facts(self);
        self.world
            .map_universal_projection_type(ty, self.env(), 0, &mut |projection| {
                let raw = Ty::AssociatedType(Arc::new(projection.clone()));
                if !self.world.raw_associated_ty_is_closed(&raw, 0)? {
                    self.world
                        .validate_universal_projection(projection, self.env())?;
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
            .validate_associated_normal_form(normal, facts.env(), context, &mut Vec::new())
            .unwrap();
        for phase in [FactsPhase::Schema, FactsPhase::Closed] {
            assert!(
                world
                    .validate_associated_normal_form(
                        normal,
                        facts.env(),
                        ProofContext {
                            predicates: &facts.predicates,
                            phase
                        },
                        &mut Vec::new(),
                    )
                    .is_err()
            );
        }
        let (_, foreign, _) = fixture();
        assert!(
            world
                .validate_associated_normal_form(
                    normal,
                    foreign.env(),
                    ProofContext::from_facts(&foreign),
                    &mut Vec::new(),
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
                    &Ty::AssociatedType(Arc::new(forged)),
                    facts.env(),
                    context,
                    &mut Vec::new(),
                )
                .is_err()
        );
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
            Ty::AssociatedType(projection) => reduce(projection)?,
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
    ) -> Result<()> {
        self.check_context(context)?;
        if !self.signature_contains_projection(signature)? {
            return Ok(());
        }
        let mut normalize = |projection: &ir::AssociatedTypeProjection| -> Result<Ty> {
            let raw = Ty::AssociatedType(Arc::new(projection.clone()));
            if !self.raw_associated_ty_is_closed(&raw, 0)?
                && !self.checked_closure_projection(projection, context.env())?
            {
                return Ok(raw);
            }
            let (normal, dependencies) =
                self.normalize_closed_projected_type(&raw, context, false)?;
            for dependency in dependencies.iter() {
                self.validate_projected_dependency(dependency, context)?;
            }
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
    ) -> Result<Arc<[Predicate]>> {
        self.check_context(context)?;
        let mut normalize = |projection: &ir::AssociatedTypeProjection| -> Result<Ty> {
            let raw = Ty::AssociatedType(Arc::new(projection.clone()));
            if !self.raw_associated_ty_is_closed(&raw, 0)?
                && !self.checked_closure_projection(projection, context.env())?
            {
                return Ok(raw);
            }
            let (normal, dependencies) =
                self.normalize_closed_projected_type(&raw, context, false)?;
            for dependency in dependencies.iter() {
                self.validate_projected_dependency(dependency, context)?;
            }
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
            &mut Vec::new(),
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
        if facts.phase() != FactsPhase::Universal
            || self.goal_is_closed(&call.trait_ref, facts.env())?
        {
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
            &mut Vec::new(),
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
        let mut dependencies = Vec::new();
        let mut reduce = |projection: &ir::AssociatedTypeProjection| -> Result<Ty> {
            if self.checked_closure_projection(projection, facts.env())?
                || self
                    .opaque_associated_binding(projection, facts.env())?
                    .is_some()
                || self.goal_is_closed(&projection.trait_ref, facts.env())?
                || self
                    .declared_equality_rhs(projection, &facts.predicates.predicates, facts.env())?
                    .is_some()
            {
                let proof = self.normalize_projection_for_facts(projection, facts)?;
                self.validate_equality_domain_in(projection, proof.ty(), context, &mut Vec::new())?;
                let ty = proof.ty().clone();
                push(&self.budget, &mut dependencies, (projection.clone(), proof))?;
                return Ok(ty);
            }
            self.require_in(
                &Predicate::Implements(projection.trait_ref.clone()),
                context,
                &mut Vec::new(),
            )?;
            retain::<ir::AssociatedTypeProjection>(&self.budget, 1)?;
            Ok(Ty::AssociatedType(Arc::new(projection.clone())))
        };
        for parameter in &mut instantiated.signature.params {
            *parameter =
                self.map_universal_projection_type(parameter, facts.env(), 0, &mut reduce)?;
        }
        instantiated.signature.result = self.map_universal_projection_type(
            &instantiated.signature.result,
            facts.env(),
            0,
            &mut reduce,
        )?;
        for predicate in instantiated.predicates.iter() {
            self.require_in(predicate, context, &mut Vec::new())?;
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
        let (contract, regions) = self.derive_carriers(&signature, &source_regions)?;
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
        Ok(CarrierContractView {
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
        self.validate_universal_required_evidence(authority, facts)?;
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
        let (application, replay) =
            self.instantiate_universal_required_item(&authority.call, facts)?;
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

    pub(super) fn validate_universal_required_evidence(
        &self,
        evidence: &UniversalMethodEvidence,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        let (_, replay) = self.instantiate_universal_required_item(&evidence.call, facts)?;
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
        Ok(())
    }
}
