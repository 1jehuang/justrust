//! Infer only a required trait member's own generic parameters. The trait
//! application and its ordered owner actuals are fixed before inference starts.
use super::*;

impl Checker<'_> {
    pub(super) fn inferred_trait_method_actuals(
        &self,
        application: &ir::TraitRef,
        name: &AstIdent,
        explicit: &[Type],
        arguments: &[Expr],
        expected: Option<&ir::Ty>,
        dot: bool,
    ) -> Result<Option<Vec<ir::Ty>>, String> {
        let plan = self.signatures.plan()?;
        let item = plan
            .trait_item(application.declaration, name, &plan.budget)?
            .ok_or("unknown required trait method")?;
        if dot && plan.declaration(item)?.receiver == ReceiverKind::None {
            return Err("associated function without self is not dot-callable".into());
        }
        let own = plan.param_env(item)?;
        if !explicit.is_empty() && explicit.len() != own.arity() {
            return Err("E0107: wrong number of trait method generic arguments".into());
        }
        if own.arity() == 0 {
            return Ok(Some(Vec::new()));
        }
        let ir::FactsCtx::Validated(facts) = self.facts() else {
            return Err("trait method inference requires canonical caller facts".into());
        };
        match facts
            .service()
            .prove_goal(&ir::Predicate::Implements(application.clone()), facts)
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
        {
            tr::ProofOutcome::Proved(_) => {}
            tr::ProofOutcome::NotProven => return Err("trait obligation not proven".into()),
        }
        let owner = plan.param_env(application.declaration)?;
        if owner.arity() != application.args.len() + 1 {
            return Err("trait argument arity mismatch".into());
        }
        self.infer_storage(owner.arity())?;
        let mut bindings = HashMap::new();
        for (index, actual) in std::iter::once(&application.self_ty)
            .chain(application.args.iter())
            .enumerate()
        {
            self.facts().validate_ty_with_budget(actual, &plan.budget)?;
            bindings.insert(
                ir::ParamId {
                    owner: owner.binder(),
                    index: index as u16,
                },
                actual.clone(),
            );
        }
        // This is the original source schema, used only to build inference
        // terms. It grants no executable authority. Final select_trait checks
        // the complete application, inferred method actuals and obligations.
        let schema = plan.raw_schema(item)?;
        let offset = usize::from(dot);
        if schema.signature.params.len() != arguments.len() + offset {
            return Err("trait method argument count mismatch".into());
        }
        let mut solver = Inference::new(self, own)?;
        for (slot, actual) in explicit.iter().enumerate() {
            let formal = self.infer_node(InferKind::Slot(slot))?;
            let actual = self.partial_type_term(actual, 0)?;
            solver.constrain(&formal, &actual, 0)?;
        }
        self.infer_storage(arguments.len() + usize::from(expected.is_some()))?;
        let mut constraints = Vec::new();
        if let Some(expected) = expected {
            let formal =
                self.method_owner_term(&solver.schema(&schema.signature.result, 0)?, &bindings, 0)?;
            let evidence = self.infer_type_term(expected, None, 0)?;
            constraints.push((formal, evidence));
        }
        for (formal, argument) in schema.signature.params[offset..].iter().zip(arguments) {
            let formal = self.method_owner_term(&solver.schema(formal, 0)?, &bindings, 0)?;
            let evidence = self.inference_evidence(argument, 0)?;
            constraints.push((formal, evidence));
        }
        if !solver.constrain_projected_terms(constraints, true)? {
            return Ok(None);
        }
        // `F: FnMut(Self::Item) -> B`: once the closure argument and the
        // owner's Self are known, the normalized `Output` is evidence for B.
        for predicate in schema.predicates.iter() {
            if let ir::Predicate::ProjectionEq { projection, ty } = predicate
                && !matches!(ty, ir::Ty::AssociatedType(_))
                && inferred_owner::mentions_param(ty)
            {
                let projection = bind_owner(
                    &ir::Ty::AssociatedType(Arc::new(projection.clone())),
                    &bindings,
                    &self.signatures.plan()?.budget,
                )?;
                let ty = bind_owner(ty, &bindings, &self.signatures.plan()?.budget)?;
                solver.constrain_projection_equality(&projection, &ty, Some(&bindings), true)?;
            }
        }
        solver.finish(true)
    }
}

/// Replace enclosing trait-owner parameters (Self and trait arguments) with
/// the selected application's actuals. Method-own parameters stay slots.
pub(super) fn bind_owner(
    ty: &ir::Ty,
    bindings: &HashMap<ir::ParamId, ir::Ty>,
    budget: &ir::PlanBudget,
) -> Result<ir::Ty, String> {
    budget.charge_work(1)?;
    Ok(match ty {
        ir::Ty::Param(p) => bindings.get(p).cloned().unwrap_or_else(|| ty.clone()),
        ir::Ty::Ref { kind, referent } => {
            budget
                .retain_bytes(std::mem::size_of::<ir::Ty>() + 2 * std::mem::size_of::<usize>())?;
            ir::Ty::Ref {
                kind: *kind,
                referent: Arc::new(bind_owner(referent, bindings, budget)?),
            }
        }
        ir::Ty::PhantomData(marker) => ir::Ty::PhantomData(marker.try_map(
            budget,
            |error| error,
            |ty| bind_owner(ty, bindings, budget),
        )?),
        ir::Ty::FnPointer(pointer) => ir::Ty::FnPointer(pointer.try_map(
            budget,
            |e| e,
            |t| bind_owner(t, bindings, budget),
        )?),
        ir::Ty::Tuple(ts) => {
            let mut mapped = ir::FnPointerTy::storage(ts.len(), budget)?;
            for t in ts.iter() {
                mapped.push(bind_owner(t, bindings, budget)?);
            }
            ir::Ty::Tuple(ir::FnPointerTy::slice(mapped, budget)?)
        }
        ir::Ty::AssociatedType(p) => {
            let mut args = ir::FnPointerTy::storage(p.trait_ref.args.len(), budget)?;
            for t in p.trait_ref.args.iter() {
                args.push(bind_owner(t, bindings, budget)?);
            }
            budget.retain_bytes(
                std::mem::size_of::<ir::AssociatedTypeProjection>()
                    + 2 * std::mem::size_of::<usize>(),
            )?;
            let mut p = p.as_ref().clone();
            p.trait_ref.self_ty = bind_owner(&p.trait_ref.self_ty, bindings, budget)?;
            p.trait_ref.args = ir::FnPointerTy::slice(args, budget)?;
            ir::Ty::AssociatedType(Arc::new(p))
        }
        other => other.clone(),
    })
}
