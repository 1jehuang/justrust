//! Macro routing reads the same immutable SOURCE facts without a new join.
use justrust_engine::compiler;
#[path = "support/source_macro_binding_oracle.rs"]
mod oracle;

#[test]
fn check_shadowing_has_no_codegen_and_preserves_source_node_keys() {
    for workers in [1, 8] {
        let plain = compiler::compile_observed(
            "fn f()->i32 {23i32} fn main(){assert_eq!(f(),23i32);}".into(),
            workers,
            true,
        )
        .unwrap();
        let shadow = compiler::compile_observed("macro_rules! panic {()=>{23i32};} fn f()->i32 {panic!()} fn main(){assert_eq!(f(),23i32);}".into(), workers, true).unwrap();
        let keys = |result: &compiler::Compilation| {
            let mut keys: Vec<_> = result
                .report
                .node_evaluations
                .iter()
                .filter(|(key, _)| key.kind == 107)
                .map(|(key, count)| {
                    assert_eq!(*count, 1);
                    assert!(!key.params.is_empty());
                    key.params.clone()
                })
                .collect();
            keys.sort();
            keys
        };
        assert_eq!(keys(&plain), keys(&shadow));
        assert!(shadow.executable.is_none());
        assert!(shadow.report.capture_complete);
        assert_eq!(shadow.report.events_dropped, 0);
        assert!(shadow.report.node_evaluations.iter().all(|(key, count)| {
            assert_eq!(*count, 1);
            !matches!(key.kind, 103 | 105 | 130..=133)
        }));
    }
}

#[test]
fn mounted_dependency_source_identity_is_not_a_new_global_barrier() {
    let fixture = oracle::Fixture::new(
        "macro_rules! panic {()=>{23i32};} fn main(){assert_eq!(dep::f(),71i32);}",
        &[],
        Some("pub fn f()->i32{71i32} pub fn unused()->i32 {panic!(\"dependency\")}"),
    );
    for workers in [1, 8] {
        let checked = fixture.native(workers, true).unwrap();
        let source_keys: Vec<_> = checked
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 107)
            .collect();
        assert!(source_keys.iter().any(|(key, _)| key.params == [2]));
        assert!(
            source_keys
                .iter()
                .all(|(key, count)| !key.params.is_empty() && *count == 1)
        );
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| key.kind != 103)
        );
        assert!(checked.executable.is_none());
    }
}
