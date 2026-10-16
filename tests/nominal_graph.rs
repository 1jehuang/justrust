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
