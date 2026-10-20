//! Parameter destructuring remains inside the parent-sealed checked body.
use super::*;

#[test]
fn parameter_pattern_authority_replays_binders_and_deref_projections() {
    use crate::compiler::frontend::PublishedCallableBody;
    let program = parse("fn main(){let f=|&&x:&&i32|x+1;}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let retained = plan
        .registry
        .program
        .closure_sources
        .values()
        .find(|source| source.stored.is_some())
        .unwrap();
    plan.registry.closure_source_owner(retained).unwrap();
    let mut forged_source = retained.clone();
    forged_source.stored.as_mut().unwrap().params[0].0 = Pattern::Wildcard;
    assert!(plan.registry.closure_source_owner(&forged_source).is_err());
    let owner = (0..plan.callable_count())
        .find_map(|index| {
            let provider = plan.callable_decl(index).ok()?;
            plan.stored_closure_owner(provider)
                .ok()
                .flatten()
                .map(|pair| pair.0)
        })
        .unwrap();
    let prepared = plan.prepare_body_constants(owner, &budget).unwrap();
    let constants = plan
        .seal_body_constants(prepared, Vec::new(), &budget)
        .unwrap();
    let (publication, closures) = plan
        .publish_callable_body_with_closures(owner, constants, &budget)
        .unwrap();
    let PublishedCallableBody::Source(publication) = publication else {
        panic!("expected source publication")
    };
    let proof =
        publication::ClosurePublicationProof::capture(&plan, &publication, &closures, &budget)
            .unwrap();
    let checked = closures.first().unwrap();
    proof
        .validate(&plan, &publication, checked, &budget)
        .unwrap();
    let (statement, local) = checked
        .body
        .body
        .statements
        .iter()
        .enumerate()
        .find_map(|(index, statement)| {
            let ir::TypedStmt::LetPattern { bindings, .. } = statement else {
                return None;
            };
            let [binding] = bindings.as_slice() else {
                return None;
            };
            assert_eq!(
                binding.projection,
                vec![ir::Projection::Deref, ir::Projection::Deref]
            );
            Some((index, binding.local))
        })
        .expect("real source pattern must lower to two checked dereferences");
    assert_eq!(checked.body.locals[local.0].name, "x");

    let mut changed = checked.clone();
    let body = Arc::make_mut(&mut changed.body);
    let ir::TypedStmt::LetPattern { bindings, .. } = &mut body.body.statements[statement] else {
        unreachable!()
    };
    bindings[0].projection.pop();
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );

    let mut changed = checked.clone();
    Arc::make_mut(&mut changed.body).locals[local.0].name = "foreign_binder".into();
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );

    let mut changed = checked.clone();
    let body = Arc::make_mut(&mut changed.body);
    let ir::TypedStmt::LetPattern { bindings, .. } = &mut body.body.statements[statement] else {
        unreachable!()
    };
    bindings[0].local = ir::LocalId(0);
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );
}
