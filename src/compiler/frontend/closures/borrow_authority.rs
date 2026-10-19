//! Borrowed environment authority is tied to exact parent capture occurrences.
use super::*;

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
