//! no_std changes source policy, not compiler graph topology or CHECK demands.
use justrust_engine::compiler;
#[allow(dead_code)]
#[path = "support/native_prelude.rs"]
mod native_prelude;
#[test]
fn no_std_check_uses_per_file_source_and_never_demands_codegen() {
    for (workers, test) in [(1, false), (8, false), (1, true), (8, true)] {
        let mut options = compiler::source::SourceOptions {
            library: true,
            ..Default::default()
        };
        if test {
            options.cfg.names.insert("test".into());
        }
        let compilation = compiler::compile_observed_with_options(
            "#![no_std]\npub fn value() -> core::option::Option<i32> { Some(3i32) }\n#[test] fn smoke() { let x = match value() { Some(x) => x, None => 0i32 }; assert_eq!(x, 3i32); }".into(),
            workers,
            true,
            options,
        )
        .unwrap_or_else(|e| panic!("{e}"));
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
