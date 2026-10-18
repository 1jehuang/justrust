//! Public matches! source, checked-body and CFG authority, not runtime execution.
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

fn plan(source: &str) -> (frontend::Program, frontend::PlanCtx) {
    let program = frontend::parse(source).unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    (program, plan)
}

fn declaration(program: &frontend::Program, plan: &frontend::PlanCtx) -> ir::DeclId {
    let index = program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == "f"
        })
        .unwrap();
    plan.function_decl(index).unwrap()
}

fn validate(program: &frontend::Program, plan: &frontend::PlanCtx) {
    frontend::check(program).unwrap();
    let id = declaration(program, plan);
    let facts = plan.callable_facts(id).unwrap();
    let checked = frontend::type_callable_with_plan(program, id, plan, facts.budget()).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let matches: Vec<_> = checked
        .exprs
        .iter()
        .filter(|expr| matches!(expr.kind, ir::ExprKind::Match { .. }))
        .collect();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].ty, ir::Ty::Bool);
    assert_eq!(matches[0].completion, ir::Completion::Returns);
    let template = cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
    template
        .function()
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
}

#[test]
fn matches_result_is_bool_across_delimiters() {
    for expression in ["matches!(1, 1)", "matches![1, 1]", "matches!{1, 1,}"] {
        let (program, plan) = plan(&format!("fn f()->bool {{{expression}}} fn main(){{}}"));
        validate(&program, &plan);
    }
}

#[test]
fn matches_scrutinee_call_occurs_once_in_checked_body() {
    let (program, plan) =
        plan("fn value()->i32 {4} fn f()->bool {matches!(value(), 4)} fn main(){}");
    let id = declaration(&program, &plan);
    let facts = plan.callable_facts(id).unwrap();
    let checked = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    let scrutinee = checked
        .exprs
        .iter()
        .find_map(|expr| match expr.kind {
            ir::ExprKind::Match { scrutinee, .. } => Some(scrutinee),
            _ => None,
        })
        .unwrap();
    assert!(matches!(
        checked.exprs[scrutinee.0].kind,
        ir::ExprKind::Call { .. }
    ));
    assert_eq!(
        checked
            .exprs
            .iter()
            .filter(|expr| matches!(expr.kind, ir::ExprKind::Call { .. }))
            .count(),
        1
    );
    validate(&program, &plan);
}

#[test]
fn enum_and_tuple_patterns_reach_validated_cfg() {
    for source in [
        "enum E {A,B} fn f(value:E)->bool {matches!(value,E::A)} fn main(){}",
        "enum E {A(i32),B} fn f(value:E)->bool {matches!(value,E::A(2))} fn main(){}",
        "fn f()->bool {matches!((1,true),(1,true))} fn main(){}",
    ] {
        let (program, plan) = plan(source);
        validate(&program, &plan);
    }
}

#[test]
fn cloned_source_preserves_public_planning_authority() {
    let (program, retained) = plan("fn f()->bool {matches!(1,1)} fn main(){}");
    let cloned = program.clone();
    validate(&cloned, &retained);
    let fresh = frontend::PlanCtx::from_program(&cloned, &ir::PlanBudget::default()).unwrap();
    validate(&cloned, &fresh);
}

#[test]
fn foreign_scrutinee_name_cannot_acquire_source_authority() {
    let source = "fn f(value:i32)->bool {matches!(value,1)} fn main(){}";
    let (mut program, plan) = plan(source);
    let foreign = frontend::parse(source).unwrap();
    let frontend::Expr::Match {
        scrutinee: other, ..
    } = foreign.functions[0].body.tail.as_deref().unwrap()
    else {
        panic!("expected match")
    };
    let id = declaration(&program, &plan);
    let facts = plan.callable_facts(id).unwrap();
    let frontend::Expr::Match { scrutinee, .. } =
        program.functions[0].body.tail.as_deref_mut().unwrap()
    else {
        panic!("expected match")
    };
    **scrutinee = (**other).clone();
    assert!(frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).is_err());
    assert!(frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).is_err());
}

#[test]
fn candidate_edits_do_not_replace_original_checked_body() {
    let (mut program, plan) = plan("fn f()->bool {matches!(true,true)} fn main(){}");
    let id = declaration(&program, &plan);
    let facts = plan.callable_facts(id).unwrap();
    let frontend::Expr::Match { scrutinee, .. } =
        program.functions[0].body.tail.as_deref_mut().unwrap()
    else {
        panic!("expected match")
    };
    **scrutinee = frontend::Expr::Bool(false);
    let candidate = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    let scrutinee_bool = |body: &ir::CheckedFunction| {
        body.exprs
            .iter()
            .find_map(|expr| {
                let ir::ExprKind::Match { scrutinee, .. } = expr.kind else {
                    return None;
                };
                let ir::ExprKind::Bool(value) = body.exprs[scrutinee.0].kind else {
                    panic!("expected bool scrutinee")
                };
                Some(value)
            })
            .unwrap()
    };
    assert!(!scrutinee_bool(&candidate));
    let target = ir::CallTarget {
        declaration: id,
        type_args: Arc::from([]),
    };
    let retained = plan.checked_body(&target, &facts, facts.budget()).unwrap();
    assert!(scrutinee_bool(&retained));
    let template = cfg::lower_template(Arc::new(retained), facts.clone(), facts.budget()).unwrap();
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
}
