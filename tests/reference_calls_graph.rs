use justrust_engine::compiler;

#[test]
fn reference_calls_share_one_analysis_per_body() {
    let source = "fn bump(p:&mut i32){*p=*p+1;}fn read(p:&i32)->i32{*p}fn main(){let mut x=5;let r=&mut x;bump(r);bump(r);if read(r)==7{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.functions, 3);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 3);
            assert_eq!(compiled.analyses.len(), 3);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            assert!(compiled.control_flow[0].loan_sites.is_empty());
            assert!(compiled.control_flow[1].loan_sites.is_empty());
            // CHECK validates source templates without demanding physical instances.
            // Repeated calls must share each user function's canonical template.
            for index in 0..3 {
                let evaluations: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 110 && key.params.as_ref() == [index])
                    .collect();
                assert_eq!(evaluations.len(), 1);
                assert_eq!(evaluations[0].1, 1);
            }
            for &kind in if check {
                &[][..]
            } else {
                &[102u32, 108, 109][..]
            } {
                let evaluations: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(actual, _)| actual.kind == kind)
                    .collect();
                assert_eq!(
                    evaluations.len(),
                    if check { 0 } else { 3 },
                    "physical nodes, kind={kind}"
                );
                assert!(
                    evaluations.iter().all(|(_, count)| *count == 1),
                    "no duplicate evaluation, kind={kind}"
                );
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
fn conflicting_call_arguments_retain_failed_analysis() {
    let source = "fn both(a:&mut i32,b:&i32){}fn main(){let mut x=1;both(&mut x,&x);}";
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
fn unused_bad_reference_callee_blocks_publication() {
    let source = "fn bad(p:&i32){*p=2;}fn main(){println!(\"must not publish\");}";
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
fn unused_elided_reference_return_is_analyzed_and_accepted() {
    let source = "fn escape(p:&i32)->&i32{p}fn main(){}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.analyses.len(), 2);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            let contract = &compiled.control_flow[0].ref_contract;
            assert_eq!(contract.region_count, 1);
            assert_eq!(contract.results.len(), 1);
            assert_eq!(contract.results[0].position.depth, 0);
            assert_eq!(contract.results[0].region, compiler::ir::RegionParamId(0));
            let compiler::ir::ReturnContract::SharedInputPaths(paths) =
                &contract.results[0].returns
            else {
                panic!("expected normalized shared return contract");
            };
            assert_eq!(paths.len(), 1);
            assert_eq!(paths[0].input.param, 0);
            assert_eq!(paths[0].input.position.depth, 0);
            assert!(paths[0].projection.is_empty());
        }
    }
}
