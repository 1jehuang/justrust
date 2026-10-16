use justrust_engine::{Key, compiler};

#[test]
fn projected_returns_retain_exact_contract_cfg_and_analysis() {
    let source = "struct Pair{left:i32,right:usize}fn wide<'a>(p:&'a mut Pair)->&'a mut usize{&mut (*p).right}fn relay<'a>(p:&'a mut Pair)->&'a mut usize{wide(p)}fn main(){let mut p=Pair{left:1,right:4294967296usize};let r=relay(&mut p);*r=18446744073709551615usize;if p.right==18446744073709551615usize{println!(\"ok\");}}";
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
                let compiler::ir::ReturnContract::MutInputPaths(paths) =
                    &function.ref_contract.returns
                else {
                    panic!("expected canonical projected mutable result");
                };
                assert_eq!(paths.len(), 1);
                assert_eq!(paths[0].param, 0);
                assert_eq!(
                    paths[0].projection.as_ref(),
                    [compiler::ir::Projection::Field(1)]
                );
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
fn projected_result_freezes_original_input_until_last_use() {
    fails_in_retained_analysis(
        "struct Pair{left:i32,right:i32}fn get<'a>(p:&'a mut Pair)->&'a mut i32{&mut (*p).right}fn main(){let mut p=Pair{left:1,right:2};let r=get(&mut p);p.left=3;*r=7;}",
    );
}

#[test]
fn unused_local_projected_return_blocks_publication() {
    fails_in_retained_analysis(
        "struct Pair{x:i32}fn bad<'a>(p:&'a Pair)->&'a i32{let local=Pair{x:7};&local.x}fn main(){println!(\"must not publish\");}",
    );
}

#[test]
fn projected_target_does_not_launder_unrelated_lifetime_dependency() {
    fails_in_retained_analysis(
        "struct Pair{x:i32}fn get<'a>(s:&'a bool,p:&'a Pair)->&'a i32{&(*p).x}fn bad<'a,'b>(s:&'b bool,p:&'a Pair)->&'a i32{get(s,p)}fn main(){}",
    );
}
