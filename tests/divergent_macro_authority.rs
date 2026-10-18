//! Public source-to-CFG authority checks, not native panic-runtime coverage.
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

const BUILTINS: [&str; 3] = ["unreachable", "todo", "unimplemented"];

fn plan(source: &str) -> (frontend::Program, frontend::PlanCtx, ir::DeclId) {
    let program = frontend::parse(source).unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let id = plan.function_decl(0).unwrap();
    (program, plan, id)
}

fn verify_terminal(source: &str) {
    let (program, plan, id) = plan(source);
    frontend::check(&program).unwrap();
    let facts = plan.callable_facts(id).unwrap();
    let checked = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let panics: Vec<_> = checked
        .exprs
        .iter()
        .enumerate()
        .filter_map(|(index, expr)| {
            let ir::ExprKind::Panic(payload) = &expr.kind else {
                return None;
            };
            assert_eq!(expr.ty, ir::Ty::Never);
            assert_eq!(expr.completion, ir::Completion::Never);
            assert!(!payload.location.is_empty());
            Some((ir::ExprId(index), payload.clone()))
        })
        .collect();
    assert_eq!(panics.len(), 1);
    let template = cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
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
    assert_eq!(terminals.len(), 1);
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
        "divergence must not write a result"
    );
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
}

#[test]
fn builtin_invocations_are_source_panics_with_real_invocation_provenance() {
    for builtin in BUILTINS {
        for args in ["()", "[]", "{}", "(\"detail\",)"] {
            let source = format!("fn f()->i32 {{\n{builtin}!{args}\n}} fn main() {{}}");
            let (program, plan, id) = plan(&source);
            let frontend::Expr::Panic(panic) = program.functions[0].body.tail.as_deref().unwrap()
            else {
                panic!("{builtin}! must lower through SourcePanic")
            };
            let _: &frontend::SourcePanic = panic;
            assert!(!panic.message.is_empty());
            let original = program.names.session().original(panic.site, 512).unwrap();
            let (_, line, column) = program
                .names
                .session()
                .sources()
                .location(original)
                .unwrap();
            assert_eq!((line, column), (2, 1));
            let facts = plan.callable_facts(id).unwrap();
            let checked =
                frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
            let payload = checked
                .exprs
                .iter()
                .find_map(|expr| match &expr.kind {
                    ir::ExprKind::Panic(payload) => Some(payload),
                    _ => None,
                })
                .unwrap();
            assert_eq!(payload.message, panic.message);
            assert!(!payload.location.is_empty());
        }
    }
}

#[test]
fn explicit_integer_unit_and_tuple_results_keep_exact_never_panic_cfg() {
    for builtin in BUILTINS {
        for result in ["i32", "()", "(i32,bool)"] {
            verify_terminal(&format!("fn f()->{result} {{{builtin}!()}} fn main() {{}}"));
        }
    }
}

#[test]
fn initializer_return_branch_and_let_else_preserve_terminal_authority() {
    for builtin in BUILTINS {
        for body in [
            format!("let value:i32={builtin}!();value"),
            format!("return {builtin}!()"),
            format!("if flag {{{builtin}!()}} else {{7i32}}"),
            format!("let E::A(inner)=value else {{{builtin}!()}};inner"),
        ] {
            verify_terminal(&format!(
                "fn f(flag:bool,value:E)->i32 {{{body}}} enum E {{A(i32),B}} fn main() {{}}"
            ));
        }
    }
}

#[test]
fn unused_arena_panics_require_both_exact_never_type_and_completion() {
    for builtin in BUILTINS {
        let (program, plan, id) = plan(&format!("fn f()->i32 {{{builtin}!()}} fn main() {{}}"));
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        let panic = checked
            .exprs
            .iter()
            .find(|expr| matches!(expr.kind, ir::ExprKind::Panic(_)))
            .unwrap();
        let mut unused = checked.clone();
        unused.exprs.push(panic.clone());
        unused
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        for wrong_type in [false, true] {
            let mut forged = unused.clone();
            let entry = forged.exprs.last_mut().unwrap();
            if wrong_type {
                entry.ty = ir::Ty::Unit;
            } else {
                entry.completion = ir::Completion::Returns;
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
fn foreign_invocation_span_is_rejected_even_after_successful_typing() {
    for builtin in BUILTINS {
        let source = format!("fn f()->i32 {{{builtin}!()}} fn main() {{}}");
        let (mut program, plan, id) = plan(&source);
        let foreign = frontend::parse(&source).unwrap();
        let facts = plan.callable_facts(id).unwrap();
        frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        let frontend::Expr::Panic(other) = foreign.functions[0].body.tail.as_deref().unwrap()
        else {
            panic!("source panic")
        };
        let frontend::Expr::Panic(panic) = program.functions[0].body.tail.as_deref_mut().unwrap()
        else {
            panic!("source panic")
        };
        panic.site = other.site;
        let error =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap_err();
        assert!(
            !error.to_ascii_lowercase().contains("unsupported"),
            "{error}"
        );
    }
}

#[test]
fn lowering_and_rejected_clones_leave_original_checked_body_unchanged() {
    for builtin in BUILTINS {
        let (program, plan, id) = plan(&format!("fn f()->i32 {{{builtin}!()}} fn main() {{}}"));
        let facts = plan.callable_facts(id).unwrap();
        let checked = Arc::new(
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap(),
        );
        let before = format!("{checked:?}");
        let mut forged = (*checked).clone();
        let panic = forged
            .exprs
            .iter_mut()
            .find(|expr| matches!(expr.kind, ir::ExprKind::Panic(_)))
            .unwrap();
        panic.ty = ir::Ty::Unit;
        assert!(cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err());
        let template = cfg::lower_template(checked.clone(), facts.clone(), facts.budget()).unwrap();
        assert!(
            ownership::verify_template(template)
                .unwrap()
                .analysis()
                .passed()
        );
        assert_eq!(format!("{checked:?}"), before);
        checked
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
    }
}
