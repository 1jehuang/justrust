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
    let program = frontend::parse("fn f<T>(value:T){}fn main(){}").unwrap();
    let budget = PlanBudget::default();
    let context = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let frontend::PublishedCallableBody::Source(publication) = context
        .publish_callable_body(context.function_decl(0).unwrap(), &budget)
        .unwrap()
    else {
        panic!("source expected")
    };
    let ty = &publication.source().locals[publication.source().params[0].0].ty;
    assert!(matches!(
        publication.facts().normal_drop_plan(ty).unwrap(),
        NormalDropPlan::Unresolved
    ));
}
