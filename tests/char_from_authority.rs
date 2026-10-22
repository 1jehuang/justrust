//! Primitive owner syntax never grants a callable or result type by itself.

#[test]
fn universal_call_types_require_exact_projection_environment() {
    let make_template = || {
        let source = "trait Has<A,B>{type Out;} fn id<I:Has<i32,bool>>(x:<I as Has<i32,bool>>::Out)-><I as Has<i32,bool>>::Out{x} fn forward<I:Has<i32,bool>>(x:<I as Has<i32,bool>>::Out)-><I as Has<i32,bool>>::Out{id::<I>(x)} fn main(){}";
        let program = frontend::parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(1).unwrap();
        let facts = plan.callable_facts(declaration).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, declaration, &plan, facts.budget())
                .unwrap();
        (checked, facts)
    };
    let (checked, facts) = make_template();
    let context = ir::FactsCtx::from_validated(&facts);
    checked.validate_with_facts(context).unwrap();
    let call = checked
        .exprs
        .iter()
        .position(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
        .unwrap();
    let ir::Ty::AssociatedType(projection) = &checked.exprs[call].ty else {
        panic!("canonical projection")
    };
    for mutation in 0..3 {
        let mut altered = projection.as_ref().clone();
        match mutation {
            0 => altered.item.snapshot = ir::SnapshotId::standalone().unwrap(),
            1 => altered.trait_ref.args = Arc::from([ir::Ty::Bool, ir::Ty::Int(ir::IntTy::I32)]),
            _ => altered.trait_ref.declaration.index = u32::MAX,
        }
        let mut forged = checked.clone();
        forged.exprs[call].ty = ir::Ty::AssociatedType(Arc::new(altered));
        assert!(
            forged.validate_with_facts(context).is_err(),
            "mutation {mutation}"
        );
    }
    let (_, foreign) = make_template();
    assert!(
        checked
            .validate_with_facts(ir::FactsCtx::from_validated(&foreign))
            .is_err()
    );
}

#[test]
fn implicit_never_trait_inference_is_explicitly_unsupported_not_coerced() {
    for workers in [1, 8] {
        for check in [true, false] {
            let error = match compiler::compile(
                "fn unused()->char{char::from(loop{})} fn main(){}".into(),
                workers,
                check,
            ) {
                Ok(_) => panic!("implicit From<!> was silently converted to From<u8>"),
                Err(error) => error.to_string(),
            };
            assert!(
                error.contains(
                    "unsupported: inferred primitive associated trait arguments requiring coercion"
                ),
                "{error}"
            );
        }
    }
}
use justrust_engine::compiler::{self, cfg, frontend, ir};
use std::sync::Arc;
#[path = "support/native_prelude.rs"]
mod native_prelude;

#[test]
fn primitive_trait_call_result_and_argument_types_remain_authenticated() {
    let program =
        native_prelude::parse("fn convert(value:u8)->char {char::from(value)} fn main(){}")
            .unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    let checked =
        frontend::type_callable_with_plan(&program, declaration, &plan, facts.budget()).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let call = checked
        .exprs
        .iter()
        .position(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
        .unwrap();
    assert_eq!(checked.exprs[call].ty, ir::Ty::Char);
    let mut forged = checked.clone();
    forged.exprs[call].ty = ir::Ty::Int(ir::IntTy::U32);
    assert!(
        forged
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .is_err()
    );
    assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
    let argument = checked
        .exprs
        .iter()
        .position(|e| e.ty == ir::Ty::Int(ir::IntTy::U8))
        .unwrap();
    let mut forged = checked.clone();
    let ir::ExprKind::Call {
        target: ir::Callee::Direct(target),
        ..
    } = &mut forged.exprs[call].kind
    else {
        panic!("canonical source implementation target");
    };
    target.declaration = plan.function_decl(1).unwrap();
    assert!(
        forged
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .is_err()
    );
    let mut forged = checked.clone();
    let ir::ExprKind::Call {
        target: ir::Callee::Direct(target),
        ..
    } = &mut forged.exprs[call].kind
    else {
        unreachable!();
    };
    target.type_args = Arc::from([ir::Ty::Int(ir::IntTy::U8)]);
    assert!(
        forged
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .is_err()
    );
    let mut forged = checked;
    forged.exprs[argument].ty = ir::Ty::Int(ir::IntTy::I8);
    assert!(
        forged
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .is_err()
    );
    assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
}
