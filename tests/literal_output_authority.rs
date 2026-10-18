//! Public literal-output source, retained-plan, IR and CFG authority.
//! Structural checks do not claim runtime parity or general formatting support.
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

fn output_mut(program: &mut frontend::Program) -> &mut frontend::SourceOutput {
    let frontend::Expr::Output(output) = program.functions[0].body.tail.as_deref_mut().unwrap()
    else {
        panic!("expected original-source literal output")
    };
    output
}

fn outputs(checked: &ir::CheckedFunction) -> Vec<(ir::ExprId, ir::LiteralOutput)> {
    checked
        .exprs
        .iter()
        .enumerate()
        .filter_map(|(index, expr)| {
            let ir::ExprKind::Output(output) = &expr.kind else {
                return None;
            };
            assert_eq!(expr.ty, ir::Ty::Unit);
            assert_eq!(expr.completion, ir::Completion::Returns);
            Some((ir::ExprId(index), output.clone()))
        })
        .collect()
}

#[test]
fn literal_forms_are_exact_unit_returning_payloads() {
    for (invocation, text, stream, newline) in [
        (
            r#"print!("hello",)"#,
            "hello",
            ir::OutputStream::Stdout,
            false,
        ),
        (
            r#"print!["λ {{brace}}\n"]"#,
            "λ {brace}\n",
            ir::OutputStream::Stdout,
            false,
        ),
        (r#"print!{""}"#, "", ir::OutputStream::Stdout, false),
        (
            r#"eprint!("error")"#,
            "error",
            ir::OutputStream::Stderr,
            false,
        ),
        (r#"eprint!["",]"#, "", ir::OutputStream::Stderr, false),
        (
            r#"eprint!{"brace"}"#,
            "brace",
            ir::OutputStream::Stderr,
            false,
        ),
        (
            r#"eprintln!("line",)"#,
            "line",
            ir::OutputStream::Stderr,
            true,
        ),
        ("eprintln![]", "", ir::OutputStream::Stderr, true),
        (r#"eprintln!{"λ"}"#, "λ", ir::OutputStream::Stderr, true),
    ] {
        let (program, plan) = plan(&format!("fn f(){{{invocation}}} fn main(){{}}"));
        frontend::check(&program).unwrap();
        let id = declaration(&program, &plan, "f");
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        checked
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        let actual = outputs(&checked);
        assert_eq!(actual.len(), 1, "{invocation}");
        assert_eq!(
            actual[0].1,
            ir::LiteralOutput {
                text: text.into(),
                stream,
                newline
            },
            "{invocation}"
        );
    }
}

#[test]
fn cfg_preserves_order_stream_newline_and_operand_free_payloads() {
    let (program, plan) = plan(
        r#"fn f(){print!("out");eprint!("err");eprintln!("line");print!("");eprintln!()} fn main(){}"#,
    );
    let id = declaration(&program, &plan, "f");
    let facts = plan.callable_facts(id).unwrap();
    let checked = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    let expected = outputs(&checked);
    assert_eq!(expected.len(), 5);
    let template = cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).unwrap();
    let function = template.function();
    function
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    let actual: Vec<_> = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| {
            // The public Output variant contains only a literal payload, not a
            // callee, argument list, place, or operand. Do not use private visitors.
            if let cfg::Statement::Assign {
                value:
                    cfg::Rvalue::Output(ir::LiteralOutput {
                        text,
                        stream,
                        newline,
                    }),
                ..
            } = &statement.value
            {
                Some((
                    statement.origin.unwrap(),
                    ir::LiteralOutput {
                        text: text.clone(),
                        stream: *stream,
                        newline: *newline,
                    },
                ))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(actual, expected);
    assert!(function.blocks.iter().all(|block| !matches!(
        block.terminator.value,
        cfg::Terminator::Call { .. } | cfg::Terminator::Panic(_)
    )));
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
}

#[test]
fn malformed_result_and_completion_fail_even_in_unused_rows() {
    for invocation in [r#"print!("out")"#, r#"eprint!("err")"#, "eprintln!()"] {
        let (program, plan) = plan(&format!("fn f(){{{invocation}}} fn main(){{}}"));
        let id = declaration(&program, &plan, "f");
        let facts = plan.callable_facts(id).unwrap();
        let checked =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        let original = outputs(&checked)[0].0.0;
        for unused in [false, true] {
            for wrong_type in [false, true] {
                let mut forged = checked.clone();
                let index = if unused {
                    forged.exprs.push(checked.exprs[original].clone());
                    forged.exprs.len() - 1
                } else {
                    original
                };
                if wrong_type {
                    forged.exprs[index].ty = ir::Ty::Never;
                } else {
                    forged.exprs[index].completion = ir::Completion::Never;
                }
                let error = forged
                    .validate_with_facts(ir::FactsCtx::Validated(&facts))
                    .unwrap_err();
                assert!(
                    error.contains("literal output requires exact Unit"),
                    "{invocation}, unused={unused}: {error}"
                );
                assert!(
                    cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err()
                );
            }
        }
    }
}

#[test]
fn foreign_original_source_site_is_rejected_including_unused_functions() {
    for invocation in [r#"print!("out")"#, r#"eprint!("err")"#, "eprintln!()"] {
        let source = format!("fn f(){{{invocation}}} fn main(){{}}");
        let (mut program, plan) = plan(&source);
        let mut foreign = frontend::parse(&source).unwrap();
        let id = declaration(&program, &plan, "f");
        let facts = plan.callable_facts(id).unwrap();
        frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
        output_mut(&mut program).site = output_mut(&mut foreign).site;
        let error =
            frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap_err();
        assert!(
            error.contains("contract") || error.contains("foreign"),
            "{error}"
        );
        let error = match frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()) {
            Err(error) => error,
            Ok(_) => panic!("unused function imported a foreign output site"),
        };
        assert!(
            error.contains("contract") || error.contains("foreign"),
            "{error}"
        );
    }
}

#[test]
fn candidate_payload_edits_do_not_replace_immutable_plan_output() {
    let (mut program, plan) = plan(r#"fn f(){print!("original")} fn main(){}"#);
    let id = declaration(&program, &plan, "f");
    let facts = plan.callable_facts(id).unwrap();
    let original = output_mut(&mut program).output.clone();
    let replacement = ir::LiteralOutput {
        text: "candidate".into(),
        stream: ir::OutputStream::Stderr,
        newline: true,
    };
    output_mut(&mut program).output = replacement.clone();
    // Raw candidate typing is not a source-equality authority check.
    let candidate = frontend::type_callable_with_plan(&program, id, &plan, facts.budget()).unwrap();
    assert_eq!(outputs(&candidate)[0].1, replacement);
    let target = ir::CallTarget {
        declaration: id,
        type_args: Arc::from([]),
    };
    let retained = plan.checked_body(&target, &facts, facts.budget()).unwrap();
    assert_eq!(outputs(&retained)[0].1, original);
    let template = cfg::lower_template(Arc::new(retained), facts.clone(), facts.budget()).unwrap();
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
}

#[test]
fn checked_body_requires_real_matching_callable_and_world_authority() {
    let source = r#"fn f(){print!("f")} fn g(){eprintln!("g")} fn main(){}"#;
    let (program, plan) = plan(source);
    let f = declaration(&program, &plan, "f");
    let g = declaration(&program, &plan, "g");
    let f_facts = plan.callable_facts(f).unwrap();
    let g_facts = plan.callable_facts(g).unwrap();
    let target = ir::CallTarget {
        declaration: f,
        type_args: Arc::from([]),
    };
    assert!(
        plan.checked_body(&target, &f_facts, f_facts.budget())
            .is_ok()
    );
    assert!(
        plan.checked_body(&target, &g_facts, g_facts.budget())
            .is_err()
    );
    let (foreign_program, foreign_plan) = self::plan(source);
    let foreign_id = declaration(&foreign_program, &foreign_plan, "f");
    let foreign_facts = foreign_plan.callable_facts(foreign_id).unwrap();
    assert!(
        plan.checked_body(&target, &foreign_facts, foreign_facts.budget())
            .is_err()
    );
}

#[test]
fn formatting_and_nonliteral_inputs_do_not_gain_output_capability() {
    for name in ["print", "eprint", "eprintln"] {
        for arguments in [r#""{}",1i32"#, r#""{value}""#, r#""{:?}""#, "1i32", "value"] {
            let source = format!("fn main(){{{name}!({arguments})}}");
            let error = frontend::parse(&source).unwrap_err();
            assert!(
                error.to_ascii_lowercase().contains("unsupported"),
                "{source}: {error}"
            );
        }
    }
    for name in ["print", "eprint"] {
        assert!(frontend::parse(&format!("fn main(){{{name}!()}}")).is_err());
    }
}
