use justrust_engine::compiler;

#[test]
fn generic_instances_reuse_facts_and_check_has_no_objects() {
    let source = "fn id<T>(x:T)->T{x}fn main(){let a=id::<u8>(7u8);let b=id::<u8>(9u8);let c=id::<u64>(18446744073709551615u64);if a==7u8&&b==9u8&&c==18446744073709551615u64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.executable.is_none(), check);
            assert_eq!(compiled.control_flow.len(), 3);
            assert_eq!(compiled.analyses.len(), 3);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            for kind in [102, 108, 109] {
                let facts: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == kind)
                    .collect();
                assert_eq!(facts.len(), 3);
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
                    let mut expected = vec![
                        Key::new(107, vec![0]), // Root source file unit.
                        Key::new(100, vec![]),
                        Key::new(106, vec![]),
                        Key::new(101, vec![]),
                        Key::new(104, vec![]),
                        Key::new(113, vec![]),
                        Key::new(111, vec![0]),
                        Key::new(112, vec![0]),
                        Key::new(115, vec![0]),
                    ];
                    for index in 0..2 {
                        for kind in [110, 102, 108, 109] {
                            expected.push(Key::new(kind, vec![index]));
                        }
                        if !check {
                            expected.push(Key::new(103, vec![index]));
                        }
                    }
                    if !check {
                        expected.push(Key::new(105, vec![]));
                    }
                    let program = compiler::frontend::parse(source).unwrap();
                    let plan = compiler::frontend::PlanCtx::from_program(
                        &program,
                        &compiler::ir::PlanBudget::default(),
                    )
                    .unwrap();
                    for index in 0..plan.callable_count() {
                        let declaration = plan.callable_decl(index).unwrap();
                        match plan.callable_provider(declaration).unwrap() {
                            compiler::frontend::CallableProvider::Source { .. } => {
                                assert!(index < compiled.functions);
                                if plan.callable_env(declaration).unwrap().is_generic()
                                    && index != 0
                                {
                                    for kind in [111, 112, 115] {
                                        expected.push(Key::new(kind, vec![index as u64]));
                                    }
                                }
                            }
                            compiler::frontend::CallableProvider::Recipe { .. } => {
                                assert!(index >= compiled.functions);
                                for kind in [110, 111, 112, 115] {
                                    expected.push(Key::new(kind, vec![index as u64]));
                                }
                            }
                            compiler::frontend::CallableProvider::UnavailableProvided {
                                ..
                            } => {
                                panic!("unavailable provider entered body enumeration")
                            }
                        }
                    }
                    assert_eq!(report.node_evaluations.len(), expected.len());
                    for key in expected {
                        let entries: Vec<_> = report
                            .node_evaluations
                            .iter()
                            .filter(|(seen, _)| *seen == key)
                            .collect();
                        assert_eq!(entries.len(), 1, "missing or duplicated {key:?}");
                        assert_eq!(entries[0].1, 1, "shared operation repeated {key:?}");
                    }
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
