use justrust_engine::compiler;

// Frozen canonical oracle P10. The returned inner reference outlives its handle.
const INNER: &str = "struct Opaque{v:i32}fn run()->i32{let owner=Opaque{v:11};let result;{let r=&owner;result=<&Opaque as core::clone::Clone>::clone(&r);}result.v}\nfn main(){if run()==11{println!(\"canonical copy oracle ok\");}}\n";

#[test]
fn canonical_recipe_universal_proofs_and_inner_origins_precede_every_object() {
    use compiler::frontend::{self, CallableProvider, PlanCtx};
    use compiler::ir::{PlanBudget, ReturnContract};
    use justrust_engine::engine::Key;

    let program = frontend::parse(INNER).unwrap();
    let context = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
    let mut recipes = Vec::new();
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
                sources += 1;
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
        for index in &recipes {
            for kind in [110, 111, 112] {
                assert!(compiled.report.node_evaluations.iter().any(|(key, count)| {
                    key.kind == kind && key.params == [*index as u64] && *count == 1
                }));
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
        let barrier_key = format!("{:?}", Key::new(114, vec![]));
        let barrier = compiled
            .report
            .events
            .iter()
            .find(|event| event.kind == "discovered" && event.detail == barrier_key)
            .unwrap()
            .node
            .unwrap();
        let published = compiled
            .report
            .events
            .iter()
            .find(|event| event.kind == "publish" && event.node == Some(barrier))
            .unwrap()
            .at_ns;
        for index in 0..compiled.control_flow.len() {
            let object_key = format!("{:?}", Key::new(103, vec![index as u64]));
            let object = compiled
                .report
                .events
                .iter()
                .find(|event| event.kind == "discovered" && event.detail == object_key)
                .unwrap();
            assert!(object.at_ns >= published);
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
                use compiler::frontend::{CallableProvider, PlanCtx};
                use compiler::ir::PlanBudget;
                use justrust_engine::Key;
                for check in [true, false] {
                    let compiled = compiler::compile(source.to_owned(), workers, check).unwrap();
                    assert_eq!(compiled.functions, 2);
                    // Two source bodies and one demanded shared-reference Clone recipe.
                    assert_eq!(compiled.control_flow.len(), 3);
                    assert_eq!(compiled.analyses.len(), 3);
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
                        Key::new(107, vec![]), // Shared configured text/file SOURCE.
                        Key::new(100, vec![]),
                        Key::new(106, vec![]),
                        Key::new(101, vec![]),
                        Key::new(104, vec![]),
                        Key::new(113, vec![]),
                        Key::new(115, vec![]),
                    ];
                    for index in 0..3 {
                        for kind in [102, 108, 109] {
                            expected.push(Key::new(kind, vec![index]));
                        }
                        if !check {
                            expected.push(Key::new(103, vec![index]));
                        }
                    }
                    if !check {
                        expected.push(Key::new(105, vec![]));
                        expected.push(Key::new(114, vec![]));
                    }
                    let program = compiler::frontend::parse(source).unwrap();
                    let context = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
                    for index in 0..context.callable_count() {
                        let declaration = context.callable_decl(index).unwrap();
                        expected.push(Key::new(110, vec![index as u64]));
                        match context.callable_provider(declaration).unwrap() {
                            CallableProvider::Source { .. } => {
                                assert!(index < compiled.functions);
                            }
                            CallableProvider::Recipe { .. } => {
                                assert!(index >= compiled.functions);
                                for kind in [111, 112] {
                                    expected.push(Key::new(kind, vec![index as u64]));
                                }
                            }
                            CallableProvider::UnavailableProvided { .. } => {
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
                    let effects = published(discovered(&Key::new(115, vec![])).node.unwrap());
                    assert!(published(discovered(&Key::new(113, vec![])).node.unwrap()) >= effects);
                    for (key, _) in &report.node_evaluations {
                        let event = discovered(key);
                        if matches!(key.kind, 110 | 112) {
                            assert!(published(event.node.unwrap()) <= effects);
                        }
                        if matches!(key.kind, 102 | 108 | 109 | 103) {
                            assert!(event.at_ns >= effects);
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
                        let sealed = published(discovered(&Key::new(114, vec![])).node.unwrap());
                        for (key, _) in &report.node_evaluations {
                            if key.kind == 103 {
                                assert!(discovered(key).at_ns >= sealed);
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
            assert!(
                !report
                    .node_evaluations
                    .iter()
                    .any(|(key, _)| key.kind == 103)
            );
        }
    }
    let override_source = "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}fn clone_from(&mut self,source:&Self){self.v=source.v+10;}}fn run()->i32{let mut destination=S{v:1};let source=S{v:2};destination.clone_from(&source);destination.v}\nfn main(){if run()==12{println!(\"canonical copy oracle ok\");}}\n";
    for workers in [1, 8] {
        let compiled = compiler::compile(override_source.to_owned(), workers, false).unwrap();
        assert!(compiled.executable.is_some());
        assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
    }
}
