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

    pub(super) fn method_selection_for_hint(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        explicit: &[Type],
        arguments: &[Expr],
    ) -> Result<Option<MethodSelection>, String> {
        let (selection, deferred) = self.method_selection_with_inference_mode(
            receiver, name, explicit, arguments, None, true,
        )?;
        if selection.is_some() || deferred {
            return Ok(selection);
        }
        match array_unsize_carrier(receiver) {
            Some(carrier) => self
                .method_selection_with_inference_mode(
                    &carrier, name, explicit, arguments, None, true,
                )
                .map(|(selection, _)| selection),
            None => Ok(None),
        }
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
        self.method_selection_with_inference_mode(
            receiver, name, explicit, arguments, expected, false,
        )
    }

    fn method_selection_with_inference_mode(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        explicit: &[Type],
        arguments: &[Expr],
        expected: Option<&ir::Ty>,
        hint_only: bool,
    ) -> Result<(Option<MethodSelection>, bool), String> {
        let plan = self.signatures.plan()?;
        let realized_receiver = if self.nominal_inputs.is_some() {
            Some(self.realize_constructor_actual(receiver, 0)?)
        } else {
            None
        };
        let receiver = realized_receiver.as_ref().unwrap_or(receiver);
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
        let selection = plan.select_method_with_arguments_scratch(
            receiver,
            name,
            self.facts(),
            (&self.module, &plan.budget),
            &self.trait_scratch,
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
                let mut same: Vec<(&ir::TraitRef, ir::DeclId)> = Vec::new();
                for (application, item) in candidates.as_slice() {
                    plan.budget.charge_work(1)?;
                    if receiver_mode(plan.declaration(*item)?.receiver) != receiver_mode(mode) {
                        continue;
                    }
                    match facts
                        .service()
                        .prove_goal_with(
                            &ir::Predicate::Implements(application.clone()),
                            facts,
                            &mut self.trait_scratch.borrow_mut(),
                        )
                        .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
                    {
                        tr::ProofOutcome::NotProven => continue,
                        tr::ProofOutcome::Proved(_) => {}
                    }
                    if let Some(previous) = selected
                        && same.is_empty()
                    {
                        let previous: &ir::TraitRef = previous;
                        if previous.declaration != application.declaration {
                            return Err("ambiguous in-scope trait item".into());
                        }
                        same.push((previous, *item));
                    }
                    if !same.is_empty() {
                        if same[0].0.declaration != application.declaration {
                            return Err("ambiguous in-scope trait item".into());
                        }
                        same.push((application, *item));
                    }
                    selected = Some(application);
                }
                if !same.is_empty() {
                    // Several applications of one trait (`Into<T>`): the
                    // expected result type picks the unique one whose method
                    // result is exactly it.
                    let Some(expected) = expected else {
                        if hint_only {
                            // A context-free hint cannot choose among result
                            // applications. Defer, preserving receiver priority,
                            // until the actual call receives its field/context.
                            deferred = true;
                            return Ok(MethodArguments::Deferred);
                        }
                        return Err(super::super::trait_selection::trait_application_ambiguity());
                    };
                    let mut fitting = None;
                    for (app, item) in &same {
                        plan.budget.charge_work(1)?;
                        let schema = plan.raw_schema(*item)?;
                        let owner = plan.param_env(app.declaration)?;
                        let mut bindings = HashMap::new();
                        for (index, actual) in std::iter::once(&app.self_ty)
                            .chain(app.args.iter())
                            .enumerate()
                        {
                            bindings.insert(
                                ir::ParamId {
                                    owner: owner.binder(),
                                    index: index as u16,
                                },
                                actual.clone(),
                            );
                        }
                        let result = super::trait_method_hint::bind_owner(
                            &schema.signature.result,
                            &bindings,
                        );
                        let matches = &result == expected
                            || self.nominal_input_types_equal(&result, expected)?;
                        if matches && fitting.replace(*app).is_some() {
                            return Err(
                                super::super::trait_selection::trait_application_ambiguity(),
                            );
                        }
                    }
                    let Some(app) = fitting else {
                        return Err(super::super::trait_selection::trait_application_ambiguity());
                    };
                    selected = Some(app);
                }
                let Some(application) = selected else {
                    return Ok(MethodArguments::Missing);
                };
                let pinned = !same.is_empty();
                Ok(
                    match self.inferred_trait_method_actuals(
                        application,
                        name,
                        explicit,
                        arguments,
                        expected,
                        true,
                    )? {
                        Some(actuals) if pinned => {
                            MethodArguments::Applied(application.clone(), Cow::Owned(actuals))
                        }
                        Some(actuals) => MethodArguments::Ready(Cow::Owned(actuals)),
                        None => {
                            deferred = true;
                            MethodArguments::Deferred
                        }
                    },
                )
            },
            |target, signature| {
                if self.nominal_inputs.is_none() {
                    return Ok(None);
                }
                match self.nominal_call_signature(target, signature.clone())? {
                    nominal_calls::NominalCallSignature::Ready { signature, .. } => {
                        Ok(Some(signature))
                    }
                    nominal_calls::NominalCallSignature::Blocked(blocked) => {
                        *self.nominal_blocked.borrow_mut() = Some(blocked);
                        Err("missing graph input: canonical method signature realization".into())
                    }
                }
            },
        )?;
        Ok((selection, deferred))
    }
}
