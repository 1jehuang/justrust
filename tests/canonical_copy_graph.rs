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
    for source in rejected {
        for workers in [1, 8] {
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
