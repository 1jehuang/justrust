use justrust_engine::compiler;

// Frozen oracle P01. These are source bytes, not a second inferred oracle.
const IDENTITY: &str = "fn id<'o,'i,T>(x:&'o &'i T)->&'o &'i T{x}\nfn run()->i32{let v=11;let r=&v;let rr=id(&r);let oracle_observation0=**rr;oracle_observation0}\nfn main(){if run()==11{println!(\"nested references oracle ok\");}}\n";

// Frozen oracle P03 keeps the original referent live after the outer handle dies.
const INNER_ESCAPE: &str = "fn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}\nfn run()->i32{let v=13;let r=&v;let out;{let rr=&r;out=inner(rr);}let oracle_observation0=*out;oracle_observation0}\nfn main(){if run()==13{println!(\"nested references oracle ok\");}}\n";

#[test]
fn nested_identity_retains_both_position_rows_through_the_same_graph() {
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(IDENTITY.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.functions, 3);
            assert_eq!(compiled.control_flow.len(), 3);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            let nested: Vec<_> = compiled
                .control_flow
                .iter()
                .filter(|function| function.ref_contract.results.len() == 2)
                .collect();
            assert_eq!(nested.len(), 1);
            let contract = &nested[0].ref_contract;
            assert_eq!(contract.input_regions.len(), 2);
            for depth in [0, 1] {
                assert_eq!(contract.input_regions[depth].input.param, 0);
                assert_eq!(
                    contract.input_regions[depth].input.position.depth,
                    depth as u8
                );
                assert_eq!(contract.results[depth].position.depth, depth as u8);
                assert_eq!(
                    contract.results[depth].region,
                    contract.input_regions[depth].region
                );
            }
            assert!(contract.outlives.contains(&(
                contract.input_regions[1].region,
                contract.input_regions[0].region,
            )));
            for kind in [102, 108, 109] {
                let nodes: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == kind)
                    .collect();
                assert_eq!(nodes.len(), 3);
                assert!(nodes.iter().all(|(_, count)| *count == 1));
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
            assert_eq!(
                compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == 114)
                    .count(),
                if check { 0 } else { 1 }
            );
        }
    }
}

#[test]
fn inner_value_escape_has_worker_stable_native_bytes_and_positional_origins() {
    let one = compiler::compile(INNER_ESCAPE.to_owned(), 1, false).unwrap();
    let eight = compiler::compile(INNER_ESCAPE.to_owned(), 8, false).unwrap();
    assert_eq!(one.executable, eight.executable);
    for compiled in [&one, &eight] {
        assert!(compiled.report.capture_complete);
        assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        assert!(
            compiled
                .report
                .node_evaluations
                .iter()
                .all(|(_, count)| *count == 1)
        );
        let inner = compiled
            .control_flow
            .iter()
            .find(|function| {
                function.ref_contract.input_regions.len() == 2
                    && function.ref_contract.results.len() == 1
            })
            .unwrap();
        assert_eq!(inner.ref_contract.results[0].position.depth, 0);
        assert_eq!(
            inner.ref_contract.results[0].region,
            inner.ref_contract.input_regions[1].region
        );
    }
}

#[test]
fn dangling_outer_and_unrelated_inner_regions_block_every_object() {
    // Frozen oracle N18 and N26, including unused invalid body coverage.
    let sources = [
        "fn bad<'i>(x:&'i i32)->&'i &'i i32{let r=x;&r}\nfn run()->i32{0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
        "fn bad<'o,'i,'unrelated,T>(x:&'o &'i T)->&'unrelated T{*x}\nfn run()->i32{0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    ];
    for source in sources {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .unwrap();
                let report = failure.report.unwrap();
                assert!(report.capture_complete);
                assert!(
                    !report
                        .node_evaluations
                        .iter()
                        .any(|(key, _)| key.kind == 103)
                );
                assert!(
                    failure
                        .analyses
                        .iter()
                        .any(|(_, analysis)| !analysis.passed())
                );
            }
        }
    }
}
