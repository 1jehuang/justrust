use justrust_engine::{Key, compiler};

#[test]
fn mutable_returns_share_exact_cfg_and_consuming_analysis() {
    let source = "fn id<'a>(p:&'a mut i32)->&'a mut i32{p}fn forward<'a>(p:&'a mut i32)->&'a mut i32{id(p)}fn main(){let mut x=1;let p=&mut x;let r=forward(p);*r=7;*p=9;if x==9{println!(\"ok\");}}";
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
                    &function.ref_contract.returns,
                    compiler::ir::ReturnContract::MutInputPaths(paths) if paths.len() == 1 && paths[0].param == 0 && paths[0].projection.is_empty()
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

fn fails_in_retained_analysis(source: &str) {
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
fn returned_unique_handle_freezes_parent_until_its_last_use() {
    fails_in_retained_analysis(
        "fn id<'a>(p:&'a mut i32)->&'a mut i32{p}fn main(){let mut x=1;let p=&mut x;let r=id(p);*p=3;*r=7;}",
    );
}

#[test]
fn unused_owned_mutable_return_blocks_executable_publication() {
    fails_in_retained_analysis(
        "fn bad<'a>(p:&'a mut i32)->&'a mut i32{let mut x=7;&mut x}fn main(){println!(\"must not publish\");}",
    );
}

#[test]
fn mutable_return_dependencies_require_declared_universal_outlives() {
    fails_in_retained_analysis(
        "fn pick<'a>(s:&'a mut bool,x:&'a mut i32)->&'a mut i32{x}fn bad<'a,'b>(s:&'b mut bool,x:&'a mut i32)->&'a mut i32{pick(s,x)}fn main(){}",
    );
}
