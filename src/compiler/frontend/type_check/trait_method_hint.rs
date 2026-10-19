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
    ) -> Result<Option<Vec<ir::Ty>>, String> {
        let plan = self.signatures.plan()?;
        let item = plan
            .trait_item(application.declaration, name, &plan.budget)?
            .ok_or("unknown required trait method")?;
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
        if schema.signature.params.len() != arguments.len() {
            return Err("trait method argument count mismatch".into());
        }
        let mut solver = Inference::new(self, own)?;
        for (slot, actual) in explicit.iter().enumerate() {
            let formal = self.infer_node(InferKind::Slot(slot))?;
            let actual = self.partial_type_term(actual, 0)?;
            solver.constrain(&formal, &actual, 0)?;
        }
        if let Some(expected) = expected {
            let formal =
                self.method_owner_term(&solver.schema(&schema.signature.result, 0)?, &bindings, 0)?;
            let evidence = self.infer_type_term(expected, None, 0)?;
            solver.constrain(&formal, &evidence, 0)?;
        }
        for (formal, argument) in schema.signature.params.iter().zip(arguments) {
            let formal = self.method_owner_term(&solver.schema(formal, 0)?, &bindings, 0)?;
            let evidence = self.inference_evidence(argument, 0)?;
            solver.constrain(&formal, &evidence, 0)?;
        }
        solver.finish(true)
    }
}
