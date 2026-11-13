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

    pub(super) fn method_result_for_hint(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        explicit: &[Type],
        arguments: &[Expr],
    ) -> Result<Option<ir::Ty>, String> {
        let (selection, deferred, partial_result) = self.method_selection_with_inference_mode(
            receiver, name, explicit, arguments, None, true,
        )?;
        if selection.is_some() || deferred {
            return Ok(selection
                .map(|selected| selected.signature.result)
                .or(partial_result));
        }
        for carrier in [
            array_unsize_carrier(receiver),
            array_unsize_mut_carrier(receiver),
        ]
        .into_iter()
        .flatten()
        {
            let (selection, deferred, partial_result) = self.method_selection_with_inference_mode(
                &carrier, name, explicit, arguments, None, true,
            )?;
            if selection.is_some() || deferred {
                return Ok(selection
                    .map(|selected| selected.signature.result)
                    .or(partial_result));
            }
        }
        // Mirror method_through_deref, but read only its exact Target. An
        // unresolved earlier receiver probe must never fall through here.
        let mut owner = match receiver {
            ir::Ty::Ref { referent, .. } => referent.as_ref(),
            ty => ty,
        };
        let mut depth = 0;
        while let ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent,
        } = owner
            && matches!(
                receiver,
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    ..
                }
            )
        {
            self.infer_work(depth)?;
            depth += 1;
            owner = referent.as_ref();
        }
        let Some(target) = self.deref_target_hint(owner)? else {
            return Ok(None);
        };
        self.method_selection_with_inference_mode(&target, name, explicit, arguments, None, true)
            .map(|(selection, _, partial_result)| {
                selection
                    .map(|selected| selected.signature.result)
                    .or(partial_result)
            })
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
        .map(|(selection, deferred, _)| (selection, deferred))
    }

    fn method_selection_with_inference_mode(
        &self,
        receiver: &ir::Ty,
        name: &AstIdent,
        explicit: &[Type],
        arguments: &[Expr],
        expected: Option<&ir::Ty>,
        hint_only: bool,
    ) -> Result<(Option<MethodSelection>, bool, Option<ir::Ty>), String> {
        let plan = self.signatures.plan()?;
        let realized_receiver = if self.nominal_inputs.is_some() {
            Some(self.realize_constructor_actual(receiver, 0)?)
        } else {
            None
        };
        let receiver = realized_receiver.as_ref().unwrap_or(receiver);
        let receiver_nominals = match self.facts() {
            ir::FactsCtx::Validated(facts) => facts
                .parent_local_receiver_inputs(receiver)
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?,
            _ => Vec::new(),
        };
        let has_const = explicit
            .iter()
            .any(|actual| matches!(actual, Type::ConstArgument(_)));
        let partial_const = has_const && explicit.iter().any(partial_annotation::has_nested_infer);
        let written = if !has_const
            && !explicit.is_empty()
            && !explicit.iter().any(partial_annotation::has_nested_infer)
        {
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
        let mut partial_result = None;
        let mut unproved_expected = false;
        // Graph-input checking resolves an inherent target against the exact
        // typed reads of its own source signature, exactly as path calls do.
        let prepare = |target: &ir::CallTarget| self.associated_signature_nominal_inputs(target);
        let prepared = self.nominal_inputs.is_some();
        let selection = plan.select_method_with_arguments_scratch(
            receiver,
            name,
            self.facts(),
            (&self.module, &plan.budget),
            &self.trait_scratch,
            |owner, mode, traits_only| {
                // Const arguments need the selected candidate's slot kinds.
                if has_const
                    && !partial_const
                    && !traits_only
                    && let Some(id) = self.explicit_method_declaration(owner, name, Some(mode))?
                {
                    let actuals = self.explicit_method_actuals(Some(id), explicit)?;
                    return Ok(if prepared {
                        MethodArguments::ReadyPrepared(Cow::Owned(actuals), &prepare)
                    } else {
                        MethodArguments::Ready(Cow::Owned(actuals))
                    });
                }
                if let Some(written) = &written {
                    // Anonymous `impl Trait` parameters cannot be written in a
                    // turbofish. Their actuals come from inference, with the
                    // written ones pinned to the leading explicit slots.
                    let anonymous = !traits_only
                        && match self.inherent_method_candidate(owner, name, Some(mode))? {
                            super::receiver_result_hint::InherentMethodCandidate::Unique(id, _) => {
                                plan.registry
                                    .source_callable(id)
                                    .is_some_and(|f| f.impl_params != 0)
                            }
                            _ => false,
                        };
                    if !anonymous {
                        return Ok(MethodArguments::Ready(Cow::Borrowed(written)));
                    }
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
                                if hint_only {
                                    partial_result = self.receiver_bound_method_result(
                                        owner,
                                        name,
                                        arguments.len(),
                                        mode,
                                    )?;
                                }
                                MethodArguments::Deferred
                            }
                            InherentMethodInference::Applicable(Some(actuals)) if prepared => {
                                MethodArguments::ReadyPrepared(Cow::Owned(actuals), &prepare)
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
                let mut open = Vec::new();
                let mut open_inferred = false;
                let (mut candidates, unproved) = plan.method_trait_candidates_open(
                    owner,
                    name,
                    self.facts(),
                    &self.module,
                    &plan.budget,
                    expected.map(|result| (result, mode)),
                    Some(&mut open),
                )?;
                unproved_expected |= unproved;
                // A trait argument the receiver leaves free (`U` in
                // `impl<T, U> Map<T, U> for [T; N]`) is completed from this
                // call's own evidence, then proved below like any candidate.
                for (trait_id, item) in open {
                    plan.budget.charge_work(1)?;
                    if receiver_mode(plan.declaration(item)?.receiver) != receiver_mode(mode) {
                        continue;
                    }
                    let Some((template, free)) =
                        plan.open_trait_application(owner, trait_id, facts, &plan.budget)?
                    else {
                        // No impl head of this trait matches the receiver.
                        continue;
                    };
                    match self.infer_open_trait_application(
                        &template, &free, item, explicit, arguments, expected,
                    )? {
                        Some(application) => candidates
                            .push((application, item))
                            .map(|()| open_inferred = true)
                            .map_err(|error| {
                                super::super::super::source::SourceError::from_token(error)
                                    .to_string()
                            })?,
                        None if hint_only => {
                            deferred = true;
                            return Ok(MethodArguments::Deferred);
                        }
                        None => {
                            return Err(
                                super::super::trait_selection::trait_application_ambiguity(),
                            );
                        }
                    }
                }
                let mut selected = None;
                let mut argument_pinned = false;
                let mut same: Vec<(&ir::TraitRef, ir::DeclId)> = Vec::new();
                for (application, item) in candidates.as_slice() {
                    plan.budget.charge_work(1)?;
                    if receiver_mode(plan.declaration(*item)?.receiver) != receiver_mode(mode) {
                        continue;
                    }
                    match facts
                        .service()
                        .prove_goal_with_nominals(
                            &ir::Predicate::Implements(application.clone()),
                            facts,
                            &receiver_nominals,
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
                if !same.is_empty() && expected.is_none() {
                    // Several applications of one trait (`Take<i32>`,
                    // `Take<bool>`) and no expected result: the arguments
                    // pick the unique application whose bound formals accept
                    // their evidence by plain equation, as stock unifies the
                    // open trait argument. Any undecided candidate keeps the
                    // ambiguity.
                    let mut fitting = None;
                    let mut undecided = false;
                    for (app, item) in &same {
                        plan.budget.charge_work(1)?;
                        match self.trait_application_fits_arguments(app, *item, arguments)? {
                            Some(true) => {
                                if fitting.replace(*app).is_some() {
                                    undecided = true;
                                }
                            }
                            Some(false) => {}
                            None => undecided = true,
                        }
                    }
                    if !undecided && let Some(app) = fitting {
                        same.clear();
                        argument_pinned = true;
                        selected = Some(app);
                    }
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
                            &plan.budget,
                        )?;
                        let matches = &result == expected
                            || self.nominal_input_types_equal(&result, expected)?;
                        if matches && fitting.replace(*app).is_some() {
                            return Err(
                                super::super::trait_selection::trait_application_ambiguity(),
                            );
                        }
                    }
                    let Some(app) = fitting else {
                        if let Some((application, item)) = same.first() {
                            plan.reject_unproved_expected_trait_result(
                                application,
                                *item,
                                expected,
                                facts,
                                &plan.budget,
                                &mut self.trait_scratch.borrow_mut(),
                            )?;
                        }
                        return Err(super::super::trait_selection::trait_application_ambiguity());
                    };
                    selected = Some(app);
                }
                let Some(application) = selected else {
                    return Ok(MethodArguments::Missing);
                };
                // Expected-result discovery can prove one exact application
                // without enumerating an open blanket-implementation frontier.
                // Preserve that application through final method selection too.
                // Applications completed from the call's own arguments are
                // not rediscovered by final selection's open frontier.
                let pinned =
                    !same.is_empty() || expected.is_some() || argument_pinned || open_inferred;
                Ok(
                    match self.inferred_trait_method_actuals(
                        application,
                        name,
                        explicit,
                        arguments,
                        expected,
                        true,
                    )? {
                        Some(actuals) => {
                            let item = plan
                                .trait_item(application.declaration, name, &plan.budget)?
                                .ok_or("inferred trait method lost original item")?;
                            let call = ir::Callee::TraitItem(ir::TraitCall {
                                trait_ref: application.clone(),
                                item,
                                method_args: Arc::from(actuals.as_slice()),
                            });
                            let nominals = self.parent_local_invocation_inputs(&call)?;
                            if !nominals.is_empty() {
                                MethodArguments::Prepared {
                                    application: pinned.then(|| application.clone()),
                                    actuals: Cow::Owned(actuals),
                                    nominals,
                                }
                            } else if pinned {
                                MethodArguments::Applied(application.clone(), Cow::Owned(actuals))
                            } else {
                                MethodArguments::Ready(Cow::Owned(actuals))
                            }
                        }
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
        if selection.is_none() && !deferred && unproved_expected {
            return Err(
                "E0277: expected method result requires an unsatisfied trait application".into(),
            );
        }
        Ok((selection, deferred, partial_result))
    }
}
