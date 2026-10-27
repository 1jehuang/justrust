use justrust_engine::{Key, compiler};

#[test]
fn runtime_index_facts_are_shared_by_check_and_build() {
    let source = "fn index()->usize{1}fn main(){let mut a=[3,7];let old=a[index()];a[index()]=9;if old+a[index()]==16{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.functions, 2);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 2);
            assert_eq!(compiled.analyses.len(), 2);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
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
                        .filter(|(actual, _)| *actual == key)
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
                    .filter(|(key, _)| key.kind == 103)
                    .count(),
                if check { 0 } else { 2 }
            );
        }
    }
}

#[test]
fn dynamic_wildcard_retains_failed_readiness_analysis() {
    let source = "struct S{x:i32}fn index()->usize{0}fn main(){let a:[S;1];let _=a[index()];}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.to_owned(), workers, check)
                .err()
                .unwrap();
            assert!(
                failure
                    .analyses
                    .iter()
                    .any(|(_, analysis)| !analysis.passed())
            );
            assert!(failure.report.unwrap().capture_complete);
        }
    }
}

#[test]
fn full_width_index_is_valid_source_even_when_runtime_bounds_will_fail() {
    let source = "fn main(){let a=[7];let i:usize=4294967296;let _=a[i];}";
    for check in [true, false] {
        let compiled = compiler::compile(source.to_owned(), 8, check).unwrap();
        assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        assert_eq!(compiled.executable.is_none(), check);
    }
}
