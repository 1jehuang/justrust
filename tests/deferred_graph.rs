use justrust_engine::{Key, compiler};

#[test]
fn deferred_initialization_uses_one_cached_analysis_per_function() {
    let source = "fn choose(b:bool)->i32{let x:i32;if b{x=3;}else{x=7;}x} fn main(){let pair:(i32,bool);pair=(choose(true),true);if pair.0==3&&pair.1{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.analyses.len(), 2);
            assert!(compiled.analyses.iter().all(|a| a.passed()));
            for index in 0..2 {
                for &kind in if check {
                    &[][..]
                } else {
                    &[102u32, 108, 109][..]
                } {
                    let key = Key::new(kind, vec![index]);
                    let evaluations: Vec<_> = compiled
                        .report
                        .node_evaluations
                        .iter()
                        .filter(|(k, _)| *k == key)
                        .collect();
                    assert_eq!(evaluations.len(), 1);
                    assert_eq!(evaluations[0].1, 1);
                }
            }
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(k, _)| k.kind == 103)
                    .count(),
                if check { 0 } else { 2 }
            );
        }
    }
}

#[test]
fn deferred_initialization_errors_retain_the_failed_analysis() {
    for source in [
        "fn main(){let x:i32;let y=x;}",
        "fn main(){let x:i32;x=1;x=2;}",
        "fn f(b:bool)->i32{let x:i32;if b{x=1;}x}fn main(){}",
        "fn f(b:bool){let x:i32;loop{x=1;if b{break;}}}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failed = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .expect("invalid initialization must fail");
                assert!(failed.analyses.iter().any(|(_, a)| !a.passed()));
                assert!(failed.report.unwrap().capture_complete);
            }
        }
    }
}

#[test]
fn ignored_uninitialized_enum_place_does_not_create_a_read() {
    let source = "enum E{A,B}fn main(){let e:E;match e{_=>{}};}";
    for check in [true, false] {
        let compiled = compiler::compile(source.to_owned(), 8, check).unwrap();
        assert!(compiled.analyses.iter().all(|a| a.passed()));
        assert!(
            compiled
                .control_flow
                .iter()
                .flat_map(|f| &f.blocks)
                .flat_map(|b| &b.statements)
                .all(|s| !matches!(
                    &s.value,
                    compiler::cfg::Statement::Assign {
                        value: compiler::cfg::Rvalue::Discriminant(_),
                        ..
                    }
                ))
        );
    }
}

#[test]
fn initialized_immutable_unused_body_is_typed_but_never_emitted() {
    use compiler::{backend, cfg, frontend, ownership};
    let source = "fn unused(){let x=1;x=2;}fn main(){}";
    let program = frontend::parse(source).unwrap();
    let env = frontend::signature_table(&program).unwrap();
    let index = program
        .functions
        .iter()
        .position(|f| {
            f.name.check(program.names.as_ref()).unwrap();
            f.name.anchor() == frontend::PathAnchor::Crate
                && f.name.len() == 1
                && program.names.spelling(f.name.segments()[0]).unwrap() == "unused"
        })
        .unwrap();
    let budget = compiler::ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.callable_decl(index).unwrap();
    let checked = frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan
        .instance_facts(&compiler::ir::CallTarget {
            declaration,
            type_args: std::sync::Arc::from([]),
        })
        .unwrap();
    let lowered = cfg::lower(&checked).unwrap();
    assert!(!ownership::analyze(&lowered).passed());
    assert!(frontend::check_function_with_signatures(&program, index, &env).is_err());
    assert!(backend::emit_checked_function(&checked, facts).is_err());
    assert!(backend::emit(&program).is_err());
    for check in [true, false] {
        let failed = compiler::compile_observed(source.to_owned(), 8, check)
            .err()
            .unwrap();
        assert!(failed.analyses.iter().any(|(_, a)| !a.passed()));
    }
}
