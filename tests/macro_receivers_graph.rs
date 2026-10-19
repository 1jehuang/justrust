//! Receiver authority is carried by existing immutable source and scope facts.
use justrust_engine::compiler;

#[test]
fn receiver_macros_check_without_codegen_or_new_graph_barriers_at_workers_1_8() {
    let sources = [
        "struct S{v:i32}impl S{fn get(&self)->i32{self.v}}fn main(){let s=S{v:42};assert_eq!(s.get(),42);}",
        "struct S{v:i32}impl S{fn get(&self)->i32{macro_rules! get{()=>{self.v};}get!()}}fn main(){let s=S{v:42};assert_eq!(s.get(),42);}",
    ];
    for workers in [1, 8] {
        let mut baseline = None;
        for source in sources {
            let checked = compiler::compile_observed(source.into(), workers, true).unwrap();
            assert!(checked.executable.is_none());
            assert!(checked.report.capture_complete);
            assert_eq!(checked.report.events_dropped, 0);
            assert!(
                checked
                    .report
                    .node_evaluations
                    .iter()
                    .all(|(key, count)| *count == 1 && !matches!(key.kind, 103 | 105 | 130..=133))
            );
            let mut keys = checked
                .report
                .node_evaluations
                .iter()
                .map(|(key, _)| (key.kind, key.params.clone()))
                .collect::<Vec<_>>();
            keys.sort();
            if let Some(expected) = &baseline {
                assert_eq!(&keys, expected);
            } else {
                baseline = Some(keys);
            }
        }
    }
}
