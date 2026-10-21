use super::*;

#[test]
fn legacy_root_body_is_only_an_alias_to_one_canonical_producer() {
    for workers in [1, 8] {
        let snapshot = Arc::new(Snapshot {
            input: Input::Text("fn main(){}".into()),
            source_options: source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let body = engine.request(crate::compiler::keys::item(BODY, 0));
        let report = engine.run(workers).unwrap();
        body.completion().unwrap();
        let canonical: Vec<_> = report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == CANONICAL_BODY)
            .collect();
        assert_eq!(canonical.len(), 1);
        assert_eq!(canonical[0].1, 1);
        assert!(!canonical[0].0.params.is_empty());
        assert!(!report.node_evaluations.iter().any(|(key, _)| matches!(
            key.kind,
            MONO_PLAN | BUILD | OBJECT | OPAQUE_REPRESENTATION
        )));
        let alias = engine
            .fact_of::<InstanceBodyFact>(&crate::compiler::keys::item(BODY, 0), 0)
            .unwrap();
        let actual = engine
            .fact_of::<InstanceBodyFact>(&canonical[0].0, 0)
            .unwrap();
        assert!(Arc::ptr_eq(&alias.checked, &actual.checked));
        assert!(Arc::ptr_eq(&alias.facts, &actual.facts));
    }
}
