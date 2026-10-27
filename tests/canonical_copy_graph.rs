#[path = "support/native_prelude.rs"]
mod native_prelude;

use justrust_engine::compiler;

// Frozen canonical oracle P10. The returned inner reference outlives its handle.
const INNER: &str = "struct Opaque{v:i32}fn run()->i32{let owner=Opaque{v:11};let result;{let r=&owner;result=<&Opaque as core::clone::Clone>::clone(&r);}result.v}\nfn main(){if run()==11{println!(\"canonical copy oracle ok\");}}\n";

#[test]
fn canonical_recipe_universal_proofs_and_inner_origins_precede_every_object() {
    use compiler::frontend::{CallableProvider, PlanCtx};
    use compiler::ir::{PlanBudget, ReturnContract};
    use justrust_engine::engine::Key;

    let program = native_prelude::parse(INNER).unwrap();
    let context = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
    let mut recipes = Vec::new();
    let mut library = Vec::new();
    let mut sources = 0;
    for index in 0..context.callable_count() {
        match context
            .callable_provider(context.callable_decl(index).unwrap())
            .unwrap()
        {
            CallableProvider::Source { .. } => {
                assert!(
                    recipes.is_empty(),
                    "source declarations precede recipe suffix"
                );
                if context
                    .is_prelude_decl(context.callable_decl(index).unwrap())
                    .unwrap()
                {
                    library.push(index);
                } else {
                    sources += 1;
                }
            }
            CallableProvider::Recipe { .. } => recipes.push(index),
            CallableProvider::UnavailableProvided { .. } => {
                panic!("unavailable provider entered executable enumeration")
            }
        }
    }
    assert!(!recipes.is_empty());
    let one = compiler::compile(INNER.to_owned(), 1, false).unwrap();
    let eight = compiler::compile(INNER.to_owned(), 8, false).unwrap();
    assert_eq!(one.executable, eight.executable);
    for compiled in [&one, &eight] {
        assert_eq!(compiled.functions, sources);
        assert!(compiled.report.capture_complete);
        assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        for index in recipes.iter().chain(&library) {
            let declaration = context.callable_decl(*index).unwrap();
            let universal = match context.callable_provider(declaration).unwrap() {
                CallableProvider::Source { .. } => {
                    context.callable_env(declaration).unwrap().is_generic()
                }
                CallableProvider::Recipe { .. } => true,
                CallableProvider::UnavailableProvided { .. } => {
                    panic!("unavailable provider entered proof enumeration")
                }
            };
            // Every library declaration has a schema. Only generic source
            // bodies and recipes have universal proofs; nongeneric library
            // bodies are checked through their instances when demanded.
            for kind in [110, 111, 112] {
                let evaluations: Vec<_> = compiled
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| key.kind == kind && key.params == [*index as u64])
                    .collect();
                // Library templates and their universal proofs are demand-driven
                // (no whole-program plan reads every template): never
                // duplicated, absent when unreached.
                if kind == 110 || universal {
                    assert!(evaluations.len() <= 1, "duplicated {kind}:{index}");
                    assert!(
                        evaluations.iter().all(|e| e.1 == 1),
                        "repeated {kind}:{index}"
                    );
                } else {
                    assert!(
                        evaluations.is_empty(),
                        "unexpected universal {kind}:{index}"
                    );
                }
            }
        }
        assert!(compiled.control_flow.len() < context.callable_count());
        assert!(compiled.control_flow.iter().any(|body| {
            body.ref_contract
                .results
                .iter()
                .any(|result| match &result.returns {
                    ReturnContract::SharedInputEnvelopes(inputs) => inputs
                        .iter()
                        .any(|input| input.input.param == 0 && input.input.position.depth == 1),
                    ReturnContract::SharedInputPaths(inputs) => inputs
                        .iter()
                        .any(|input| input.input.param == 0 && input.input.position.depth == 1),
                    _ => false,
                })
        }));
        // Every object follows its own instance proof (no global barrier).
        let publish_of = |key: Key| {
            let spelling = format!("{key:?}");
            let node = compiled
                .report
                .events
                .iter()
                .find(|event| event.kind == "discovered" && event.detail == spelling)
                .unwrap()
                .node
                .unwrap();
            compiled
                .report
                .events
                .iter()
                .find(|event| event.kind == "publish" && event.node == Some(node))
                .unwrap()
                .at_ns
        };
        // Instances are keyed by root slot or canonical target words.
        let objects: Vec<_> = compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 103)
            .map(|(key, _)| key.params.clone())
            .collect();
        assert_eq!(objects.len(), compiled.control_flow.len());
        for params in objects {
            let object = publish_of(Key::new(103, params.clone()));
            assert!(object >= publish_of(Key::new(109, params)));
        }
    }
}

