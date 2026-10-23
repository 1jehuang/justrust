use super::*;

#[test]
fn opaque_dispatch_v4_argument_identity_and_malformed_keys() {
    struct Probe(CompilerProvider);
    impl Provider for Probe {
        fn outputs(&self, key: &Key) -> usize {
            self.0.outputs(key)
        }

        fn start(&self, cx: Context, requested: Key) -> TaskFuture {
            if requested.kind != 9002 {
                return self.0.start(cx, requested);
            }
            let provider = self.0.clone();
            Box::pin(async move {
                let facts = Facts { cx: &cx };
                let types = facts.types().await?;
                let template: Arc<TemplateBodyFact> = facts
                    .get(crate::compiler::keys::item(TEMPLATE_BODY, 1))
                    .await?;
                let caller = ir::CallTarget {
                    declaration: types.callable_decl(1).map_err(Error::new)?,
                    type_args: Arc::from([]),
                };
                let source = provider.validated_body(&types, 1, &template)?;
                let (occurrence, call) = source
                    .exprs
                    .iter()
                    .enumerate()
                    .find_map(|(index, expression)| match &expression.kind {
                        ir::ExprKind::Call {
                            target: ir::Callee::TraitItem(call),
                            ..
                        } if matches!(call.trait_ref.self_ty, ir::Ty::Opaque(_)) => {
                            Some((ir::ExprId(index), call.clone()))
                        }
                        _ => None,
                    })
                    .ok_or_else(|| Error::new("fixture has no opaque bound call"))?;
                let budget = &provider.0.budget;
                let canonical = opaque_dispatch::key(&caller, occurrence, &call, budget)?;
                assert_eq!(canonical.params[0], 4);

                // Keep the caller, occurrence, receiver and declarations identical:
                // only the ordered public trait/method actuals may distinguish these.
                let u8_ty = ir::Ty::Int(ir::IntTy::U8);
                let u16_ty = ir::Ty::Int(ir::IntTy::U16);
                let mut argument_keys = vec![canonical.clone()];
                for (trait_args, method_args) in [
                    (vec![u8_ty.clone()], vec![]),
                    (vec![u16_ty.clone()], vec![]),
                    (vec![], vec![u8_ty.clone()]),
                    (vec![], vec![u16_ty.clone()]),
                    (vec![u8_ty.clone(), u16_ty.clone()], vec![]),
                    (vec![u16_ty.clone(), u8_ty.clone()], vec![]),
                    (vec![], vec![u8_ty.clone(), u16_ty.clone()]),
                    (vec![], vec![u16_ty.clone(), u8_ty.clone()]),
                    (vec![u8_ty.clone()], vec![u16_ty.clone()]),
                    (vec![u16_ty], vec![u8_ty]),
                ] {
                    let mut changed = call.clone();
                    changed.trait_ref.args = trait_args.into();
                    changed.method_args = method_args.into();
                    let key = opaque_dispatch::key(&caller, occurrence, &changed, budget)?;
                    assert!(
                        !argument_keys.contains(&key),
                        "different ordered trait/method arguments shared a dispatch key"
                    );
                    argument_keys.push(key);
                }

                // A borrowed receiver must not alias the bare family or any
                // other ordered shared/mutable wrapper sequence.
                for wrappers in [
                    vec![ir::BorrowKind::Shared],
                    vec![ir::BorrowKind::Mut],
                    vec![ir::BorrowKind::Shared, ir::BorrowKind::Shared],
                    vec![ir::BorrowKind::Shared, ir::BorrowKind::Mut],
                    vec![ir::BorrowKind::Mut, ir::BorrowKind::Shared],
                ] {
                    let mut changed = call.clone();
                    for kind in wrappers.into_iter().rev() {
                        changed.trait_ref.self_ty = ir::Ty::Ref {
                            kind,
                            referent: Arc::new(changed.trait_ref.self_ty),
                        };
                    }
                    let key = opaque_dispatch::key(&caller, occurrence, &changed, budget)?;
                    assert!(
                        !argument_keys.contains(&key),
                        "reference wrapper identity collided"
                    );
                    argument_keys.push(key);
                }
                for receiver in [
                    ir::Ty::RawPtr {
                        kind: ir::BorrowKind::Shared,
                        pointee: Arc::new(call.trait_ref.self_ty.clone()),
                    },
                    ir::Ty::Tuple(Arc::from([call.trait_ref.self_ty.clone()])),
                ] {
                    let mut changed = call.clone();
                    changed.trait_ref.self_ty = receiver;
                    assert!(opaque_dispatch::key(&caller, occurrence, &changed, budget).is_err());
                }

                let caller_end = 2 + canonical.params[1] as usize;
                let opaque_len_index = caller_end + 6;
                let trait_frame =
                    opaque_len_index + 1 + canonical.params[opaque_len_index] as usize;
                let method_frame = trait_frame + 1 + canonical.params[trait_frame] as usize;
                assert_eq!(
                    method_frame + 1 + canonical.params[method_frame] as usize,
                    canonical.params.len(),
                    "v4 must end with exactly the trait and method argument frames"
                );
                let mut rejected = argument_keys.into_iter().skip(1).collect::<Vec<_>>();
                // Every proper prefix, including cuts inside each application frame,
                // must fail rather than recovering an authorization from the prefix.
                for end in 0..canonical.params.len() {
                    let mut key = canonical.clone();
                    key.params.truncate(end);
                    rejected.push(key);
                }
                for index in [1, opaque_len_index, trait_frame, method_frame] {
                    let mut key = canonical.clone();
                    key.params[index] = u64::MAX;
                    rejected.push(key);
                }
                for (index, value) in [
                    (0, 3), // A key without the full receiver frame cannot authorize dispatch.
                    (caller_end, u64::MAX),
                    (caller_end + 1, 1), // Unsupported invocation capability.
                    (trait_frame, 0),
                    (method_frame, 0),
                ] {
                    let mut key = canonical.clone();
                    key.params[index] = value;
                    rejected.push(key);
                }
                let mut trailing = canonical.clone();
                trailing.params.push(0);
                rejected.push(trailing);
                let mut swapped = canonical.clone();
                swapped.params.truncate(trait_frame);
                swapped
                    .params
                    .extend_from_slice(&canonical.params[method_frame..]);
                swapped
                    .params
                    .extend_from_slice(&canonical.params[trait_frame..method_frame]);
                rejected.push(swapped);

                for key in rejected {
                    assert_ne!(key, canonical);
                    let request = cx.request(key.clone());
                    assert!(
                        cx.read_fact::<opaque_dispatch::OpaqueCallAuthorization>(request.get(0))
                            .await
                            .is_err(),
                        "altered key authorized dispatch: {key:?}"
                    );
                    assert!(
                        cx.read_fact::<opaque_dispatch::PhysicalCalleeReady>(request.get(1))
                            .await
                            .is_err(),
                        "altered key published physical readiness: {key:?}"
                    );
                }
                // Failed requests must not poison the independently canonical key.
                let request = cx.request(canonical);
                let authorization: Arc<opaque_dispatch::OpaqueCallAuthorization> =
                    cx.read_fact(request.get(0)).await?;
                let ready: Arc<opaque_dispatch::PhysicalCalleeReady> =
                    cx.read_fact(request.get(1)).await?;
                assert_eq!(authorization.target(), ready.target());
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
        let probe = engine.request(crate::compiler::keys::test_key(9002, vec![]));
        engine.run(workers).unwrap();
        probe.completion().unwrap();
    }
}

#[test]
fn borrowed_opaque_dispatch_replays_exact_source_wrappers() {
    struct Probe(CompilerProvider);
    impl Provider for Probe {
        fn outputs(&self, key: &Key) -> usize {
            self.0.outputs(key)
        }
        fn start(&self, cx: Context, requested: Key) -> TaskFuture {
            if requested.kind != 9003 {
                return self.0.start(cx, requested);
            }
            let provider = self.0.clone();
            Box::pin(async move {
                let facts = Facts { cx: &cx };
                let types = facts.types().await?;
                let main: Arc<TemplateBodyFact> = facts.get(keys::item(TEMPLATE_BODY, 2)).await?;
                let apply = types.callable_decl(1).map_err(Error::new)?;
                let main_source = provider.validated_body(&types, 2, &main)?;
                let caller = main_source
                    .exprs
                    .iter()
                    .find_map(|expr| match &expr.kind {
                        ir::ExprKind::Call {
                            target: ir::Callee::Direct(target),
                            ..
                        } if target.declaration == apply => Some(target.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| Error::new("fixture lost apply instance"))?;
                let template: Arc<TemplateBodyFact> =
                    facts.get(keys::item(TEMPLATE_BODY, 1)).await?;
                let publication = template.publication.as_ref().unwrap();
                let occurrence = publication
                    .source()
                    .exprs
                    .iter()
                    .enumerate()
                    .find_map(|(index, expr)| {
                        matches!(
                            expr.kind,
                            ir::ExprKind::Call {
                                target: ir::Callee::TraitItem(_),
                                ..
                            }
                        )
                        .then_some(ir::ExprId(index))
                    })
                    .ok_or_else(|| Error::new("fixture lost borrowed invocation"))?;
                let budget = &provider.0.budget;
                let mut specialization = opaque_specialization::Specialization::new(
                    publication,
                    &caller,
                    &types,
                    budget,
                    cx.authority(),
                )?;
                let (ir::Callee::TraitItem(call), _) = specialization.call(occurrence)? else {
                    return Err(Error::new("fixture changed invocation kind"));
                };
                assert!(matches!(
                    call.trait_ref.self_ty,
                    ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        ..
                    }
                ));
                let canonical = opaque_dispatch::key(&caller, occurrence, &call, budget)?;
                let request = cx.request(canonical.clone());
                let representations = opaque_instances::read(
                    &facts,
                    std::slice::from_ref(&call.trait_ref.self_ty),
                    budget,
                )
                .await?;
                let authorization: Arc<opaque_dispatch::OpaqueCallAuthorization> =
                    cx.read_fact(request.get(0)).await?;
                let ready: Arc<opaque_dispatch::PhysicalCalleeReady> =
                    cx.read_fact(request.get(1)).await?;
                assert_eq!(authorization.target(), ready.target());
                authorization.replay(
                    publication,
                    &caller,
                    occurrence,
                    &representations,
                    &types,
                    budget,
                )?;
                let ir::Ty::Ref { referent, .. } = &call.trait_ref.self_ty else {
                    unreachable!()
                };
                for receiver in [
                    referent.as_ref().clone(),
                    ir::Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        referent: referent.clone(),
                    },
                    ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: Arc::new(call.trait_ref.self_ty.clone()),
                    },
                ] {
                    let mut changed = call.clone();
                    changed.trait_ref.self_ty = receiver.clone();
                    let forged = opaque_dispatch::key(&caller, occurrence, &changed, budget)?;
                    assert_ne!(forged, canonical);
                    let request = cx.request(forged);
                    assert!(
                        cx.read_fact::<opaque_dispatch::OpaqueCallAuthorization>(request.get(0))
                            .await
                            .is_err()
                    );
                    assert!(
                        cx.read_fact::<opaque_dispatch::PhysicalCalleeReady>(request.get(1))
                            .await
                            .is_err()
                    );
                    let mut changed_caller = caller.clone();
                    changed_caller.type_args = Arc::from([receiver]);
                    assert!(
                        authorization
                            .replay(
                                publication,
                                &changed_caller,
                                occurrence,
                                &representations,
                                &types,
                                budget
                            )
                            .is_err()
                    );
                }
                assert!(
                    authorization
                        .replay(
                            publication,
                            &caller,
                            ir::ExprId(usize::MAX),
                            &representations,
                            &types,
                            budget
                        )
                        .is_err()
                );
                cx.publish_fact(0, ())
            })
        }
    }
    for workers in [1, 8] {
        let snapshot = Arc::new(Snapshot {
            input: Input::Text("fn make_adder(n:i32)->impl Fn(i32)->i32{move |x|x+n} fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{f(x)} fn main(){let a=make_adder(5);assert!(apply(&a,10)==15);}".into()),
            source_options: source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let engine = Engine::new(Probe(CompilerProvider(snapshot)));
        let probe = engine.request(keys::test_key(9003, vec![]));
        engine.run(workers).unwrap();
        probe.completion().unwrap();
    }
}

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
                let template: Arc<TemplateBodyFact> = facts
                    .get(crate::compiler::keys::item(TEMPLATE_BODY, source_index))
                    .await?;
                let caller = ir::CallTarget {
                    declaration: types.callable_decl(source_index).map_err(Error::new)?,
                    type_args: if types
                        .callable_env(types.callable_decl(source_index).map_err(Error::new)?)
                        .map_err(Error::new)?
                        .is_generic()
                    {
                        Arc::from([ir::Ty::Int(ir::IntTy::U8)])
                    } else {
                        Arc::from([])
                    },
                };
                let source = provider.validated_body(&types, source_index, &template)?;
                let occurrence = source
                    .exprs
                    .iter()
                    .enumerate()
                    .find_map(|(i, expression)| match &expression.kind {
                        ir::ExprKind::Call {
                            target: ir::Callee::TraitItem(call),
                            ..
                        } if matches!(call.trait_ref.self_ty, ir::Ty::Opaque(_)) => {
                            Some(ir::ExprId(i))
                        }
                        _ => None,
                    })
                    .ok_or_else(|| Error::new("fixture has no original opaque bound call"))?;
                let publication = template.publication.as_ref().unwrap();
                let mut specialization = opaque_specialization::Specialization::new(
                    publication,
                    &caller,
                    &types,
                    &provider.0.budget,
                    cx.authority(),
                )?;
                let (ir::Callee::TraitItem(specialized_call), _) =
                    specialization.call(occurrence)?
                else {
                    return Err(Error::new(
                        "test lost original trait call after substitution",
                    ));
                };
                let key = opaque_dispatch::key(
                    &caller,
                    occurrence,
                    &specialized_call,
                    &provider.0.budget,
                )?;
                let request = cx.request(key);
                let authorization_handle = request.get(0);
                let ready_handle = request.get(1);
                let authorization: Arc<opaque_dispatch::OpaqueCallAuthorization> =
                    cx.read_fact(authorization_handle).await?;
                let ready: Arc<opaque_dispatch::PhysicalCalleeReady> =
                    cx.read_fact(ready_handle).await?;
                let representations = opaque_instances::read(
                    &facts,
                    std::slice::from_ref(&specialized_call.trait_ref.self_ty),
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
                    std::slice::from_ref(&authorization),
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
                let validation = physical
                    .validation(
                        publication,
                        lowered.function(),
                        &representations,
                        &types,
                        &provider.0.budget,
                    )
                    .map_err(Error::new)?;
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
                assert!(foreign_publication.is_err());
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
                if !caller.type_args.is_empty() {
                    let changed_caller = ir::CallTarget {
                        declaration: caller.declaration,
                        type_args: Arc::from([ir::Ty::Int(ir::IntTy::U16)]),
                    };
                    assert!(
                        authorization
                            .replay(
                                publication,
                                &changed_caller,
                                occurrence,
                                &representations,
                                &types,
                                &provider.0.budget,
                            )
                            .is_err()
                    );
                    assert!(
                        opaque_physical::normalize(
                            publication,
                            &changed_caller,
                            lowered.function(),
                            &representations,
                            std::slice::from_ref(&authorization),
                            &types,
                            &provider.0.budget,
                        )
                        .is_err()
                    );
                }
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
                let mut detached_shape = physical.function().clone();
                detached_shape.lowering_proof = None;
                assert!(validation.validate_cfg(&detached_shape, &closed).is_err());
                let mut copied_shape = physical.function().clone();
                copied_shape.lowering_proof = physical
                    .function()
                    .lowering_proof
                    .as_ref()
                    .map(|shape| Arc::new(shape.as_ref().clone()));
                assert!(validation.validate_cfg(&copied_shape, &closed).is_err());
                assert!(
                    validation
                        .validate_cfg(&physical.function().clone(), &closed)
                        .is_ok()
                );
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
                let defining: Arc<TemplateBodyFact> = facts
                    .get(crate::compiler::keys::item(TEMPLATE_BODY, 0))
                    .await?;
                let defining_publication = defining.publication.as_ref().unwrap();
                let ir::Ty::Opaque(defining_opaque) = &specialized_call.trait_ref.self_ty else {
                    return Err(Error::new("fixture lost its original opaque call identity"));
                };
                let defining_target = ir::CallTarget {
                    declaration: types.callable_decl(0).map_err(Error::new)?,
                    type_args: Arc::from(defining_opaque.owner_args()),
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
                let defining_validation = defining_physical
                    .validation(
                        defining_publication,
                        defining_lowered.function(),
                        &representations,
                        &types,
                        &provider.0.budget,
                    )
                    .map_err(Error::new)?;
                defining_physical
                    .function()
                    .validate_physical(&defining_closed, &defining_validation)
                    .map_err(Error::new)?;
                if defining_target.type_args.len() == 2 {
                    let frontend::PublishedCallableBody::Source(republished_definition) = types
                        .publish_callable_body(defining_target.declaration, &provider.0.budget)
                        .map_err(|error| source_fact_error(error, &provider.0.budget))?
                    else {
                        return Err(Error::new(
                            "generic fixture expected canonical source publication",
                        ));
                    };
                    assert!(
                        opaque_physical::normalize(
                            &republished_definition,
                            &defining_target,
                            defining_lowered.function(),
                            &representations,
                            &[],
                            &types,
                            &provider.0.budget,
                        )
                        .is_err(),
                        "generic defining CFG accepted another publication of the same owner"
                    );
                    let mut reversed = defining_target.clone();
                    let mut args = reversed.type_args.to_vec();
                    args.swap(0, 1);
                    reversed.type_args = args.into();
                    let reversed_facts = types.instance_facts(&reversed).map_err(Error::new)?;
                    assert!(
                        defining_validation
                            .validate_cfg(defining_physical.function(), &reversed_facts)
                            .is_err(),
                        "physical CFG accepted another ordered generic instantiation"
                    );
                    assert!(
                        opaque_physical::normalize(
                            defining_publication,
                            &reversed,
                            defining_lowered.function(),
                            &representations,
                            &[],
                            &types,
                            &provider.0.budget,
                        )
                        .is_err(),
                        "same hidden type authorized a different ordered representation"
                    );
                }
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
        for source in [
            "trait Read { fn read(&self)->i32; } struct Number { value:i32 } impl Read for Number { fn read(&self)->i32 { self.value } } fn make()->impl Read { Number {value:42} } fn use_value()->i32 { let value=make(); value.read() } fn main(){assert!(use_value()==42);}",
            "trait Read { fn read(&self)->i32; } struct Number { value:i32 } impl Read for Number { fn read(&self)->i32 { self.value } } fn make<A:Copy,B:Copy>(a:A,b:B)->impl Read { let copied=a; let other=b; Number {value:42} } fn use_value()->i32 { let value=make::<u8,u16>(1u8,2u16); value.read() } fn main(){assert!(use_value()==42);}",
            "trait Read { fn read(&self)->i32; } struct Number { value:i32 } impl Read for Number { fn read(&self)->i32 { self.value } } fn make<A:Copy,B:Copy>(a:A,b:B)->impl Read { let copied=a; let other=b; Number {value:42} } fn use_value<T:Copy>(input:T)->i32 { let value=make::<T,u16>(input,2u16); value.read() } fn main(){assert!(use_value::<u8>(1u8)==42);}",
        ] {
            let snapshot = Arc::new(Snapshot {
                input: Input::Text(source.into()),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(Probe(CompilerProvider(snapshot)));
            let probe = engine.request(crate::compiler::keys::test_key(9001, vec![]));
            engine.run(workers).unwrap();
            probe.completion().unwrap();
        }
    }
}
