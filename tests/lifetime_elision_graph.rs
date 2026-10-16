use justrust_engine::{Key, compiler};

#[test]
fn elided_projected_returns_share_canonical_cfg_and_analysis() {
    let source = "struct Pair{left:i32,right:usize}fn get(p:&Pair)->&usize{&(*p).right}fn wide(p:&mut Pair)->&mut usize{&mut (*p).right}fn relay(p:&mut Pair)->&mut usize{wide(p)}fn main(){let mut p=Pair{left:1,right:4294967296usize};let r=relay(&mut p);*r=18446744073709551615usize;let s=get(&p);if *s==18446744073709551615usize{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.functions, 4);
            assert_eq!(compiled.control_flow.len(), 4);
            assert_eq!(compiled.analyses.len(), 4);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for (index, function) in compiled.control_flow[..3].iter().enumerate() {
                let contract = &function.ref_contract;
                assert_eq!(contract.region_count, 1);
                assert_eq!(
                    contract.input_regions.as_ref(),
                    [compiler::ir::InputRegion {
                        input: compiler::ir::InputReference {
                            param: 0,
                            position: compiler::ir::ReferencePosition { depth: 0 },
                        },
                        region: compiler::ir::RegionParamId(0),
                    }]
                );
                assert_eq!(contract.results.len(), 1);
                assert_eq!(contract.results[0].position.depth, 0);
                assert_eq!(contract.results[0].region, compiler::ir::RegionParamId(0));
                assert!(contract.outlives.is_empty());
                let paths = match &contract.results[0].returns {
                    compiler::ir::ReturnContract::SharedInputPaths(paths) if index == 0 => paths,
                    compiler::ir::ReturnContract::MutInputPaths(paths) if index != 0 => paths,
                    _ => panic!("expected normalized shared/mutable projected contract"),
                };
                assert_eq!(paths.len(), 1);
                assert_eq!(paths[0].input.param, 0);
                assert_eq!(paths[0].input.position.depth, 0);
                assert_eq!(
                    paths[0].projection.as_ref(),
                    [compiler::ir::Projection::Field(1)]
                );
            }
            for index in 0..4 {
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
                if check { 0 } else { 4 }
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
fn named_output_cannot_infer_anonymous_input_outlives_bound() {
    fails_in_retained_analysis(
        "fn bad<'a>(named:&'a i32,anonymous:&i32)->&'a i32{anonymous}fn main(){}",
    );
}

#[test]
fn unused_elided_local_return_blocks_publication() {
    fails_in_retained_analysis(
        "struct Pair{x:i32}fn bad(p:&Pair)->&i32{let local=Pair{x:7};&local.x}fn main(){println!(\"must not publish\");}",
    );
}

#[test]
fn elided_return_preserves_dependent_storage_lifetime() {
    fails_in_retained_analysis(
        "fn id(p:&i32)->&i32{p}fn main(){let r:&i32;{let local=7;r=id(&local);}let value=*r;}",
    );
}
