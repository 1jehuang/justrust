use super::*;
use crate::compiler::frontend;

#[test]
fn normal_drop_descriptor_rejects_abi_compatible_noncanonical_target_and_signature() {
    let source = "fn f(){let d=D{v:1};}fn impostor(d:&mut D){}struct D{v:i32}impl Drop for D{fn drop(&mut self){}}fn main(){}";
    let program = frontend::parse(source).unwrap();
    let budget = PlanBudget::default();
    let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let frontend::PublishedCallableBody::Source(publication) = context
        .publish_callable_body(context.function_decl(0).unwrap(), &budget)
        .unwrap()
    else {
        panic!("source expected")
    };
    let facts = publication.facts();
    let owner = publication
        .source()
        .locals
        .iter()
        .find(|l| matches!(l.ty, Ty::Struct(_)))
        .unwrap()
        .ty
        .clone();
    assert!(facts.normal_drop_required(&owner, &budget).unwrap());
    assert!(!facts.normal_drop_required(&Ty::Bool, &budget).unwrap());
    assert!(
        !facts
            .normal_drop_required(
                &Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(owner.clone()),
                },
                &budget,
            )
            .unwrap()
    );
    for len in [0, 1, 3] {
        let array = Ty::Array {
            element: Arc::new(owner.clone()),
            len,
        };
        assert_eq!(
            facts.normal_drop_required(&array, &budget).unwrap(),
            len != 0
        );
        assert_eq!(
            facts.normal_drop_required(&array, &budget).unwrap(),
            matches!(
                facts.normal_drop_plan(&array).unwrap(),
                NormalDropPlan::Destructor(_)
            )
        );
    }
    let NormalDropPlan::Destructor(valid) = facts.normal_drop_plan(&owner).unwrap() else {
        panic!("destructor expected")
    };
    valid.validate(&owner, facts).unwrap();
    let mut forged = valid.as_ref().clone();
    forged.target = Some(CallTarget {
        declaration: context.function_decl(1).unwrap(),
        type_args: Arc::from([]),
    });
    assert!(forged.validate(&owner, facts).is_err());
    let mut forged = valid.as_ref().clone();
    forged
        .signature
        .as_mut()
        .expect("user Drop destructor has a canonical signature")
        .result = Ty::Bool;
    assert!(forged.validate(&owner, facts).is_err());
}

#[test]
fn closure_capture_cleanup_rejects_changed_glue_and_same_layout_owner() {
    let source = "fn f(){let a=D{v:1};let b=D{v:2};let first=move||a;let second=move||b;}struct D{v:i32}impl Drop for D{fn drop(&mut self){}}fn main(){}";
    let program = frontend::parse(source).unwrap();
    let budget = PlanBudget::default();
    let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let frontend::PublishedCallableBody::Source(publication) = context
        .publish_callable_body(context.function_decl(0).unwrap(), &budget)
        .unwrap()
    else {
        panic!("source expected")
    };
    let facts = publication.facts();
    let closures: Vec<_> = publication
        .source()
        .locals
        .iter()
        .filter_map(|local| matches!(local.ty, Ty::Closure(_)).then_some(&local.ty))
        .collect();
    assert_eq!(closures.len(), 2);
    let NormalDropPlan::Destructor(valid) = facts.normal_drop_plan(closures[0]).unwrap() else {
        panic!("captured owner requires destruction")
    };
    valid.validate(closures[0], facts).unwrap();
    assert!(matches!(valid.glue(), DropGlue::Fields(fields) if fields.len() == 1));
    assert!(valid.validate(closures[1], facts).is_err());
    let mut forged = valid.as_ref().clone();
    forged.glue = DropGlue::None;
    assert!(forged.validate(closures[0], facts).is_err());
    let mut forged = valid.as_ref().clone();
    let DropGlue::Fields(fields) = &mut forged.glue else {
        unreachable!()
    };
    fields[0].0 = 1;
    assert!(forged.validate(closures[0], facts).is_err());
}

#[test]
fn normal_drop_unresolved_parameter_is_not_negative_evidence() {
    let program = frontend::parse(
        "fn f<T>(value:T, d:D){}struct D;impl Drop for D{fn drop(&mut self){}}fn main(){}",
    )
    .unwrap();
    let budget = PlanBudget::default();
    let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let frontend::PublishedCallableBody::Source(publication) = context
        .publish_callable_body(context.function_decl(0).unwrap(), &budget)
        .unwrap()
    else {
        panic!("source expected")
    };
    let ty = &publication.source().locals[publication.source().params[0].0].ty;
    let facts = publication.facts();
    assert!(matches!(
        facts.normal_drop_plan(ty).unwrap(),
        NormalDropPlan::Unresolved
    ));
    assert!(!facts.normal_drop_required(ty, &budget).unwrap());
    let owner = publication.source().locals[publication.source().params[1].0]
        .ty
        .clone();
    assert!(facts.normal_drop_required(&owner, &budget).unwrap());
    // Even a known destructor in another field cannot hide an unresolved child.
    for parts in [[owner.clone(), ty.clone()], [ty.clone(), owner.clone()]] {
        let aggregate = Ty::Tuple(Arc::from(parts));
        assert!(!facts.normal_drop_required(&aggregate, &budget).unwrap());
        assert!(matches!(
            facts.normal_drop_plan(&aggregate).unwrap(),
            NormalDropPlan::Unresolved
        ));
    }
    // Zero arrays still compute the element's unresolved status. Skipping
    // this child would let the other field's known destructor win.
    let zero = Ty::Array {
        element: Arc::new(ty.clone()),
        len: 0,
    };
    for parts in [[owner.clone(), zero.clone()], [zero, owner.clone()]] {
        let aggregate = Ty::Tuple(Arc::from(parts));
        assert!(!facts.normal_drop_required(&aggregate, &budget).unwrap());
        assert!(matches!(
            facts.normal_drop_plan(&aggregate).unwrap(),
            NormalDropPlan::Unresolved
        ));
    }
    // Invalid children cannot turn into a successful empty-array answer.
    let invalid = Ty::Array {
        element: Arc::new(Ty::Slice {
            element: Arc::new(owner.clone()),
        }),
        len: 0,
    };
    let aggregate = Ty::Tuple(Arc::from([owner, invalid]));
    assert!(facts.normal_drop_required(&aggregate, &budget).is_err());
    assert!(facts.normal_drop_plan(&aggregate).is_err());
}
