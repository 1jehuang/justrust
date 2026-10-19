//! Capability keys are canonical and cannot authorize stronger closure modes.
use super::*;

#[test]
fn closure_capability_keys_reject_primary_alias_and_unsupported_mode() {
    for workers in [1, 8] {
        for (source, mode, diagnostic) in [
            (
                "fn main(){let f=|x:i32|x+1;}",
                0,
                "noncanonical primary closure capability key",
            ),
            (
                "struct Item{i:i32}fn main(){let x=Item{i:7};let f=move ||x;}",
                0,
                "capability",
            ),
        ] {
            let snapshot = Arc::new(Snapshot {
                input: Input::Text(source.into()),
                source_options: source::SourceOptions::default(),
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(CompilerProvider(snapshot.clone()));
            let signatures = engine.request(Key::new(SIGNATURES, vec![]));
            engine.run(workers).unwrap();
            signatures.completion().unwrap();
            let types = signatures.fact::<frontend::PlanCtx>(0).unwrap();
            let index = (0..types.callable_count())
                .find(|index| {
                    types
                        .stored_closure_owner(types.callable_decl(*index).unwrap())
                        .unwrap()
                        .is_some()
                })
                .unwrap();
            let engine = Engine::new(CompilerProvider(snapshot));
            let invalid = engine.request(Key::new(TEMPLATE_BODY, vec![index as u64, mode]));
            let _ = engine.run(workers);
            let error = invalid
                .completion()
                .expect_err("invalid mode key must fail");
            assert!(error.to_string().contains(diagnostic), "{error}");
        }
    }
}
