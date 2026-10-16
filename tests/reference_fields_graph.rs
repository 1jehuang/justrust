use justrust_engine::{Key, compiler};

#[test]
fn implicit_reference_fields_retain_real_deref_and_shared_graph_facts() {
    let source = "struct Pair{left:i32,right:usize}fn get(p:&Pair)->&usize{&p.right}fn wide(p:&mut Pair)->&mut usize{&mut p.right}fn relay(p:&mut Pair)->&mut usize{wide(p)}fn main(){let mut p=Pair{left:1,right:4294967296usize};let r=relay(&mut p);*r=18446744073709551615usize;let s=get(&p);if *s==18446744073709551615usize{println!(\"ok\");}}";
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
                assert_eq!(function.ref_contract.region_count, 1);
                let paths = match &function.ref_contract.returns {
                    compiler::ir::ReturnContract::SharedInputPaths(paths) if index == 0 => paths,
                    compiler::ir::ReturnContract::MutInputPaths(paths) if index != 0 => paths,
                    _ => panic!("expected canonical projected return contract"),
                };
                assert_eq!(paths.len(), 1);
                assert_eq!(paths[0].param, 0);
                assert_eq!(
                    paths[0].projection.as_ref(),
                    [compiler::ir::Projection::Field(1)]
                );
            }
            for function in &compiled.control_flow[..2] {
                assert_eq!(function.loan_sites.len(), 1);
                assert_eq!(
                    function.loan_sites[0].place.projection.as_slice(),
                    [
                        compiler::ir::Projection::Deref,
                        compiler::ir::Projection::Field(1)
                    ]
                );
                assert_eq!(function.loan_sites[0].place.local, function.params[0]);
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
fn projected_mutable_result_keeps_original_parent_frozen() {
    fails_in_retained_analysis(
        "struct Pair{left:i32,right:i32}fn get(p:&mut Pair)->&mut i32{&mut p.right}fn main(){let mut p=Pair{left:1,right:2};let r=get(&mut p);p.left=3;*r=7;}",
    );
}

#[test]
fn unused_implicit_shared_write_body_blocks_publication() {
    fails_in_retained_analysis(
        "struct Pair{x:i32}fn bad(p:&Pair){p.x=7;}fn main(){println!(\"must not publish\");}",
    );
}

#[test]
fn implicit_field_runtime_index_is_a_typed_unsupported_boundary() {
    let source = "struct Data{cells:[i32;2]}fn get(p:&Data,i:usize)->i32{p.cells[i]}fn main(){}";
    for workers in [1, 8] {
        for check in [true, false] {
            let failure = compiler::compile_observed(source.to_owned(), workers, check)
                .err()
                .unwrap();
            assert!(failure.to_string().contains("unsupported"));
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            assert!(
                report
                    .node_evaluations
                    .iter()
                    .any(|(key, _)| key.kind == 110)
            );
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
