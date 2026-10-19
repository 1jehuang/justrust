//! Infer method arguments at the canonical receiver candidate, not by peeling
//! references or choosing a trait by spelling. Final selection checks all facts.
use super::super::method_argument_selection::MethodArguments;
use super::inherent_method_hint::InherentMethodInference;
use super::*;
use std::borrow::Cow;

impl Checker<'_> {
    pub(super) fn method_selection_with_inference(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        explicit: &[Type],
        arguments: &[Expr],
        expected: Option<&ir::Ty>,
    ) -> Result<Option<MethodSelection>, String> {
        self.method_selection_with_inference_state(receiver, name, explicit, arguments, expected)
            .map(|(selection, _)| selection)
    }

    /// Preserve an unresolved higher-priority candidate when considering an
    /// unsizing fallback. A deferred candidate is not an absent method.
    pub(super) fn method_selection_with_inference_state(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        explicit: &[Type],
        arguments: &[Expr],
        expected: Option<&ir::Ty>,
    ) -> Result<(Option<MethodSelection>, bool), String> {
        let plan = self.signatures.plan()?;
        let written =
            if !explicit.is_empty() && !explicit.iter().any(partial_annotation::has_nested_infer) {
                self.infer_storage(explicit.len())?;
                Some(
                    explicit
                        .iter()
                        .map(|actual| self.checked_type_actual(actual))
                        .collect::<Result<Vec<_>, _>>()?,
                )
            } else {
                None
            };
        let mut deferred = false;
        let selection = plan.select_method_with_arguments(
            receiver,
            name,
            self.facts(),
            &self.module,
            &plan.budget,
            |owner, mode, traits_only| {
                if let Some(written) = &written {
                    return Ok(MethodArguments::Ready(Cow::Borrowed(written)));
                }
                if !traits_only {
                    return Ok(
                        match self.inherent_method_actuals(
                            owner,
                            name,
                            arguments,
                            explicit,
                            expected,
                            Some(mode),
                        )? {
                            InherentMethodInference::Missing => MethodArguments::Missing,
                            InherentMethodInference::Applicable(None) => {
                                deferred = true;
                                MethodArguments::Deferred
                            }
                            InherentMethodInference::Applicable(Some(actuals)) => {
                                MethodArguments::Ready(Cow::Owned(actuals))
                            }
                        },
                    );
                }
                let ir::FactsCtx::Validated(facts) = self.facts() else {
                    return Err("method inference requires canonical caller facts".into());
                };
                let candidates = plan.method_trait_candidates(
                    owner,
                    name,
                    self.facts(),
                    &self.module,
                    &plan.budget,
                )?;
                let mut selected = None;
                for (application, item) in candidates.as_slice() {
                    plan.budget.charge_work(1)?;
                    if receiver_mode(plan.declaration(*item)?.receiver) != receiver_mode(mode) {
                        continue;
                    }
                    match facts
                        .service()
                        .prove_goal(&ir::Predicate::Implements(application.clone()), facts)
                        .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
                    {
                        tr::ProofOutcome::NotProven => continue,
                        tr::ProofOutcome::Proved(_) => {}
                    }
                    if selected.replace(application).is_some() {
                        return Err("ambiguous in-scope trait item".into());
                    }
                }
                let Some(application) = selected else {
                    return Ok(MethodArguments::Missing);
                };
                Ok(
                    match self.inferred_trait_method_actuals(
                        application,
                        name,
                        explicit,
                        arguments,
                        expected,
                        true,
                    )? {
                        Some(actuals) => MethodArguments::Ready(Cow::Owned(actuals)),
                        None => {
                            deferred = true;
                            MethodArguments::Deferred
                        }
                    },
                )
            },
        )?;
        Ok((selection, deferred))
    }
}
