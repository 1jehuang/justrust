//! Source-to-template panic authority, not native runtime or unwinding coverage.
//! Formatted panic bodies and macro shadowing retain their source identity.
//! Every fact and source handle below comes from a real parsed program.
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;
#[path = "support/iterator_oracle.rs"]
mod oracle;

#[test]
fn thread_panicking_ordinary_source_calls_imports_and_shadows_match_stock() {
    for source in [
        "fn main(){assert!(!std::thread::panicking());}",
        "use std::thread::panicking as query;fn main(){assert!(!query());}",
        "use std as standard;fn main(){assert!(!standard::thread::panicking());}",
        "use std::thread as thread_query;fn main(){assert!(!thread_query::panicking());}",
        "use std::thread::*;fn main(){assert!(!panicking());}",
        "mod exported{pub use std::thread::panicking as query;}fn main(){assert!(!exported::query());}",
        "fn main(){let query=std::thread::panicking;assert!(!query());}",
        "mod std{pub mod thread{pub fn panicking()->bool{true}}}fn main(){assert!(std::thread::panicking());}",
        "mod std{pub mod thread{pub fn panicking()->bool{true}}}use std as local_std;fn main(){assert!(local_std::thread::panicking());}",
        "use std::thread::panicking as real;mod local{pub mod std{pub mod thread{pub fn panicking()->bool{true}}}pub fn query()->bool{std::thread::panicking()}}fn main(){assert!(!real());assert!(local::query());}",
        "mod thread{pub fn panicking()->bool{true}}fn main(){assert!(thread::panicking());assert!(!std::thread::panicking());}",
        "use std::thread::panicking;fn main(){let panicking=||true;assert!(panicking());assert!(!std::thread::panicking());}",
        "use std::thread::*;fn panicking()->bool{true}fn main(){assert!(panicking());}",
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn thread_panicking_invalid_unused_calls_and_core_namespace_match_stock_rejection() {
    for source in [
        "fn unused(){std::thread::panicking(1i32);}fn main(){}",
        "fn unused()->i32{std::thread::panicking()}fn main(){}",
        "fn unused(){std::thread::panicking::<bool>();}fn main(){}",
        "fn unused(){core::thread::panicking();}fn main(){}",
        "use core as c;fn unused(){c::thread::panicking();}fn main(){}",
        "extern crate core as std;fn unused(){std::thread::panicking();}fn main(){}",
        "const fn unused()->bool{std::thread::panicking()}fn main(){}",
        "#![no_std]\nfn unused(){std::thread::panicking();}fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn thread_panicking_recipe_is_exact_bool_call_and_rejects_forged_owner() {
    let (program, plan) =
        plan("fn query()->bool{std::thread::panicking()}fn ordinary()->bool{false}fn main(){}");
    let query_id = declaration(&program, &plan, "query");
    let query_facts = plan.callable_facts(query_id).unwrap();
    let query =
        frontend::type_callable_with_plan(&program, query_id, &plan, query_facts.budget()).unwrap();
    let target = query
        .exprs
        .iter()
        .find_map(|expression| match &expression.kind {
            ir::ExprKind::Call {
                target: ir::Callee::Direct(target),
                args,
                signature,
                ..
            } => {
                assert!(args.is_empty());
                assert!(signature.params.is_empty());
                assert_eq!(signature.result, ir::Ty::Bool);
                Some(target.clone())
            }
            _ => None,
        })
        .expect("canonical query must remain an ordinary direct call");
    assert!(matches!(
        plan.callable_provider(target.declaration).unwrap(),
        frontend::CallableProvider::Recipe {
            descriptor: justrust_engine::compiler::traits::RecipeDescriptor::ThreadPanicking
        }
    ));
    let facts = plan.callable_facts(target.declaration).unwrap();
    let recipe = plan.checked_body(&target, &facts, facts.budget()).unwrap();
    assert!(recipe.params.is_empty());
    assert!(recipe.locals.is_empty());
    assert_eq!(recipe.result, ir::Ty::Bool);
    assert_eq!(recipe.exprs.len(), 1);
    assert!(matches!(
        recipe.exprs[0].kind,
        ir::ExprKind::RuntimeQuery(_)
    ));
    assert_eq!(recipe.exprs[0].completion, ir::Completion::Returns);
    recipe
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    cfg::lower_template(Arc::new(recipe.clone()), facts.clone(), facts.budget()).unwrap();

    let ordinary_id = declaration(&program, &plan, "ordinary");
    let ordinary_facts = plan.callable_facts(ordinary_id).unwrap();
    assert!(
        recipe
            .validate_with_facts(ir::FactsCtx::Validated(&ordinary_facts))
            .is_err()
    );
    assert!(
        cfg::lower_template(
            Arc::new(recipe.clone()),
            ordinary_facts.clone(),
            ordinary_facts.budget()
        )
        .is_err()
    );
    let mut ordinary =
        frontend::type_callable_with_plan(&program, ordinary_id, &plan, ordinary_facts.budget())
            .unwrap();
    ordinary.exprs.push(recipe.exprs[0].clone());
    assert!(
        ordinary
            .validate_with_facts(ir::FactsCtx::Validated(&ordinary_facts))
            .is_err(),
        "an unused forged query expression still needs its genuine recipe owner"
    );

    let (foreign_program, foreign_plan) =
        self::plan("fn query()->bool{std::thread::panicking()}fn main(){}");
    let foreign_id = declaration(&foreign_program, &foreign_plan, "query");
    let foreign_facts = foreign_plan.callable_facts(foreign_id).unwrap();
    assert!(
        recipe
            .validate_with_facts(ir::FactsCtx::Validated(&foreign_facts))
            .is_err()
    );
}

#[test]
fn thread_panicking_source_certificate_cannot_be_reconstructed() {
    let (program, plan) =
        plan("fn query()->bool{std::thread::panicking()}fn ordinary()->bool{true}fn main(){}");
    let query_id = declaration(&program, &plan, "query");
    let facts = plan.callable_facts(query_id).unwrap();
    let mut forged = program.clone();
    let frontend::Expr::Call { name, .. } =
        forged.functions[0].body.tail.as_mut().unwrap().as_mut()
    else {
        panic!("query source call changed")
    };
    *name = frontend::AstPath::new(program.names.as_ref(), name.anchor(), name.segments()).unwrap();
    assert!(frontend::type_callable_with_plan(&forged, query_id, &plan, facts.budget()).is_err());
}

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

#[test]
fn literal_panic_is_exact_never_and_terminal_without_result_assignment() {
    let source = r#"
fn empty()->i32 {panic!()}
fn literal()->i32 {panic!("cold",)}
fn bracket()->i32 {panic!["bracket"]}
fn brace()->i32 {panic!{"brace"}}
fn escaped()->i32 {panic!("line\n\"quoted\"")}
fn generic<T>(value:T)->T {panic!("generic")}
fn branch(flag:bool)->i32 {if flag {panic!("branch")} else {7i32}}
fn initializer()->i32 {let value:i32=panic!("initializer");value}
fn returning()->i32 {return panic!("return")}
enum E {A(i32),B}
fn let_else(value:E)->i32 {let E::A(inner)=value else {panic!("else")};inner}
fn main() {}
"#;
    let (program, plan) = plan(source);
    frontend::check(&program).unwrap();
    for (name, message) in [
        ("empty", "explicit panic"),
        ("literal", "cold"),
        ("bracket", "bracket"),
        ("brace", "brace"),
        ("escaped", "line\n\"quoted\""),
        ("generic", "generic"),
        ("branch", "branch"),
        ("initializer", "initializer"),
        ("returning", "return"),
        ("let_else", "else"),
    ] {
        let id = declaration(&program, &plan, name);
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        checked
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        let panics: Vec<_> = checked
            .exprs
            .iter()
            .enumerate()
            .filter_map(|(index, expr)| match &expr.kind {
                ir::ExprKind::Panic(payload) => {
                    assert_eq!(expr.ty, ir::Ty::Never, "{name}");
                    assert_eq!(expr.completion, ir::Completion::Never, "{name}");
                    assert_eq!(payload.message, message, "{name}");
                    assert!(!payload.location.is_empty(), "{name}");
                    Some((ir::ExprId(index), payload.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(panics.len(), 1, "{name}");
        let template =
            cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
        let function = template.function();
        function
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        let terminals: Vec<_> = function
            .blocks
            .iter()
            .filter(|block| matches!(block.terminator.value, cfg::Terminator::Panic(_)))
            .collect();
        assert_eq!(terminals.len(), 1, "{name}");
        let block = terminals[0];
        assert_eq!(block.terminator.origin, Some(panics[0].0));
        let cfg::Terminator::Panic(payload) = &block.terminator.value else {
            unreachable!()
        };
        assert_eq!(payload, &panics[0].1);
        assert!(
            !block.statements.iter().any(|statement| {
                statement.origin == block.terminator.origin
                    && matches!(statement.value, cfg::Statement::Assign { .. })
            }),
            "panic must not assign its nonexistent result: {name}"
        );
        // The distinct Panic variant carries only its diagnostic payload,
        // with neither a successor block nor a result operand.
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
fn forged_panic_type_or_completion_is_rejected_including_unused_dead_entries() {
    let (program, plan) = plan("fn f()->i32 {panic!(\"cold\")} fn main() {}");
    let id = declaration(&program, &plan, "f");
    let facts = plan.callable_facts(id).unwrap();
    let checked = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    let index = checked
        .exprs
        .iter()
        .position(|expr| matches!(expr.kind, ir::ExprKind::Panic(_)))
        .unwrap();
    for unused in [false, true] {
        for wrong_type in [false, true] {
            let mut forged = checked.clone();
            let index = if unused {
                forged.exprs.push(checked.exprs[index].clone());
                forged.exprs.len() - 1
            } else {
                index
            };
            if wrong_type {
                forged.exprs[index].ty = ir::Ty::Unit;
            } else {
                forged.exprs[index].completion = ir::Completion::Returns;
            }
            let error = forged
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .unwrap_err();
            assert!(error.contains("panic requires exact Never"), "{error}");
            assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
        }
    }
}

#[test]
fn foreign_original_source_span_cannot_reuse_a_retained_plan() {
    let source = "fn f()->i32 {panic!(\"cold\")} fn main() {}";
    let (mut program, plan) = plan(source);
    let foreign = frontend::parse(source).unwrap();
    let id = declaration(&program, &plan, "f");
    let facts = plan.callable_facts(id).unwrap();
    frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    let frontend::Expr::Panic(foreign_panic) = foreign.functions[0].body.tail.as_deref().unwrap()
    else {
        panic!("expected real original-source panic")
    };
    let frontend::Expr::Panic(panic) = program.functions[0].body.tail.as_deref_mut().unwrap()
    else {
        panic!("expected real original-source panic")
    };
    panic.site = foreign_panic.site;
    let error = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap_err();
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "{error}"
    );
}

#[test]
fn lexical_macro_and_ordinary_function_named_panic_keep_their_identity() {
    let (program, plan) = plan(
        r#"
fn panic()->i32 {19i32}
fn f()->i32 {macro_rules! panic {() => {23i32};} panic!()}
fn ordinary()->i32 {panic()}
fn main() {}
"#,
    );
    frontend::check(&program).unwrap();
    for name in ["f", "ordinary"] {
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
}

#[test]
fn known_module_macro_is_not_silently_replaced_by_builtin_panic() {
    oracle::stock_native(
        "macro_rules! panic {() => {23i32};} fn f()->i32 {panic!()} fn main() {assert_eq!(f(),23i32);}",
    );
}

#[test]
fn formatting_is_supported_and_invalid_arguments_still_reject() {
    for source in [
        "fn unused()->i32{panic!(\"{}\", 1i32)} fn main(){}",
        "fn unused()->i32{panic!(\"first\");panic!(\"{}\", 1i32)} fn main(){}",
    ] {
        oracle::stock_native(source);
    }
    for source in [
        "fn main(){panic!(1i32)}",
        "fn unused(){panic!(\"{}\")} fn main(){}",
        "fn unused(){panic!(\"literal\",1i32)} fn main(){}",
    ] {
        oracle::stock_and_native_reject(source);
    }
}

#[test]
fn all_body_dead_code_and_unused_generic_bodies_still_receive_diagnostics() {
    for source in [
        "fn f()->i32 {panic!(\"stop\");missing} fn main() {}",
        "fn f()->i32 {panic!(\"stop\");let value:i32=true;value} fn main() {}",
        "fn f<T>(value:T)->i32 {panic!(\"stop\");missing} fn main() {}",
        "fn f<T>(value:T)->i32 {panic!(\"stop\");let number:i32=true;number} fn main() {}",
    ] {
        let program = frontend::parse(source).expect("dead semantic errors must parse");
        let error = match frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()) {
            Err(error) => error,
            Ok(_) => frontend::check(&program).expect_err("dead body errors must not be discarded"),
        };
        assert!(
            !error.to_ascii_lowercase().contains("unsupported"),
            "{error}"
        );
    }
}
