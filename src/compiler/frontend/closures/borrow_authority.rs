//! Borrowed environment authority is tied to exact parent capture occurrences.
use super::*;

#[test]
fn closure_receipt_mode_transition_requires_original_provider_facts() {
    let program = parse("fn main(){let f=|x:i32|x+1;}fn other(){}").unwrap();
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
    let closure = bodies
        .iter()
        .find(|body| body.ty.provider() == provider)
        .unwrap();
    let world = plan.trait_world().unwrap();
    let original = world.callable_facts(provider).unwrap();
    let shared = world
        .closure_callable_facts_for(closure.ty.clone(), ir::ClosureKind::Fn)
        .unwrap();
    let mutable = world
        .closure_callable_facts_for(closure.ty.clone(), ir::ClosureKind::FnMut)
        .unwrap();
    shared
        .validate_parent_local_closure_source(&original)
        .unwrap();
    mutable
        .validate_parent_local_closure_source(&original)
        .unwrap();
    assert!(
        original
            .validate_parent_local_closure_source(&original)
            .is_err()
    );
    assert!(
        mutable
            .validate_parent_local_closure_source(&shared)
            .is_err()
    );
    assert!(
        shared
            .validate_parent_local_closure_source(&shared)
            .is_err()
    );
    let parent = world.callable_facts(owner).unwrap();
    assert!(
        shared
            .validate_parent_local_closure_source(&parent)
            .is_err()
    );
    let other_plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let foreign_provider = (0..other_plan.callable_count())
        .filter_map(|index| other_plan.callable_decl(index).ok())
        .find(|declaration| {
            other_plan
                .stored_closure_owner(*declaration)
                .unwrap()
                .is_some()
        })
        .unwrap();
    let foreign = other_plan
        .trait_world()
        .unwrap()
        .callable_facts(foreign_provider)
        .unwrap();
    assert!(
        shared
            .validate_parent_local_closure_source(&foreign)
            .is_err()
    );
}

#[test]
fn borrowed_capture_fields_and_capabilities_reject_forgery() {
    let program =
        parse("struct P{a:i32,b:i32}fn main(){let mut p=P{a:7,b:1};let mut f=||{p.b+=1;p.a+p.b};}")
            .unwrap();
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
    assert_eq!(original.ty.kind(), ir::ClosureKind::FnMut);
    assert_eq!(original.ty.sources().len(), 2);
    let target = |ty: Arc<ir::ClosureTy>| ir::CallTarget {
        declaration: provider,
        type_args: Arc::from([
            ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent: Arc::new(ir::Ty::Closure(ty.clone())),
            },
            ir::Ty::Unit,
            ty.result().clone(),
        ]),
    };
    plan.validate_closure_instance(original, &target(original.ty.clone()))
        .unwrap();
    for change in 0..4 {
        let mut sources = original.ty.sources().to_vec();
        let mut captures = original.ty.captures().to_vec();
        let mut kind = original.ty.kind();
        match change {
            0 => sources[0].place.projection = sources[1].place.projection.clone(),
            1 => sources[0].place.local = ir::LocalId(usize::MAX),
            2 => {
                sources[0].kind = ir::CaptureKind::Value;
                captures[0] = ir::Ty::Int(ir::IntTy::I32);
            }
            3 => kind = ir::ClosureKind::Fn,
            _ => unreachable!(),
        }
        let forged = Arc::new(
            ir::ClosureTy::new(
                owner,
                Arc::from([]),
                original.ty.site(),
                provider,
                original.ty.params().into(),
                original.ty.result().clone(),
                captures.into(),
                kind,
            )
            .with_sources(sources.into()),
        );
        assert!(
            plan.validate_closure_instance(original, &target(forged))
                .is_err(),
            "forgery {change}"
        );
    }
}
