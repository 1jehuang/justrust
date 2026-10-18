//! Public configured-source / PlanCtx controls for bounded debug assertions.
//! These are authority tests, not execution evidence. Disabled argument side
//! effects and panic suppression are exercised by the executable debug corpus.
use justrust_engine::compiler::{cfg, frontend, ir, ownership, source};
use std::sync::Arc;

fn prepared(text: &str, enabled: bool) -> source::PreparedSource {
    let mut options = source::SourceOptions::default();
    options.cfg.set_debug_assertions(enabled);
    source::prepare_text("debug-authority.rs", text, &options).unwrap()
}

fn program(text: &str, enabled: bool) -> frontend::Program {
    frontend::resolve(&frontend::parse_prepared(&prepared(text, enabled)).unwrap()).unwrap()
}

fn plan(program: &frontend::Program) -> frontend::PlanCtx {
    frontend::PlanCtx::from_program(program, &ir::PlanBudget::default()).unwrap()
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

fn validate(
    program: &frontend::Program,
    plan: &frontend::PlanCtx,
    name: &str,
) -> ir::CheckedFunction {
    let id = declaration(program, plan, name);
    let facts = plan.callable_facts(id).unwrap();
    let checked = frontend::type_callable_with_plan(program, id, plan, facts.budget()).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let template =
        cfg::lower_template(Arc::new(checked.clone()), facts.clone(), facts.budget()).unwrap();
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
    checked
}

fn panic_mut(expr: &mut frontend::Expr) -> &mut frontend::SourcePanic {
    match expr {
        frontend::Expr::Panic(payload) => payload,
        frontend::Expr::If {
            then_block,
            else_block,
            ..
        } => {
            if matches!(else_block.tail.as_deref(), Some(frontend::Expr::Panic(_))) {
                panic_mut(else_block.tail.as_deref_mut().unwrap())
            } else {
                panic_mut(then_block.tail.as_deref_mut().unwrap())
            }
        }
        _ => panic!("expected debug assertion and its retained cold panic"),
    }
}

#[test]
fn configured_preparation_retains_profile_independent_of_later_options_and_parse_order() {
    let text = "fn f(){debug_assert!(true)} fn main(){}";
    let mut options = source::SourceOptions::default();
    options.cfg.set_debug_assertions(false);
    let disabled = source::prepare_text("disabled.rs", text, &options).unwrap();
    options.cfg.set_debug_assertions(true);
    let enabled = source::prepare_text("enabled.rs", text, &options).unwrap();
    // Exercise both parse orders and cloned retained preparations. Host cfg is
    // intentionally not consulted by this test or used as its expected value.
    for input in [&disabled, &enabled, &enabled.clone(), &disabled.clone()] {
        let expected = input.debug_assertions();
        let parsed = frontend::resolve(&frontend::parse_prepared(input).unwrap()).unwrap();
        let retained = plan(&parsed);
        let id = declaration(&parsed, &retained, "f");
        let facts = retained.callable_facts(id).unwrap();
        let target = ir::CallTarget {
            declaration: id,
            type_args: Arc::from([]),
        };
        let checked = retained
            .checked_body(&target, &facts, facts.budget())
            .unwrap();
        // Retained-plan retrieval must preserve the same configured body, not
        // reparse under the compiler process's build profile.
        validate(&parsed, &retained, "f");
        let conditions: Vec<_> = checked
            .exprs
            .iter()
            .filter_map(|expr| {
                if let ir::ExprKind::If { condition, .. } = expr.kind {
                    Some(condition)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(conditions.len(), if expected { 1 } else { 2 });
        if !expected {
            assert!(conditions.iter().any(|condition| matches!(
                checked.exprs[condition.0].kind,
                ir::ExprKind::Bool(false)
            )));
        }
    }
    assert!(!disabled.debug_assertions());
    assert!(enabled.debug_assertions());
}

#[test]
fn all_three_macros_keep_bool_conditions_unit_results_and_cold_metadata() {
    for enabled in [false, true] {
        for invocation in [
            "debug_assert!(true, \"cold\")",
            "debug_assert_eq![true,true,\"cold\",]",
            "debug_assert_ne!{(),(),\"cold\"}",
            "debug_assert_eq!(1i32,1i32,\"cold\")",
            "debug_assert_ne!(1.0f64,2.0f64,\"cold\")",
        ] {
            let text = format!("fn f(){{\n{invocation}\n}} fn main(){{}}");
            let parsed = program(&text, enabled);
            frontend::check(&parsed).unwrap();
            let checked = validate(&parsed, &plan(&parsed), "f");
            let mut panics = 0;
            for expr in &checked.exprs {
                match &expr.kind {
                    ir::ExprKind::If { condition, .. } => {
                        assert_eq!(expr.ty, ir::Ty::Unit, "{enabled}: {invocation}");
                        assert_eq!(checked.exprs[condition.0].ty, ir::Ty::Bool);
                    }
                    ir::ExprKind::Panic(payload) => {
                        panics += 1;
                        assert_eq!(expr.ty, ir::Ty::Never);
                        assert_eq!(expr.completion, ir::Completion::Never);
                        assert_eq!(payload.message, "cold");
                        assert!(payload.location.ends_with(":2:1"), "{}", payload.location);
                    }
                    _ => {}
                }
            }
            assert_eq!(panics, 1, "{enabled}: {invocation}");
        }
    }
}

#[test]
fn divergent_arguments_are_checked_in_both_profiles() {
    for enabled in [false, true] {
        let parsed = program(
            "fn f(){debug_assert!(panic!(\"argument\"))} fn main(){}",
            enabled,
        );
        frontend::check(&parsed).unwrap();
        let checked = validate(&parsed, &plan(&parsed), "f");
        assert!(checked.exprs.iter().any(|expr| matches!(&expr.kind,
            ir::ExprKind::Panic(payload) if payload.message == "argument")
            && expr.ty == ir::Ty::Never));

        // Equality has a separate, existing bounded Never-operand boundary.
        // Disabling evaluation does not grant unsupported comparison authority.
        for invocation in [
            "debug_assert_eq!(panic!(\"argument\"),1i32)",
            "debug_assert_ne!(1i32,panic!(\"argument\"))",
        ] {
            let parsed = program(&format!("fn f(){{{invocation}}} fn main(){{}}"), enabled);
            let error = match frontend::PlanCtx::from_program(&parsed, &ir::PlanBudget::default()) {
                Err(error) => error,
                Ok(_) => frontend::check(&parsed).expect_err("Never equality remains unsupported"),
            };
            assert!(
                error.contains("binary operator on never is unsupported"),
                "{enabled}: {invocation}: {error}"
            );
        }
    }
}

#[test]
fn disabled_arguments_still_require_valid_names_types_and_unit_results() {
    for enabled in [false, true] {
        for text in [
            "fn main(){debug_assert!(1i32)}",
            "fn main(){debug_assert!(())}",
            "fn main(){debug_assert!(missing)}",
            "fn main(){debug_assert_eq!(true,1i32)}",
            "fn main(){debug_assert_ne!(1u8,1i32)}",
            "fn f()->i32{debug_assert!(true)} fn main(){}",
            "fn main(){let value:i32=debug_assert_eq!(1i32,1i32);}",
            "fn main(){let value:bool=debug_assert_ne!(true,false);}",
            "fn main(){panic!();debug_assert_eq!(true,1i32)}",
            "fn f<T>(value:T){debug_assert!(value)} fn main(){}",
            "fn f<T>(value:T){panic!();debug_assert!(value)} fn main(){}",
        ] {
            let preparation = prepared(text, enabled);
            let parsed =
                frontend::parse_prepared(&preparation).expect("semantic controls must parse");
            let error = match frontend::resolve(&parsed) {
                Err(error) => error,
                Ok(program) => {
                    match frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()) {
                        Err(error) => error,
                        Ok(_) => {
                            frontend::check(&program).expect_err("disabled does not mean untyped")
                        }
                    }
                }
            };
            assert!(
                !error.to_ascii_lowercase().contains("unsupported"),
                "{enabled}: {text}: {error}"
            );
        }
    }
}

#[test]
fn unsupported_equality_domains_and_formatting_do_not_disappear_when_disabled() {
    for enabled in [false, true] {
        for text in [
            "fn main(){let value=1i32;debug_assert_eq!(&value,&value)}",
            "fn main(){debug_assert_ne!((1i32,2i32),(2i32,1i32))}",
            "fn main(){debug_assert_eq!([1i32],[1i32])}",
        ] {
            let parsed = program(text, enabled);
            let error = match frontend::PlanCtx::from_program(&parsed, &ir::PlanBudget::default()) {
                Err(error) => error,
                Ok(_) => frontend::check(&parsed)
                    .expect_err("unsupported domain must not acquire authority"),
            };
            assert!(
                error.to_ascii_lowercase().contains("unsupported"),
                "{enabled}: {text}: {error}"
            );
        }
        for text in [
            "fn main(){debug_assert!(true,\"{}\",1i32)}",
            "fn main(){debug_assert_eq!(1i32,1i32,\"{value}\")}",
            "fn main(){debug_assert_ne!(1i32,2i32,1i32)}",
        ] {
            let error = frontend::parse_prepared(&prepared(text, enabled)).unwrap_err();
            assert!(
                error
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("unsupported"),
                "{enabled}: {text}: {error}"
            );
        }
    }
}

#[test]
fn disabled_cold_panic_metadata_cannot_be_borrowed_from_a_foreign_source() {
    for enabled in [false, true] {
        for invocation in [
            "debug_assert!(true)",
            "debug_assert_eq!(1i32,1i32)",
            "debug_assert_ne!(true,false)",
        ] {
            let text = format!("fn f(){{{invocation}}} fn main(){{}}");
            let mut parsed = program(&text, enabled);
            let mut foreign = program(&text, enabled);
            let foreign_site =
                panic_mut(foreign.functions[0].body.tail.as_deref_mut().unwrap()).site;
            let retained = plan(&parsed);
            let id = declaration(&parsed, &retained, "f");
            let facts = retained.callable_facts(id).unwrap();
            validate(&parsed, &retained, "f");
            panic_mut(parsed.functions[0].body.tail.as_deref_mut().unwrap()).site = foreign_site;
            let error = frontend::type_callable_with_plan(&parsed, id, &retained, facts.budget())
                .unwrap_err();
            assert!(
                error.contains("contract") || error.contains("foreign"),
                "foreign debug assertion metadata must not acquire authority: {error}"
            );
            assert!(frontend::check(&parsed).is_err());
            assert!(frontend::PlanCtx::from_program(&parsed, &ir::PlanBudget::default()).is_err());
            // Candidate rejection must not poison retrieval of the original body.
            let target = ir::CallTarget {
                declaration: id,
                type_args: Arc::from([]),
            };
            retained
                .checked_body(&target, &facts, facts.budget())
                .unwrap();
        }
    }
}

#[test]
fn lexical_macros_and_ordinary_functions_keep_exact_identity_in_both_profiles() {
    let text = r#"
fn debug_assert()->i32 {19i32}
fn ordinary()->i32 {debug_assert()}
fn boolean()->i32 {macro_rules! debug_assert {()=>{23i32};} debug_assert!()}
fn equal()->i32 {macro_rules! debug_assert_eq {()=>{29i32};} debug_assert_eq!()}
fn unequal()->i32 {macro_rules! debug_assert_ne {()=>{31i32};} debug_assert_ne!()}
fn main(){}
"#;
    for enabled in [false, true] {
        let parsed = program(text, enabled);
        frontend::check(&parsed).unwrap();
        let retained = plan(&parsed);
        for name in ["ordinary", "boolean", "equal", "unequal"] {
            let checked = validate(&parsed, &retained, name);
            assert!(
                !checked.exprs.iter().any(|expr| matches!(
                    expr.kind,
                    ir::ExprKind::Panic(_) | ir::ExprKind::If { .. }
                ))
            );
        }
        for name in ["debug_assert", "debug_assert_eq", "debug_assert_ne"] {
            let text = format!("macro_rules! {name} {{()=>{{23i32}};}} fn main(){{{name}!()}}");
            let error = frontend::parse_prepared(&prepared(&text, enabled)).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("module macros in expression position are unsupported"),
                "{error}"
            );
        }
    }
}
