use justrust_engine::{Key, compiler};

#[test]
fn array_lowering_and_analysis_are_shared_by_check_and_build() {
    let source = "fn make()->[i32;3]{[2,3,5]}fn main(){let mut a=make();let old=a;a[1]=7;if old[1]+a[1]==10{println!(\"ok\");}}";
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
                for kind in [102, 108, 109] {
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
fn ignored_index_still_retains_failed_initialization_analysis() {
    let source = "fn main(){let a:[i32;2];let _=a[0];}";
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
fn noncopy_indexed_moves_never_reach_native_emission() {
    let source = "struct S{x:i32}fn main(){let a=[S{x:1}];let s=a[0];}";
    for check in [true, false] {
        assert!(compiler::compile(source.to_owned(), 8, check).is_err());
    }
}
