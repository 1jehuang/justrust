//! Public source-to-template assertion authority. These tests do not claim
//! runtime execution, unwind, formatting, or arbitrary Rust compatibility.
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

fn plan(source: &str) -> (frontend::Program, frontend::PlanCtx) {
    let program = frontend::parse(source).unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    (program, plan)
}

fn declaration(program: &frontend::Program, plan: &frontend::PlanCtx, name: &str) -> ir::DeclId {
    let index = program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap();
    plan.function_decl(index).unwrap()
}

fn source_panic(expr: &frontend::Expr) -> &frontend::SourcePanic {
    let frontend::Expr::If { else_block, .. } = expr else {
        panic!("expected direct-condition assertion If")
    };
    let frontend::Expr::Panic(panic) = else_block.tail.as_deref().unwrap() else {
        panic!("expected cold assertion panic")
    };
    panic
}

#[test]
fn assertion_has_one_condition_unit_result_and_terminal_cold_panic() {
    let (program, plan) = plan(
        r#"
fn condition()->bool {true}
fn plain() {assert!(condition())}
fn literal() {assert!(true, "cold",)}
fn bracket() {assert![true]}
fn brace() {assert!{true, "brace"}}
fn initializer() {let value:()=assert!(true);value}
fn generic<T>(value:T) {assert!(true)}
fn main() {}
"#,
    );
    frontend::check(&program).unwrap();
    for (name, message) in [
        ("plain", "assertion failed"),
        ("literal", "cold"),
        ("bracket", "assertion failed"),
        ("brace", "brace"),
        ("initializer", "assertion failed"),
        ("generic", "assertion failed"),
    ] {
        let id = declaration(&program, &plan, name);
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        checked
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        let assertions: Vec<_> = checked
            .exprs
            .iter()
            .filter_map(|expr| match &expr.kind {
                ir::ExprKind::If { condition, .. } => {
                    assert_eq!(expr.ty, ir::Ty::Unit, "{name}");
                    assert_eq!(expr.completion, ir::Completion::Returns, "{name}");
                    assert_eq!(checked.exprs[condition.0].ty, ir::Ty::Bool, "{name}");
                    Some(condition)
                }
                _ => None,
            })
            .collect();
        assert_eq!(assertions.len(), 1, "{name}");
        let panics: Vec<_> = checked
            .exprs
            .iter()
            .enumerate()
            .filter_map(|(index, expr)| match &expr.kind {
                ir::ExprKind::Panic(payload) => {
                    assert_eq!(expr.ty, ir::Ty::Never, "{name}");
                    assert_eq!(expr.completion, ir::Completion::Never, "{name}");
                    assert_eq!(payload.message, message, "{name}");
                    Some((ir::ExprId(index), payload.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(panics.len(), 1, "{name}");
        if name == "plain" {
            assert!(matches!(
                checked.exprs[assertions[0].0].kind,
                ir::ExprKind::Call { .. }
            ));
            assert_eq!(
                checked
                    .exprs
                    .iter()
                    .filter(|expr| matches!(expr.kind, ir::ExprKind::Call { .. }))
                    .count(),
                1,
                "condition must not be duplicated or wrapped in overloaded Not"
            );
        }
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        template
            .function()
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        let terminals: Vec<_> = template
            .function()
            .blocks
            .iter()
            .filter(|block| matches!(block.terminator.value, cfg::Terminator::Panic(_)))
            .collect();
        assert_eq!(terminals.len(), 1, "{name}");
        assert_eq!(terminals[0].terminator.origin, Some(panics[0].0));
        let cfg::Terminator::Panic(payload) = &terminals[0].terminator.value else {
            unreachable!()
        };
        assert_eq!(payload, &panics[0].1);
        assert!(!terminals[0].statements.iter().any(|statement| {
            statement.origin == terminals[0].terminator.origin
                && matches!(statement.value, cfg::Statement::Assign { .. })
        }));
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed(),
            "{name}"
        );
    }
}

#[test]
fn forged_assertion_condition_and_result_fail_ir_validation_even_when_unused() {
    let (program, plan) = plan("fn f(){assert!(true)} fn main(){}");
    let id = declaration(&program, &plan, "f");
    let facts = plan.callable_facts(id).unwrap();
    let checked = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    let (assertion, condition) = checked
        .exprs
        .iter()
        .enumerate()
        .find_map(|(index, expr)| match expr.kind {
            ir::ExprKind::If { condition, .. } => Some((index, condition)),
            _ => None,
        })
        .unwrap();
    let integer = ir::Ty::Int(ir::IntTy::I32);
    for unused in [false, true] {
        let mut forged = checked.clone();
        let index = if unused {
            forged.exprs.push(checked.exprs[assertion].clone());
            forged.exprs.len() - 1
        } else {
            assertion
        };
        forged.exprs[index].ty = integer.clone();
        assert!(
            forged
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .is_err()
        );
        assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
    }
    let mut forged = checked.clone();
    forged.exprs[condition.0].kind = ir::ExprKind::Int(ir::IntValue {
        ty: ir::IntTy::I32,
        bits: 1,
    });
    forged.exprs[condition.0].ty = integer;
    assert!(
        forged
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .is_err()
    );
    assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
}

#[test]
fn nonbool_conditions_and_nonunit_results_fail_semantically_not_as_unsupported() {
    for source in [
        "fn main(){assert!(1i32)}",
        "fn main(){assert!(!1i32)}",
        "fn main(){assert!(())}",
        "fn main(){assert!((true,false))}",
        "fn f()->i32 {assert!(true)} fn main(){}",
        "fn main(){let value:i32=assert!(true);}",
        "fn f<T>(value:T){assert!(value)} fn main(){}",
        "fn main(){assert!(true);assert!(1i32)}",
        "fn main(){panic!();assert!(1i32)}",
        "fn f<T>(value:T){panic!();assert!(value)} fn main(){}",
        "fn main(){assert!(true);missing}",
    ] {
        let program = frontend::parse(source).expect("semantic errors must parse");
        let error = match frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()) {
            Err(error) => error,
            Ok(_) => frontend::check(&program).expect_err("must reject invalid assertion program"),
        };
        assert!(
            !error.to_ascii_lowercase().contains("unsupported"),
            "{source}: {error}"
        );
    }
}

#[test]
fn formatting_and_nonliteral_messages_remain_explicit_parse_boundaries() {
    for source in [
        "fn main(){assert!(true, 1i32)}",
        "fn main(){assert!(true, \"{}\", 1i32)}",
        "fn main(){assert!(true, \"value {value}\")}",
        "fn main(){assert!(true, \"literal\", 1i32)}",
    ] {
        let error =
            frontend::parse(source).expect_err("unsupported messages must fail during parsing");
        assert!(
            error.to_ascii_lowercase().contains("unsupported"),
            "{source}: {error}"
        );
    }
}

#[test]
fn assertion_locations_are_original_callsites_including_expansion() {
    for (source, suffix) in [
        ("fn f(){\nassert!(true)\n} fn main(){}", ":2:1"),
        (
            "fn f(){\nmacro_rules! check {()=>{assert!(true)};}\ncheck!()\n} fn main(){}",
            ":3:1",
        ),
    ] {
        let (program, plan) = plan(source);
        let id = declaration(&program, &plan, "f");
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        let message = checked
            .exprs
            .iter()
            .find_map(|expr| match &expr.kind {
                ir::ExprKind::Panic(payload) => Some(payload),
                _ => None,
            })
            .unwrap();
        assert!(message.location.ends_with(suffix), "{}", message.location);
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }
}

#[test]
fn foreign_assertion_panic_site_cannot_reuse_retained_source_authority() {
    let source = "fn f(){assert!(true)} fn main(){}";
    let (mut program, plan) = plan(source);
    let foreign = frontend::parse(source).unwrap();
    let foreign_site = source_panic(foreign.functions[0].body.tail.as_deref().unwrap()).site;
    let id = declaration(&program, &plan, "f");
    let facts = plan.callable_facts(id).unwrap();
    frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    let frontend::Expr::If { else_block, .. } =
        program.functions[0].body.tail.as_deref_mut().unwrap()
    else {
        unreachable!()
    };
    let frontend::Expr::Panic(panic) = else_block.tail.as_deref_mut().unwrap() else {
        unreachable!()
    };
    panic.site = foreign_site;
    // Public retained-plan typing validates the mutated source authority.
    let error = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap_err();
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "{error}"
    );
}

#[test]
fn raw_candidate_edits_do_not_replace_the_retained_plan_body() {
    for change_message in [false, true] {
        let (mut program, plan) = plan("fn f(){assert!(true,\"original\")} fn main(){}");
        let id = declaration(&program, &plan, "f");
        let facts = plan.callable_facts(id).unwrap();
        frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        let frontend::Expr::If {
            condition,
            else_block,
            ..
        } = program.functions[0].body.tail.as_deref_mut().unwrap()
        else {
            unreachable!()
        };
        if change_message {
            let frontend::Expr::Panic(panic) = else_block.tail.as_deref_mut().unwrap() else {
                unreachable!()
            };
            panic.message = "forged".into();
        } else {
            **condition = frontend::Expr::Bool(false);
        }
        // This public API checks a supplied candidate against the declaration's
        // signature. It deliberately does not claim immutable-body replay.
        let candidate =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        assert!(candidate.exprs.iter().any(|expr| {
            if change_message {
                matches!(&expr.kind, ir::ExprKind::Panic(payload) if payload.message == "forged")
            } else {
                matches!(expr.kind, ir::ExprKind::Bool(false))
            }
        }));
        // Retained-body retrieval must still use the plan's original source,
        // not the independent candidate that was just successfully checked.
        let target = ir::CallTarget {
            declaration: id,
            type_args: Arc::from([]),
        };
        let retained = plan.checked_body(&target, &facts, facts.budget()).unwrap();
        assert!(
            retained
                .exprs
                .iter()
                .any(|expr| matches!(expr.kind, ir::ExprKind::Bool(true)))
        );
        assert!(
            !retained
                .exprs
                .iter()
                .any(|expr| matches!(expr.kind, ir::ExprKind::Bool(false)))
        );
        let payload = retained
            .exprs
            .iter()
            .find_map(|expr| match &expr.kind {
                ir::ExprKind::Panic(payload) => Some(payload),
                _ => None,
            })
            .unwrap();
        assert_eq!(payload.message, "original");
        let template =
            cfg::lower_template(Arc::new(retained), facts.clone(), facts.budget()).unwrap();
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }
}

#[test]
fn lexical_assert_macro_and_ordinary_function_keep_their_identity() {
    let (program, plan) = plan(
        r#"
fn assert()->i32 {19i32}
fn shadow()->i32 {macro_rules! assert {()=>{23i32};} assert!()}
fn ordinary()->i32 {assert()}
fn main(){}
"#,
    );
    frontend::check(&program).unwrap();
    for name in ["shadow", "ordinary"] {
        let id = declaration(&program, &plan, name);
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        assert!(
            !checked
                .exprs
                .iter()
                .any(|expr| matches!(expr.kind, ir::ExprKind::Panic(_)))
        );
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
    }
    let error = frontend::parse("macro_rules! assert {()=>{23i32};} fn main(){assert!()}")
        .expect_err("known module macro must not fall back to builtin");
    assert!(
        error.contains("module macros in expression position are unsupported"),
        "{error}"
    );
}

#[test]
fn parenthesized_and_bracket_assertions_do_not_gain_if_statement_syntax() {
    for source in [
        "fn main(){assert!(true) let value=1i32;}",
        "fn main(){assert![true] let value=1i32;}",
    ] {
        assert!(frontend::parse(source).is_err(), "{source}");
    }
    let (program, _) = plan("fn main(){assert!{true} let value=1i32;}");
    frontend::check(&program).unwrap();
}
