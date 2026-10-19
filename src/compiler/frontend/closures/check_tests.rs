use super::*;

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
