//! Canonical conversions are ordinary per-file source and per-item graph work.
use justrust_engine::compiler;
#[allow(dead_code)]
#[path = "support/native_prelude.rs"]
mod native_prelude;

#[test]
fn check_uses_embedded_source_without_demanding_native_codegen() {
    for workers in [1, 8] {
        let compilation = compiler::compile_observed(
            "fn main(){assert_eq!(char::from(255u8),'ÿ');}".into(),
            workers,
            true,
        )
        .unwrap();
        assert!(compilation.report.capture_complete);
        assert!(compilation.executable.is_none());
        assert!(
            compilation
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130..=133))
        );
        for source in native_prelude::library_source_keys() {
            assert!(
                compilation
                    .report
                    .node_evaluations
                    .iter()
                    .any(|(key, count)| key == &source && *count == 1)
            );
        }
    }
}
