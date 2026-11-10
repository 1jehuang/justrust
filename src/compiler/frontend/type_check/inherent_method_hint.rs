//! Existing inherent inference, with explicit absence versus unresolved evidence.
use super::receiver_result_hint::InherentMethodCandidate;
use super::*;

pub(super) enum InherentMethodInference {
    Missing,
    Applicable(Option<Vec<ir::Ty>>),
}

impl Checker<'_> {
    pub(super) fn inherent_method_actuals(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        args: &[Expr],
        explicit: &[Type],
        expected: Option<&ir::Ty>,
        filter: Option<ReceiverKind>,
    ) -> Result<InherentMethodInference, String> {
        let plan = self.signatures.plan()?;
        let mut root = receiver;
        if filter.is_none() {
            // Compatibility for associated owner inference. Dot inference is
            // already inside the canonical receiver search and must not peel
            // a reference or skip a candidate depth independently.
            while let ir::Ty::Ref { referent, .. } = root {
                root = referent;
            }
        }
        let (id, owner_bindings) = match self.inherent_method_candidate(root, name, filter)? {
            InherentMethodCandidate::Missing => return Ok(InherentMethodInference::Missing),
            InherentMethodCandidate::Ambiguous => {
                return Ok(InherentMethodInference::Applicable(None));
            }
            InherentMethodCandidate::Unique(id, bindings) => (id, bindings),
        };
        let info = plan.declaration(id)?;
        let schema = plan.function_schema(id)?;
        if info.param_env.arity() == 0 {
            return Ok(InherentMethodInference::Applicable(Some(Vec::new())));
        }
        let params = &schema.signature.params;
        let formals = match params.len().checked_sub(args.len()) {
            Some(0) => &params[..],
            Some(1) => &params[1..],
            _ => return Ok(InherentMethodInference::Applicable(None)),
        };
        let mut solver = Inference::new(self, info.param_env.clone())?;
        if !explicit.is_empty() {
            let anonymous = plan
                .registry
                .source_callable(id)
                .map_or(0, |f| f.impl_params);
            if explicit.len() + anonymous != info.param_env.arity() {
                return Err("E0107: wrong number of method generic arguments".into());
            }
            for (slot, ty) in explicit.iter().enumerate() {
                let formal = self.infer_node(InferKind::Slot(slot))?;
                let actual = self.partial_type_term(ty, 0)?;
                solver.constrain(&formal, &actual, 0)?;
            }
        }
        self.infer_storage(formals.len() + usize::from(expected.is_some()))?;
        let mut constraints = Vec::new();
        if let Some(expected) = expected {
            let formal = self.method_owner_term(
                &solver.schema(&schema.signature.result, 0)?,
                &owner_bindings,
                0,
            )?;
            let actual = self.infer_type_term(expected, None, 0)?;
            constraints.push((formal, actual));
        }
        for (formal, expr) in formals.iter().zip(args) {
            // Arguments that do not mention a method parameter carry no
            // evidence; skip them (they are checked normally afterwards).
            if !inferred_owner::mentions_param(formal) {
                continue;
            }
            let Ok(evidence) = self.inference_evidence(expr, 0) else {
                return Ok(InherentMethodInference::Applicable(None));
            };
            let Ok(formal) = solver
                .schema(formal, 0)
                .and_then(|term| self.method_owner_term(&term, &owner_bindings, 0))
            else {
                return Ok(InherentMethodInference::Applicable(None));
            };
            constraints.push((formal, evidence));
        }
        // Projection families are not injective. The expected result is an
        // equality to prove after input constraints close its arguments, never
        // a source of invented method actuals. Reuse the shared pending queue.
        self.infer_storage(constraints.len())?;
        if matches!(
            solver.constrain_projected_terms_progress(constraints.clone(), false)?,
            super::projected_formals::ProjectedConstraintProgress::Conflict(_)
        ) {
            return Ok(InherentMethodInference::Applicable(None));
        }
        // A known argument can determine another method parameter through a
        // bound such as R: RangeBounds<Q>. Enumerate canonical applications of
        // the resolved R, not display names or a preferred implementation.
        // Ambiguous or incomplete frontiers supply no equations. Selection
        // below must still prove the complete instantiated method obligations.
        if let ir::FactsCtx::Validated(facts) = self.facts() {
            for _ in 0..info.param_env.arity() {
                let mut progress = false;
                for predicate in schema.predicates.predicates() {
                    plan.budget.charge_work(1)?;
                    let ir::Predicate::Implements(application) = predicate else {
                        continue;
                    };
                    if application.args.is_empty() {
                        continue;
                    }
                    let mut unresolved = false;
                    for argument in application.args.iter() {
                        let argument = self.method_owner_term(
                            &solver.schema(argument, 0)?,
                            &owner_bindings,
                            0,
                        )?;
                        unresolved |= solver
                            .resolve_projection_input(&argument, 0, false)?
                            .is_none();
                    }
                    if !unresolved {
                        continue;
                    }
                    let owner = self.method_owner_term(
                        &solver.schema(&application.self_ty, 0)?,
                        &owner_bindings,
                        0,
                    )?;
                    let Some(owner) = solver.resolve_projection_input(&owner, 0, false)? else {
                        continue;
                    };
                    let (applications, deferred) = plan.inferred_trait_applications(
                        &owner,
                        application.declaration,
                        facts,
                        &plan.budget,
                    )?;
                    let [candidate] = applications.as_slice() else {
                        continue;
                    };
                    if deferred || candidate.args.len() != application.args.len() {
                        continue;
                    }
                    for (formal, actual) in application.args.iter().zip(candidate.args.iter()) {
                        let formal =
                            self.method_owner_term(&solver.schema(formal, 0)?, &owner_bindings, 0)?;
                        if solver
                            .resolve_projection_input(&formal, 0, false)?
                            .is_some()
                        {
                            continue;
                        }
                        let actual = self.infer_type_term(actual, None, 0)?;
                        solver.constrain(&formal, &actual, 0)?;
                        progress = true;
                    }
                }
                if !progress {
                    break;
                }
            }
        }
        // The closure argument can determine a method parameter that occurs
        // only in its callable Output equality, e.g. Result::map_err's G.
        // As for trait methods, normalize the original predicate after binding
        // the receiver's impl parameters. Final method selection still proves
        // the complete instantiated signature and its obligations.
        for predicate in schema.predicates.predicates() {
            plan.budget.charge_work(1)?;
            // `K: Borrow<Q>` with K bound by the receiver (u32) and only the
            // reflexive impl applicable: Q = u32 before literal defaulting.
            if let ir::Predicate::Implements(application) = predicate
                && application.args.len() == 1
                && let ir::Ty::Param(slot) = &application.args[0]
            {
                let self_ty = super::trait_method_hint::bind_owner(
                    &application.self_ty,
                    &owner_bindings,
                    &plan.budget,
                )?;
                let open = match solver
                    .core
                    .slots()
                    .get(slot.index as usize)
                    .cloned()
                    .flatten()
                {
                    Some(bound) => solver.resolve(&bound, false, 0)?.is_none(),
                    None => true,
                };
                if open
                    && !inferred_owner::mentions_param(&self_ty)
                    && self.unique_reflexive_impl(application.declaration, &self_ty)?
                {
                    let formal = solver.schema(&application.args[0], 0)?;
                    let actual = self.infer_type_term(&self_ty, None, 0)?;
                    solver.constrain(&formal, &actual, 0)?;
                }
            }
            if let ir::Predicate::ProjectionEq { projection, ty } = predicate
                && !matches!(ty, ir::Ty::AssociatedType(_))
                && inferred_owner::mentions_param(ty)
            {
                let projection = super::trait_method_hint::bind_owner(
                    &ir::Ty::AssociatedType(Arc::new(projection.clone())),
                    &owner_bindings,
                    &plan.budget,
                )?;
                let ty = super::trait_method_hint::bind_owner(ty, &owner_bindings, &plan.budget)?;
                solver.constrain_projection_equality(
                    &projection,
                    &ty,
                    Some(&owner_bindings),
                    true,
                )?;
            }
        }
        // Output equalities may have supplied previously absent method slots.
        // Revisit the pending equations only now, with numeric fallback last.
        if !matches!(
            solver.constrain_projected_terms_progress(constraints, true)?,
            super::projected_formals::ProjectedConstraintProgress::Complete
        ) {
            return Ok(InherentMethodInference::Applicable(None));
        }
        Ok(InherentMethodInference::Applicable(solver.finish(true)?))
    }
}
