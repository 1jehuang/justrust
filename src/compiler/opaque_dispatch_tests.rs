use super::*;

/// This is a real compiler-provider dependency test, not a replacement solver.
#[test]
fn opaque_bound_dispatch_reads_exact_representation_before_physical_body() {
    struct Probe(CompilerProvider);
    impl Provider for Probe {
        fn outputs(&self, key: &Key) -> usize {
            self.0.outputs(key)
        }
        fn start(&self, cx: Context, requested: Key) -> TaskFuture {
            if requested.kind != 9001 {
                return self.0.start(cx, requested);
            }
            let provider = self.0.clone();
            Box::pin(async move {
                let facts = Facts { cx: &cx };
                let types = facts.types().await?;
                // make and use_value are the two original free declarations.
                let source_index = 1;
                let template: Arc<TemplateBodyFact> =
                    facts.get(Facts::item(TEMPLATE_BODY, source_index)).await?;
                let caller = ir::CallTarget {
                    declaration: types.callable_decl(source_index).map_err(Error::new)?,
                    type_args: Arc::from([]),
                };
                let source = provider.validated_body(&types, source_index, &template)?;
                let (occurrence, call) = source
                    .exprs
                    .iter()
                    .enumerate()
                    .find_map(|(i, expression)| match &expression.kind {
                        ir::ExprKind::Call {
                            target: ir::Callee::TraitItem(call),
                            ..
                        } if matches!(call.trait_ref.self_ty, ir::Ty::Opaque(_)) => {
                            Some((ir::ExprId(i), call.clone()))
                        }
                        _ => None,
                    })
                    .ok_or_else(|| Error::new("fixture has no original opaque bound call"))?;
                let key = opaque_dispatch::key(&caller, occurrence, &call, &provider.0.budget)?;
                let request = cx.request(key);
                let authorization_handle = request.get(0);
                let ready_handle = request.get(1);
                let authorization: Arc<opaque_dispatch::OpaqueCallAuthorization> =
                    cx.read_fact(authorization_handle).await?;
                let ready: Arc<opaque_dispatch::PhysicalCalleeReady> =
                    cx.read_fact(ready_handle).await?;
                let representations = opaque_instances::read(
                    &facts,
                    &[call.trait_ref.self_ty.clone()],
                    &provider.0.budget,
                )
                .await?;
                authorization.replay(
                    template.publication.as_ref().unwrap(),
                    &caller,
                    occurrence,
                    &representations,
                    &types,
                    &provider.0.budget,
                )?;
                if ready.target() != authorization.target() {
                    return Err(Error::new(
                        "ready physical body differs from sealed dispatch target",
                    ));
                }
                let publication = template.publication.as_ref().unwrap();
                let lowered = cfg::lower_source_template(publication, &provider.0.budget)
                    .map_err(|e| Error::new(e.to_string()))?;
                let closed = types.instance_facts(&caller).map_err(Error::new)?;
                let mut physical = opaque_physical::normalize(
                    publication,
                    &caller,
                    lowered.function(),
                    &representations,
                    &[authorization.clone()],
                    &types,
                    &provider.0.budget,
                )?;
                physical
                    .seal(
                        publication,
                        lowered.function(),
                        &representations,
                        &types,
                        &closed,
                        &provider.0.budget,
                    )
                    .map_err(Error::new)?;
                let validation = physical.validation(
                    publication,
                    lowered.function(),
                    &representations,
                    &types,
                    &provider.0.budget,
                );
                physical
                    .function()
                    .validate_physical(&closed, &validation)
                    .map_err(Error::new)?;
                // The same typed body republished by the canonical factory has
                // a different authorization seal, even with the same owner.
                let frontend::PublishedCallableBody::Source(republished) = types
                    .publish_callable_body(caller.declaration, &provider.0.budget)
                    .map_err(|error| source_fact_error(error, &provider.0.budget))?
                else {
                    return Err(Error::new("fixture expected original source publication"));
                };
                assert_ne!(publication.identity(), republished.identity());
                let foreign_publication = physical.validation(
                    &republished,
                    lowered.function(),
                    &representations,
                    &types,
                    &provider.0.budget,
                );
                assert!(
                    foreign_publication
                        .validate_cfg(physical.function(), &closed)
                        .is_err()
                );
                assert!(
                    authorization
                        .replay(
                            &republished,
                            &caller,
                            occurrence,
                            &representations,
                            &types,
                            &provider.0.budget,
                        )
                        .is_err()
                );
                assert!(
                    authorization
                        .replay(
                            publication,
                            &caller,
                            ir::ExprId(usize::MAX),
                            &representations,
                            &types,
                            &provider.0.budget,
                        )
                        .is_err()
                );
                let foreign_program = frontend::parse("fn foreign() -> i32 { 1 } fn main() {}")
                    .map_err(Error::new)?;
                let foreign_types =
                    frontend::PlanCtx::from_program(&foreign_program, &provider.0.budget)
                        .map_err(Error::new)?;
                let foreign_target = ir::CallTarget {
                    declaration: foreign_types.callable_decl(0).map_err(Error::new)?,
                    type_args: Arc::from([]),
                };
                let foreign_facts = foreign_types
                    .instance_facts(&foreign_target)
                    .map_err(Error::new)?;
                assert!(
                    validation
                        .validate_cfg(physical.function(), &foreign_facts)
                        .is_err()
                );
                let mut changed_type = physical.function().clone();
                changed_type.result = ir::Ty::Bool;
                assert!(validation.validate_cfg(&changed_type, &closed).is_err());
                let mut direct_abi_controls = 0;
                for block in &physical.function().blocks {
                    if let cfg::Terminator::Call {
                        target: callee,
                        signature,
                        ..
                    } = &block.terminator.value
                    {
                        assert!(validation.validate_call(None, callee, signature).is_err());
                        let mut changed_signature = signature.clone();
                        changed_signature.result = ir::Ty::Bool;
                        assert!(
                            validation
                                .validate_call(block.terminator.origin, callee, &changed_signature)
                                .is_err()
                        );
                        let mut wrong_target = caller.clone();
                        wrong_target.declaration = types.callable_decl(0).map_err(Error::new)?;
                        if callee != &ir::Callee::Direct(wrong_target.clone()) {
                            assert!(
                                validation
                                    .validate_call(
                                        block.terminator.origin,
                                        &ir::Callee::Direct(wrong_target),
                                        signature,
                                    )
                                    .is_err()
                            );
                        }
                        if let Some(site) = block.terminator.origin
                            && matches!(
                                publication.source().exprs[site.0].kind,
                                ir::ExprKind::Call {
                                    target: ir::Callee::Direct(_),
                                    ..
                                }
                            )
                        {
                            direct_abi_controls += 1;
                        }
                    }
                }
                assert!(
                    direct_abi_controls > 0,
                    "fixture must replay original direct-call ABI"
                );
                let body_boundary = validation
                    .body_carriers(physical.function(), &closed)
                    .map_err(Error::new)?;
                body_boundary.validate().map_err(Error::new)?;
                if !body_boundary.contract().inputs.is_empty()
                    || !body_boundary.contract().results.is_empty()
                {
                    return Err(Error::new(
                        "physical caller boundary unexpectedly carries references",
                    ));
                }
                for (index, block) in physical.function().blocks.iter().enumerate() {
                    if matches!(block.terminator.value, cfg::Terminator::Call { .. }) {
                        let boundary = validation
                            .call_carriers(physical.function(), &closed, cfg::BlockId(index))
                            .map_err(Error::new)?;
                        boundary.validate().map_err(Error::new)?;
                        validation
                            .replay_call_arguments(
                                physical.function(),
                                &closed,
                                cfg::BlockId(index),
                            )
                            .map_err(Error::new)?;
                    }
                }
                let physical_concrete = ownership::analyze_physical(
                    Arc::new(physical.function().clone()),
                    closed.clone(),
                    &validation,
                )
                .map_err(|analysis| {
                    Error::new(format!("physical caller ownership failed: {analysis:?}"))
                })?;
                let physical_object =
                    backend::emit_physical_cfg_function(&physical_concrete, &|target| {
                        Ok(format!(
                            "physical_{}_{}",
                            target.declaration.snapshot.graph_word(),
                            target.declaration.index
                        ))
                    })
                    .map_err(Error::new)?;
                if physical_object.code.is_empty() {
                    return Err(Error::new("physical caller emitted no machine code"));
                }
                if physical
                    .function()
                    .validate_with_facts(ir::FactsCtx::from_validated(&closed))
                    .is_ok()
                {
                    return Err(Error::new(
                        "physical CFG was accepted without borrowed normalization authority",
                    ));
                }
                let defining: Arc<TemplateBodyFact> =
                    facts.get(Facts::item(TEMPLATE_BODY, 0)).await?;
                let defining_publication = defining.publication.as_ref().unwrap();
                let defining_target = ir::CallTarget {
                    declaration: types.callable_decl(0).map_err(Error::new)?,
                    type_args: Arc::from([]),
                };
                let defining_lowered =
                    cfg::lower_source_template(defining_publication, &provider.0.budget)
                        .map_err(|e| Error::new(e.to_string()))?;
                let defining_closed = types.instance_facts(&defining_target).map_err(Error::new)?;
                let mut defining_physical = opaque_physical::normalize(
                    defining_publication,
                    &defining_target,
                    defining_lowered.function(),
                    &representations,
                    &[],
                    &types,
                    &provider.0.budget,
                )?;
                defining_physical
                    .seal(
                        defining_publication,
                        defining_lowered.function(),
                        &representations,
                        &types,
                        &defining_closed,
                        &provider.0.budget,
                    )
                    .map_err(Error::new)?;
                let defining_validation = defining_physical.validation(
                    defining_publication,
                    defining_lowered.function(),
                    &representations,
                    &types,
                    &provider.0.budget,
                );
                defining_physical
                    .function()
                    .validate_physical(&defining_closed, &defining_validation)
                    .map_err(Error::new)?;
                let defining_boundary = defining_validation
                    .body_carriers(defining_physical.function(), &defining_closed)
                    .map_err(Error::new)?;
                defining_boundary.validate().map_err(Error::new)?;
                if defining_boundary.original().contract().results.is_empty()
                    || !defining_boundary.contract().results.is_empty()
                {
                    return Err(Error::new(
                        "physical defining boundary did not preserve abstract-to-concrete carrier distinction",
                    ));
                }
                let defining_concrete = ownership::analyze_physical(
                    Arc::new(defining_physical.function().clone()),
                    defining_closed.clone(),
                    &defining_validation,
                )
                .map_err(|analysis| {
                    Error::new(format!("physical defining ownership failed: {analysis:?}"))
                })?;
                let defining_object =
                    backend::emit_physical_cfg_function(&defining_concrete, &|_| {
                        Err("defining opaque fixture has no physical calls".into())
                    })
                    .map_err(Error::new)?;
                if defining_object.code.is_empty() {
                    return Err(Error::new("physical defining body emitted no machine code"));
                }
                cx.publish_fact(0, ())
            })
        }
    }
    for workers in [1, 8] {
        let snapshot = Arc::new(Snapshot {
            input: Input::Text("trait Read { fn read(&self)->i32; } struct Number { value:i32 } impl Read for Number { fn read(&self)->i32 { self.value } } fn make()->impl Read { Number {value:42} } fn use_value()->i32 { let value=make(); value.read() } fn main(){assert!(use_value()==42);}".into()),
            source_options: source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let engine = Engine::new(Probe(CompilerProvider(snapshot)));
        let probe = engine.request(Key::new(9001, vec![]));
        engine.run(workers).unwrap();
        probe.completion().unwrap();
    }
}
