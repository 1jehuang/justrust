//! Discover generic trait-method constraints at the canonical receiver frontier.
//! These are inference terms only. No closure type, proof or executable target is
//! fabricated: ordinary method selection still discharges the final application.
use super::super::method_argument_selection::MethodArguments;
use super::*;

impl Solver<'_, '_, '_> {
    pub(super) fn trait_method_constraints(
        &mut self,
        receiver: &ir::Ty,
        name: AstIdent,
        arguments: &[Term],
        explicit: &[Term],
    ) -> Result<Option<Term>, String> {
        if !arguments
            .iter()
            .any(|argument| matches!(self.domain(self.peel_refs(*argument)), Domain::Callable(_)))
        {
            return Ok(None);
        }
        let checker = self.checker;
        let plan = checker.signatures.plan()?;
        let ir::FactsCtx::Validated(facts) = checker.facts() else {
            return Ok(None);
        };
        let mut discovered = None;
        // Reuse the ordinary receiver/adjustment and inherent-before-trait order.
        // Returning Deferred stops at the first applicable frontier, rather than
        // silently considering a lower-priority trait while its arguments wait.
        let _ = plan.select_method_with_arguments(
            receiver,
            &name,
            checker.facts(),
            &checker.module,
            &plan.budget,
            |owner, mode, traits_only| {
                if !traits_only {
                    if let Some((nominal, _)) = nominal_parts(owner) {
                        if self.candidate(nominal, name)?.is_some() {
                            return Ok(MethodArguments::Deferred);
                        }
                    }
                    return Ok(MethodArguments::Missing);
                }
                let candidates = plan.method_trait_candidates(
                    owner,
                    &name,
                    checker.facts(),
                    &checker.module,
                    &plan.budget,
                )?;
                let mut applicable = None;
                for (application, item) in candidates.as_slice() {
                    checker.infer_work(0)?;
                    if receiver_mode(plan.declaration(*item)?.receiver) != receiver_mode(mode) {
                        continue;
                    }
                    if matches!(
                        facts
                            .service()
                            .prove_goal(&ir::Predicate::Implements(application.clone()), facts,)
                            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?,
                        tr::ProofOutcome::NotProven
                    ) {
                        continue;
                    }
                    if applicable.replace((application.clone(), *item)).is_some() {
                        return Err("ambiguous in-scope trait item".into());
                    }
                }
                let Some(application) = applicable else {
                    return Ok(MethodArguments::Missing);
                };
                discovered = Some(application);
                Ok(MethodArguments::Deferred)
            },
        )?;
        let Some((application, item)) = discovered else {
            return Ok(None);
        };
        let schema = plan.raw_schema(item)?;
        if schema.signature.params.len() != arguments.len() + 1 {
            return Err(self.user_error("E0061: wrong number of method arguments"));
        }
        let own = plan.param_env(item)?;
        if !explicit.is_empty() && explicit.len() != own.arity() {
            return Err(self.user_error("E0107: wrong number of method generic arguments"));
        }
        let owner = plan.param_env(application.declaration)?;
        if owner.arity() != application.args.len() + 1 {
            return Err("trait constraint owner arity changed".into());
        }
        let mut params = self.parameters(&schema.env)?;
        for (index, actual) in std::iter::once(&application.self_ty)
            .chain(application.args.iter())
            .enumerate()
        {
            let term = self.fixed(actual.clone())?;
            params.insert(
                ir::ParamId {
                    owner: owner.binder(),
                    index: index as u16,
                },
                term,
            );
        }
        for (index, term) in explicit.iter().enumerate() {
            params.insert(
                ir::ParamId {
                    owner: own.binder(),
                    index: index as u16,
                },
                *term,
            );
        }
        for (argument, formal) in arguments.iter().zip(schema.signature.params.iter().skip(1)) {
            let formal = self.schema(formal, &params, 0)?;
            self.coerce(*argument, formal)?;
        }
        self.callable_predicates(&schema.predicates, &params)?;
        self.schema(&schema.signature.result, &params, 0).map(Some)
    }
}
