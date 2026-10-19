//! Forged invocation operands and source occurrences must fail canonical publication.
use super::*;

fn invocation_publication() -> (ir::PlanBudget, Arc<sr::CanonicalSourcePublication>) {
    use crate::compiler::{CompilerProvider, Input, RESOLVE, SIGNATURES, Snapshot};
    use crate::engine::{Engine, Key};
    let budget = ir::PlanBudget::default();
    let snapshot = Arc::new(Snapshot {
        input: Input::Text("fn invoke<F:Fn(i32)->i32>(f:F)->i32{(f)(7)}fn main(){}".into()),
        source_options: crate::compiler::source::SourceOptions::default(),
        budget: budget.clone(),
    });
    let engine = Engine::new(CompilerProvider(snapshot));
    let signatures = engine.request(Key::new(SIGNATURES, vec![]));
    let resolved = engine.request(Key::new(RESOLVE, vec![]));
    engine.run(2).unwrap();
    signatures.completion().unwrap();
    resolved.completion().unwrap();
    let plan = signatures.fact::<PlanCtx>(0).unwrap();
    let program = resolved.fact::<Arc<Program>>(0).unwrap();
    let index = program
        .functions
        .iter()
        .position(|function| {
            function
                .name
                .segments()
                .last()
                .is_some_and(|name| program.names.spelling(*name).unwrap() == "invoke")
        })
        .unwrap();
    let declaration = plan.function_decl(index).unwrap();
    let PublishedCallableBody::Source(publication) =
        plan.publish_callable_body(declaration, &budget).unwrap()
    else {
        panic!("original source publication required")
    };
    (budget, publication)
}

#[test]
fn invocation_operands_and_foreign_source_cannot_replace_canonical_publication() {
    let (budget, publication) = invocation_publication();
    let source = publication.source();
    source
        .validate_with_facts(ir::FactsCtx::Validated(publication.facts()))
        .unwrap();
    publication
        .validate_source(source, publication.facts(), &budget)
        .unwrap();
    let call = source
        .exprs
        .iter()
        .position(|expr| matches!(expr.kind, ir::ExprKind::Call { .. }))
        .unwrap();
    let ir::ExprKind::Call { args, .. } = &source.exprs[call].kind else {
        unreachable!()
    };
    assert_eq!(args.len(), 2, "receiver and actual tuple remain distinct");
    for mode in 0..3 {
        let mut changed = source.as_ref().clone();
        let ir::ExprKind::Call { args, .. } = &mut changed.exprs[call].kind else {
            unreachable!()
        };
        match mode {
            0 => args[0] = args[1],
            1 => args[1] = args[0],
            _ => {
                args.pop();
            }
        }
        assert!(
            changed
                .validate_with_facts(ir::FactsCtx::Validated(publication.facts()))
                .is_err(),
            "forged invocation operand mode {mode}"
        );
        assert!(
            publication
                .validate_source(&Arc::new(changed), publication.facts(), &budget)
                .is_err()
        );
    }
    let (_, foreign) = invocation_publication();
    assert!(
        publication
            .validate_source(foreign.source(), publication.facts(), &budget)
            .is_err()
    );
    let mut original = publication.original().source_for_test();
    let foreign_original = foreign.original().source_for_test();
    assert!(!original.calls.is_empty());
    original.calls[0].node = foreign_original.calls[0].node.clone();
    let error = replay_source_body(&original, source, publication.facts(), &budget).unwrap_err();
    assert_eq!(error.kind(), tr::FactsErrorKind::InvalidMetadata);
}
