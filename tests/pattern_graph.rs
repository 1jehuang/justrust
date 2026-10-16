use justrust_engine::{Key, compiler};

#[test]
fn patterns_share_one_checked_cfg_and_analysis_per_function() {
    let source = "struct S{x:i32} fn main(){let t=(S{x:2},S{x:5});let first=t.0;let (_,second)=t;let _=t;if first.x+second.x==7 {println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert_eq!(result.functions, 1);
            assert_eq!(result.structs, 1);
            assert_eq!(result.executable.is_none(), check);
            assert!(result.report.capture_complete);
            assert_eq!(result.analyses.len(), 1);
            assert!(result.analyses[0].passed());
            assert!(result.analyses[0].stats.moves > 0);
            for kind in [102, 108, 109] {
                let key = Key::new(kind, vec![0]);
                let seen: Vec<_> = result
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(k, _)| *k == key)
                    .collect();
                assert_eq!(seen.len(), 1);
                assert_eq!(seen[0].1, 1);
            }
            assert_eq!(
                result
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 103)
                    .count(),
                usize::from(!check)
            );
        }
    }
}

#[test]
fn selected_moved_pattern_leaf_retains_failed_analysis() {
    let source = "struct S{x:i32} fn main(){let t=(S{x:2},S{x:5});let first=t.0;let (again,_)=t;}";
    for check in [true, false] {
        let failure = compiler::compile_observed(source.to_owned(), 8, check)
            .err()
            .unwrap();
        assert!(failure.analyses.iter().any(|(_, a)| !a.passed()));
        assert!(failure.report.unwrap().capture_complete);
    }
}
