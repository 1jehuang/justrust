use justrust_engine::{Key, compiler};

#[test]
fn shared_returns_share_exact_body_cfg_and_analysis() {
    let source = "fn id<'a>(p:&'a i32)->&'a i32{p}fn forward<'a>(p:&'a i32)->&'a i32{id(p)}fn main(){let x=7;let r=forward(&x);if *r==7{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.functions, 3);
            assert_eq!(compiled.control_flow.len(), 3);
            assert_eq!(compiled.analyses.len(), 3);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for function in &compiled.control_flow[..2] {
                assert_eq!(function.ref_contract.region_count, 1);
                assert!(matches!(
                    &function.ref_contract.results[0].returns,
                    compiler::ir::ReturnContract::SharedInputPaths(paths) if paths.len() == 1 && paths[0].input.param == 0 && paths[0].projection.is_empty()
                ));
            }
            for index in 0..3 {
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
                if check { 0 } else { 3 }
            );
        }
    }
}

#[test]
fn unrelated_pointee_lifetime_dependency_retains_failed_analysis() {
    let source = "fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}fn main(){let x=7;let r:&i32;{let s=true;r=pick(&s,&x);}println!(\"must not publish\");let y=*r;}";
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
                assert!(
                    !report
                        .node_evaluations
                        .iter()
                        .any(|(key, _)| key.kind == 103)
                );
            }
        }
    }
}

#[test]
fn unused_local_reference_return_blocks_publication() {
    let source =
        "fn bad<'a>(p:&'a i32)->&'a i32{let x=7;&x}fn main(){println!(\"must not publish\");}";
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
fn missing_universal_outlives_is_not_inferred_from_body() {
    let source = "fn pick<'a>(s:&'a bool,x:&'a i32)->&'a i32{x}fn bad<'a,'b>(s:&'b bool,x:&'a i32)->&'a i32{pick(s,x)}fn main(){}";
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
