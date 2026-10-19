use super::*;

#[test]
fn closure_lowering_requires_exact_parent_body_and_mode() {
    let program = parse("fn main(){let n=7i32;let f=move ||n;}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (provider, owner) = (0..plan.callable_count())
        .find_map(|index| {
            let provider = plan.callable_decl(index).ok()?;
            plan.stored_closure_owner(provider)
                .ok()
                .flatten()
                .map(|(owner, _)| (provider, owner))
        })
        .unwrap();
    let (_, bodies) =
        type_check::type_callable_source_and_closures(&program, owner, &plan, &budget, None)
            .unwrap();
    let original = bodies
        .iter()
        .find(|body| body.ty.provider() == provider)
        .unwrap();
    let world = plan.trait_world().unwrap();
    let facts = world
        .closure_callable_facts_for(original.ty.clone(), ir::ClosureKind::Fn)
        .unwrap();
    let source = original.body_for(ir::ClosureKind::Fn).unwrap();
    let lowered = crate::compiler::cfg::lower_closure_template(
        source.clone(),
        facts.clone(),
        original,
        ir::ClosureKind::Fn,
        &budget,
    )
    .unwrap();
    // Equal payloads do not manufacture the canonical parent's allocation.
    assert!(
        crate::compiler::cfg::lower_closure_template(
            Arc::new(source.as_ref().clone()),
            facts.clone(),
            original,
            ir::ClosureKind::Fn,
            &budget
        )
        .is_err()
    );
    assert!(
        crate::compiler::cfg::lower_closure_template(
            source.clone(),
            facts.clone(),
            original,
            ir::ClosureKind::FnOnce,
            &budget
        )
        .is_err()
    );
    let mut foreign = original.clone();
    foreign.ty = Arc::new(
        ir::ClosureTy::new(
            owner,
            Arc::from([]),
            original.ty.site() + 1,
            provider,
            original.ty.params().into(),
            original.ty.result().clone(),
            original.ty.captures().into(),
            original.ty.kind(),
        )
        .with_sources(original.ty.sources().into()),
    );
    assert!(
        crate::compiler::cfg::lower_closure_template(
            source,
            facts.clone(),
            &foreign,
            ir::ClosureKind::Fn,
            &budget
        )
        .is_err()
    );
    let mut changed = lowered.function().as_ref().clone();
    changed.name.push_str(" forged");
    assert!(changed.validate_provenance(&facts, &budget).is_err());
    let once = world
        .closure_callable_facts_for(original.ty.clone(), ir::ClosureKind::FnOnce)
        .unwrap();
    assert!(
        lowered
            .function()
            .validate_provenance(&once, &budget)
            .is_err()
    );
}

#[test]
fn capture_sources_and_receiver_modes_are_parent_authorized() {
    let program = parse("struct Item{i:i32}fn main(){let x=Item{i:7};let f=move ||x;}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (provider, owner) = (0..plan.callable_count())
        .find_map(|index| {
            let provider = plan.callable_decl(index).ok()?;
            plan.stored_closure_owner(provider)
                .ok()
                .flatten()
                .map(|(owner, _)| (provider, owner))
        })
        .unwrap();
    let (_, bodies) =
        type_check::type_callable_source_and_closures(&program, owner, &plan, &budget, None)
            .unwrap();
    let original = bodies
        .iter()
        .find(|body| body.ty.provider() == provider)
        .unwrap();
    assert_eq!(original.ty.kind(), ir::ClosureKind::FnOnce);
    assert!(original.body_for(ir::ClosureKind::Fn).is_err());
    assert!(original.body_for(ir::ClosureKind::FnMut).is_err());
    let target = |ty: Arc<ir::ClosureTy>| ir::CallTarget {
        declaration: provider,
        type_args: Arc::from([
            ir::Ty::Closure(ty.clone()),
            ir::Ty::Unit,
            ty.result().clone(),
        ]),
    };
    plan.validate_closure_instance(original, &target(original.ty.clone()))
        .unwrap();
    let mut sources = original.ty.sources().to_vec();
    sources[0].place.local = ir::LocalId(usize::MAX);
    let forged = ir::ClosureTy::new(
        owner,
        Arc::from([]),
        original.ty.site(),
        provider,
        original.ty.params().into(),
        original.ty.result().clone(),
        original.ty.captures().into(),
        original.ty.kind(),
    )
    .with_sources(sources.into());
    assert!(
        plan.validate_closure_instance(original, &target(Arc::new(forged)))
            .is_err()
    );
    let mut shared = target(original.ty.clone());
    shared.type_args = Arc::from([
        ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Closure(original.ty.clone())),
        },
        ir::Ty::Unit,
        original.ty.result().clone(),
    ]);
    assert!(plan.validate_closure_instance(original, &shared).is_err());
}

