#[allow(dead_code)]
#[path = "support/native_prelude.rs"]
mod native_prelude;

use justrust_engine::compiler;

#[test]
fn generic_instances_reuse_facts_and_check_has_no_objects() {
    let source = "fn id<T>(x:T)->T{x}fn main(){let a=id::<u8>(7u8);let b=id::<u8>(9u8);let c=id::<u64>(18446744073709551615u64);if a==7u8&&b==9u8&&c==18446744073709551615u64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            // CHECK proves generic bodies universally and never demands
            // per-instance BODY/LOWER/ANALYZE (stock `--emit=metadata` does
            // not monomorphize). BUILD reaches the 3 closed instances
            // (main, id::<u8>, id::<u64>) through INSTANCE (142).
            let instances = if check { 0 } else { 3 };
            assert_eq!(compiled.control_flow.len(), if check { 2 } else { 3 });
            assert_eq!(compiled.analyses.len(), if check { 2 } else { 3 });
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for kind in [102, 108, 109, 142] {
                let facts: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == kind)
                    .collect();
                assert_eq!(facts.len(), instances);
                assert!(facts.iter().all(|(_, count)| *count == 1));
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
fn unused_generic_moves_are_checked_universally_before_native_publication() {
    for source in [
        "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){println!(\"must not publish\");}",
        "fn bad<T>(x:T)->(T,T){(x,x)}fn main(){let p=bad::<u64>(7u64);}",
    ] {
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
                assert!(!report.published_kind(105));
            }
        }
    }
}

#[test]
fn unused_generic_types_and_nominal_applications_cannot_use_concrete_witnesses() {
    for (case, source) in [
        "fn bad<T>(x:T)->T{x+1}fn main(){}",
        "struct Wrap<T>{value:T}fn bad<T>(x:Wrap<T>)->Wrap<u64>{x}fn main(){}",
        "fn bad<T>(x:T)->u8{x as u8}fn main(){}",
        "fn bad<T>(x:T)->T{x}fn main(){let x=7u8;let y=bad::<&u8>(&x);}",
    ]
    .into_iter()
    .enumerate()
    {
        for workers in [1, 8] {
            for check in [true, false] {
                // The unchanged fourth source now uses the admitted explicit root
                // reference actual. Its identity body still needs universal proof.
                if case == 3 {
                    use justrust_engine::Key;
                    let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
                    assert_eq!(compiled.functions, 2);
                    assert_eq!(compiled.control_flow.len(), 2);
                    assert_eq!(compiled.analyses.len(), 2);
                    assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
                    assert_eq!(compiled.executable.is_none(), check);
                    if let Some(executable) = &compiled.executable {
                        assert!(executable.len() >= 64, "truncated native executable");
                        assert_eq!(&executable[..6], b"\x7fELF\x02\x01");
                    }
                    let report = &compiled.report;
                    assert!(report.capture_complete);
                    assert_eq!(report.events_dropped, 0);
                    assert_eq!(report.failures, 0);
                    // Per-instance planning (no MONO_PLAN 113): CHECK evaluates no
                    // instance-keyed node; BUILD reaches exactly main and
                    // bad::<&u8> through one INSTANCE (142) each, with one
                    // BODY/LOWER/ANALYZE/OBJECT per instance. Every node runs once.
                    assert!(
                        !report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| key.kind == 113)
                    );
                    assert!(report.node_evaluations.iter().all(|(_, count)| *count == 1));
                    for kind in [102, 108, 109, 142] {
                        let count = report
                            .node_evaluations
                            .iter()
                            .filter(|(key, _)| key.kind == kind)
                            .count();
                        assert_eq!(count, if check { 0 } else { 2 }, "kind {kind}");
                    }
                    for kind in [100, 101, 104, 106] {
                        let root: Vec<_> = report
                            .node_evaluations
                            .iter()
                            .filter(|(key, _)| key.kind == kind)
                            .collect();
                        assert_eq!(root.len(), 1);
                        assert_eq!(root[0].0, Key::new(kind, vec![]));
                    }
                    assert_eq!(
                        report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| *key == Key::new(105, vec![])),
                        !check
                    );
                    let discovered = |key: &Key| {
                        let spelling = format!("{key:?}");
                        report
                            .events
                            .iter()
                            .find(|event| event.kind == "discovered" && event.detail == spelling)
                            .expect("required graph node was not discovered")
                    };
                    let published = |node| {
                        report
                            .events
                            .iter()
                            .find(|event| event.kind == "publish" && event.node == Some(node))
                            .expect("required graph node did not publish")
                            .at_ns
                    };
                    let effects = report
                        .node_evaluations
                        .iter()
                        .filter(|(key, _)| key.kind == 115)
                        .map(|(key, _)| published(discovered(key).node.unwrap()))
                        .max()
                        .expect("required universal admission fact missing");
                    for (key, _) in &report.node_evaluations {
                        let event = discovered(key);
                        if matches!(key.kind, 110 | 112) {
                            assert!(published(event.node.unwrap()) <= effects);
                        }
                    }
                    if check {
                        assert!(
                            !report
                                .node_evaluations
                                .iter()
                                .any(|(key, _)| key.kind == 103)
                        );
                    } else {
                        // Each object follows its own instance proof (no global barrier).
                        for (key, _) in &report.node_evaluations {
                            if key.kind == 103 {
                                let proof = Key::new(109, key.params.clone());
                                let proved = published(discovered(&proof).node.unwrap());
                                assert!(published(discovered(key).node.unwrap()) >= proved);
                            }
                        }
                    }
                    continue;
                }
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .unwrap();
                let report = failure.report.unwrap();
                assert!(report.capture_complete);
                assert!(!report.published_kind(105));
            }
        }
    }
}

#[test]
fn exact_generic_recursion_deduplicates_but_growing_instances_fail_boundedly() {
    let exact = "fn relay<T>(x:T,n:i32)->T{if n==0{x}else{relay::<T>(x,n-1)}}fn main(){let x=relay::<u64>(18446744073709551615u64,3);if x==18446744073709551615u64{println!(\"ok\");}}";
    let growing = "fn grow<T>(x:T,n:i32)->i32{if n==0{7}else{grow::<(T,)>((x,),n-1)}}fn main(){let x=grow::<u8>(1u8,1);}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(exact.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.control_flow.len(), 2);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            // Stock rustc accepts the growing chain at check and rejects it
            // only while instantiating for codegen.
            if check {
                assert!(compiler::compile(growing.to_owned(), workers, check).is_ok());
                continue;
            }
            let failure = compiler::compile_observed(growing.to_owned(), workers, check)
                .err()
                .unwrap();
            let message = failure.error.to_string().to_lowercase();
            assert!(
                message.contains("generic") && message.contains("limit"),
                "{message}"
            );
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            assert!(!report.published_kind(105));
        }
    }
}
