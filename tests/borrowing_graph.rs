use justrust_engine::{Key, compiler};

#[test]
fn local_loans_share_the_cached_analysis_gate() {
    let source = "fn value()->i32{7}fn main(){let mut x=value();let r=&mut x;*r=*r+2;let seen=*r;x=x+1;if seen+x==19{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.functions, 2);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 2);
            assert_eq!(compiled.analyses.len(), 2);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            assert_eq!(
                compiled
                    .control_flow
                    .iter()
                    .map(|function| function.loan_sites.len())
                    .sum::<usize>(),
                1
            );
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
fn rejected_alias_mutation_retains_failed_analysis_without_emission() {
    let source = "fn main(){let mut x=1;let r=&x;x=2;let seen=*r;}";
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
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            if check {
                assert!(!report.published_kind(105));
            }
        }
    }
}

#[test]
fn dangling_reference_never_reaches_native_emission() {
    let source = "fn main(){let r:&i32;{let x=7;r=&x;}let seen=*r;}";
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