#[test]
fn parent_body_closure_metadata_rejects_forged_layout_signature_and_site() {
    let program = parse("fn main(){let f=|x:i32|x+1;}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (provider, owner) = (0..plan.callable_count())
        .find_map(|index| {
            let provider = plan.callable_decl(index).ok()?;
            plan.stored_closure_owner(provider)
                .ok()
                .flatten()
                .map(|(owner, _)| (provider, owner))
        })
        .unwrap();
    let (_, bodies) =
        type_check::type_callable_source_and_closures(&program, owner, &plan, &budget, None)
            .unwrap();
    let original = bodies
        .iter()
        .find(|body| body.ty.provider() == provider)
        .unwrap();
    let target = |ty: Arc<ir::ClosureTy>| ir::CallTarget {
        declaration: provider,
        type_args: Arc::from([
            ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(ir::Ty::Closure(ty.clone())),
            },
            ir::Ty::Tuple(ty.params().into()),
            ty.result().clone(),
        ]),
    };
    plan.validate_closure_instance(original, &target(original.ty.clone()))
        .unwrap();
    for forged in [
        ir::ClosureTy::new(
            owner,
            Arc::from([]),
            original.ty.site() + 1,
            provider,
            original.ty.params().into(),
            original.ty.result().clone(),
            Arc::from([]),
            ir::ClosureKind::Fn,
        ),
        ir::ClosureTy::new(
            owner,
            Arc::from([]),
            original.ty.site(),
            provider,
            Arc::from([ir::Ty::Bool]),
            original.ty.result().clone(),
            Arc::from([]),
            ir::ClosureKind::Fn,
        ),
        ir::ClosureTy::new(
            owner,
            Arc::from([]),
            original.ty.site(),
            provider,
            original.ty.params().into(),
            ir::Ty::Bool,
            Arc::from([]),
            ir::ClosureKind::Fn,
        ),
        ir::ClosureTy::new(
            owner,
            Arc::from([]),
            original.ty.site(),
            provider,
            original.ty.params().into(),
            original.ty.result().clone(),
            Arc::from([ir::Ty::Bool]),
            ir::ClosureKind::Fn,
        ),
        ir::ClosureTy::new(
            owner,
            Arc::from([ir::Ty::Bool]),
            original.ty.site(),
            provider,
            original.ty.params().into(),
            original.ty.result().clone(),
            Arc::from([]),
            ir::ClosureKind::Fn,
        ),
        ir::ClosureTy::new(
            owner,
            Arc::from([]),
            original.ty.site(),
            provider,
            original.ty.params().into(),
            original.ty.result().clone(),
            Arc::from([]),
            ir::ClosureKind::FnOnce,
        ),
    ] {
        assert!(
            plan.validate_closure_instance(original, &target(Arc::new(forged)))
                .is_err()
        );
    }
    let mut wrong_result = target(original.ty.clone());
    wrong_result.type_args = Arc::from([
        wrong_result.type_args[0].clone(),
        wrong_result.type_args[1].clone(),
        ir::Ty::Bool,
    ]);
    assert!(
        plan.validate_closure_instance(original, &wrong_result)
            .is_err()
    );
}
