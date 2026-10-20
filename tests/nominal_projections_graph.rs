//! Declaration-local projection checks must be graph facts even for unused types.
use justrust_engine::compiler;

#[test]
fn unused_nominal_membership_is_per_declaration_without_codegen() {
    let source = "trait Base<A,B>{type Item;}trait Sub<A,B>:Base<A,B>{}struct Direct<T:Base<i32,bool>>{value:<T as Base<i32,bool>>::Item}enum Inherited<T:Sub<i32,bool>>{Value(<T as Base<i32,bool>>::Item),Empty}fn main(){}";
    for workers in [1, 8] {
        let checked = compiler::compile_observed(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        assert!(checked.report.capture_complete);
        assert_eq!(checked.report.events_dropped, 0);
        let declarations = checked
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 138)
            .collect::<Vec<_>>();
        assert!(declarations.len() >= 2);
        for (key, count) in declarations {
            assert_eq!(*count, 1);
            assert_eq!(key.params.len(), 2);
        }
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130..=133))
        );
    }
}
