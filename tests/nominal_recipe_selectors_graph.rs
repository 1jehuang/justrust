//! Index inference must remain an ordinary graph consumer, not a layout shortcut.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;

#[test]
fn nominal_index_check_reads_graph_facts_without_codegen() {
    let source = "fn main(){let v=vec![7i32];let i=0usize;let n:i32=v[i];}";
    for workers in [1, 8] {
        let checked = compiler::compile_observed(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        assert!(checked.report.capture_complete);
        assert_eq!(checked.report.failures, 0);
        assert_eq!(checked.report.events_dropped, 0);
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130..=133))
        );
        let declarations = checked
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 138)
            .collect::<Vec<_>>();
        assert!(!declarations.is_empty());
        for (key, count) in declarations {
            assert_eq!(*count, 1);
            assert_eq!(key.params.len(), 2);
        }
    }
}