#[test]
fn invalid_unused_clone_and_unavailable_dispatch_never_reach_objects() {
    // Frozen N21, X30, X31 and P13, unchanged source bytes.
    let rejected = [
        "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{*self}}fn run()->i32{0}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
        "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}fn run()->i32{let mut destination=S{v:1};let source=S{v:2};destination.clone_from(&source);destination.v}\nfn main(){if run()==2{println!(\"canonical copy oracle ok\");}}\n",
        "fn run()->i32{let v=22;let r=&v;let rr=&r;let cloned=<&&i32 as core::clone::Clone>::clone(&rr);**cloned}\nfn main(){if run()==22{println!(\"canonical copy oracle ok\");}}\n",
    ];
    for (case, source) in rejected.into_iter().enumerate() {
        for workers in [1, 8] {
            // X31's exact source now has independent native/stock parity evidence.
            // N21 and the unavailable clone_from default remain negative below.
            if case == 2 {
                use justrust_engine::Key;
                for check in [true, false] {
                    let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
                    assert_eq!(compiled.functions, 2);
                    // CHECK checks the two source bodies only. Instances and the
                    // shared-reference Clone recipe are admitted on demand by
                    // BUILD (471ca7b45), which adds that recipe's body.
                    let bodies = if check { 2 } else { 3 };
                    assert_eq!(compiled.control_flow.len(), bodies);
                    assert_eq!(compiled.analyses.len(), bodies);
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
                    // No whole-program MONO_PLAN and no repeated shared operation.
                    assert!(
                        !report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| key.kind == 113)
                    );
                    for (key, runs) in &report.node_evaluations {
                        assert_eq!(*runs, 1, "shared operation repeated {key:?}");
                    }
                    // CHECK admits source bodies without BODY/LOWER/ANALYZE.
                    for index in 0..if check { 0 } else { 2 } {
                        for &kind in if check {
                            &[][..]
                        } else {
                            &[102u32, 108, 109][..]
                        } {
                            let key = Key::new(kind, vec![index]);
                            assert!(
                                report.node_evaluations.iter().any(|(seen, _)| *seen == key),
                                "missing {key:?}"
                            );
                        }
                    }
                    // The demanded Clone recipe is one canonical instance body.
                    let canonical = report
                        .node_evaluations
                        .iter()
                        .filter(|(key, _)| key.kind == 126)
                        .count();
                    assert!(if check {
                        canonical == 0
                    } else {
                        canonical >= 1
                    });
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
                }
                continue;
            }
            let failure = compiler::compile_observed(source.to_owned(), workers, false)
                .err()
                .unwrap();
            let report = failure.report.unwrap();
            assert!(report.capture_complete);
            assert!(!report.published_kind(105));
        }
    }
    let override_source = "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}fn clone_from(&mut self,source:&Self){self.v=source.v+10;}}fn run()->i32{let mut destination=S{v:1};let source=S{v:2};destination.clone_from(&source);destination.v}\nfn main(){if run()==12{println!(\"canonical copy oracle ok\");}}\n";
    for workers in [1, 8] {
        let compiled = compiler::compile(override_source.to_owned(), workers, false).unwrap();
        assert!(compiled.executable.is_some());
        assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
    }
}
