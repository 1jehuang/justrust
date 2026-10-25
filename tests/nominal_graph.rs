use justrust_engine::{Key, compiler};

#[test]
fn nominal_types_moves_and_cached_analysis_reach_both_roots() {
    let source = "struct Leaf { n:i32 } struct Pair { a:Leaf,b:Leaf } fn take(p:Pair)->i32 {let a=p.a; a.n+p.b.n} fn main(){let p=Pair{b:Leaf{n:4},a:Leaf{n:3}};if take(p)==7 {println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert_eq!(result.structs, 2);
            assert_eq!(result.functions, 2);
            assert_eq!(result.executable.is_none(), check);
            assert_eq!(result.analyses.len(), 2);
            assert!(result.analyses.iter().all(|a| a.passed()));
            assert!(result.analyses.iter().map(|a| a.stats.moves).sum::<usize>() > 0);
            assert!(result.report.capture_complete);
            assert_eq!(result.report.failures, 0);
            for kind in [100, 101, 104, 106] {
                assert_eq!(
                    result
                        .report
                        .node_evaluations
                        .iter()
                        .filter(|(key, count)| *key == Key::new(kind, vec![]) && *count == 1)
                        .count(),
                    1
                );
            }
            for index in 0..2 {
                for kind in [102, 108, 109] {
                    assert_eq!(
                        result
                            .report
                            .node_evaluations
                            .iter()
                            .filter(
                                |(key, count)| *key == Key::new(kind, vec![index]) && *count == 1
                            )
                            .count(),
                        1
                    );
                }
            }
            assert_eq!(
                result
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 103)
                    .count(),
                if check { 0 } else { 2 }
            );
        }
    }
}

#[test]
fn rejected_source_move_retains_failed_analysis_before_codegen() {
    let source = "struct S{x:i32} fn main(){let s=S{x:1};let moved=s;let again=s;}";
    for check in [true, false] {
        let failure = compiler::compile_observed(source.to_owned(), 8, check)
            .err()
            .unwrap();
        assert!(!failure.analyses.is_empty());
        assert!(failure.analyses.iter().any(|(_, a)| !a.passed()));
        let report = failure.report.unwrap();
        assert!(report.capture_complete);
        assert!(report.failures > 0);
    }
}

#[test]
fn module_signature_producers_are_keyed_and_do_not_grow_with_sibling_callables() {
    for workers in [1, 8] {
        let mut previous = None;
        for count in [2, 4] {
            let mut source = String::from("mod nested{pub fn identity(x:i32)->i32{x}}");
            for index in 0..count {
                source.push_str(&format!("fn leaf{index}(x:i32)->i32{{x}}"));
            }
            source.push_str("fn main(){let _=nested::identity(2);}");
            let compiled = compiler::compile(source, workers, true).unwrap();
            let modules = compiled
                .report
                .node_evaluations
                .iter()
                .filter(|(key, _)| key.kind == 149)
                .cloned()
                .collect::<Vec<_>>();
            assert!(
                modules.len() >= 2,
                "root and nested module must have separate schema producers"
            );
            assert!(
                modules
                    .iter()
                    .all(|(key, evaluations)| key.params.len() == 1 && *evaluations == 1)
            );
            if let Some(previous) = &previous {
                assert_eq!(
                    &modules, previous,
                    "adding sibling functions must not add schema graph dependencies"
                );
            }
            previous = Some(modules);
            assert!(compiled.executable.is_none());
            assert!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130..=133))
            );
        }
    }
}

#[test]
fn module_signature_diagnostics_follow_callable_order_not_module_completion_order() {
    // The module's free function registers before the root's bad free
    // signature, but its bad associated signature registers afterward. Module
    // zero therefore must not win just because it is collected first.
    let source = "mod a{pub fn good(){}pub struct S{x:i32}impl S{pub fn later()->[i32]{loop{}}}}fn earlier(x:i32,x:i32){}fn main(){}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.into(), workers, check)
                .err()
                .unwrap();
            assert!(
                failure.error.to_string().contains("duplicate parameter"),
                "{}",
                failure.error
            );
            assert!(
                !failure.error.to_string().contains("unsupported"),
                "later unsupported signatures must not mask an earlier invalid signature"
            );
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            let module_ids = report
                .node_evaluations
                .iter()
                .zip(&report.node_ids)
                .filter_map(|((key, _), id)| (key.kind == 149).then_some(*id))
                .collect::<Vec<_>>();
            assert!(
                module_ids.len() >= 2,
                "the invalid fixture must reach independent signature module producers"
            );
            assert!(
                report
                    .node_errors
                    .iter()
                    .all(|(id, _)| !module_ids.contains(id)),
                "source diagnostics must be immutable outputs, not racing module failures"
            );
        }
    }
}
