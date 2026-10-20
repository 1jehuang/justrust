//! Pure packed-query authority uses the real CHECK graph, never native codegen.
use justrust_engine::{Key, compiler};
#[allow(dead_code)]
#[path = "support/native_prelude.rs"]
mod native_prelude;

#[test]
fn packed_query_check_demands_genuine_embedded_helper_without_codegen() {
    let text = "fn main(){}";
    // Derive the expected item key from the same actual embedded loader/parser,
    // rather than assuming a stable global function index or minting source.
    let program = native_prelude::parse(text).unwrap();
    let source_index = program
        .functions
        .iter()
        .position(|function| {
            function
                .name
                .segments()
                .last()
                .is_some_and(|name| program.names.spelling(*name).unwrap() == "packed_reserve")
        })
        .expect("real embedded packed helper is present");
    let budget = compiler::ir::PlanBudget::default();
    let plan = compiler::frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let provider_index = (0..plan.callable_count())
        .find(|index| {
            let declaration = plan.callable_decl(*index).unwrap();
            let info = plan.declaration(declaration).unwrap();
            info.kind == compiler::ir::DeclKind::Function && info.source_index == source_index
        })
        .unwrap();
    let helper_template = Key::new(110, vec![provider_index as u64]);
    for workers in [1, 8] {
        // compile_observed constructs the production Engine/CompilerProvider,
        // with its actual embedded Source reader and CHECK root.
        let compilation = compiler::compile_observed(text.into(), workers, true)
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(compilation.report.capture_complete);
        assert!(compilation.executable.is_none());
        assert!(
            compilation
                .report
                .node_evaluations
                .iter()
                .any(|(key, count)| { key == &Key::new(104, vec![]) && *count == 1 })
        );
        assert!(
            compilation
                .report
                .node_evaluations
                .iter()
                .any(|(key, count)| { key == &helper_template && *count == 1 }),
            "CHECK must validate the unused genuine packed helper body"
        );
        assert!(
            compilation
                .report
                .node_evaluations
                .iter()
                .any(|(key, count)| {
                    key == &Key::new(128, vec![provider_index as u64]) && *count == 1
                }),
            "CHECK must admit the genuine helper through SOURCE_ADMISSION"
        );
        assert!(
            compilation
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| { !matches!(key.kind, 103 | 105 | 113 | 130..=133) }),
            "CHECK must not demand OBJECT, BUILD or optimizer/image work"
        );
        for source in native_prelude::library_source_keys() {
            assert!(
                compilation
                    .report
                    .node_evaluations
                    .iter()
                    .any(|(key, count)| { key == &source && *count == 1 })
            );
        }
    }
}
