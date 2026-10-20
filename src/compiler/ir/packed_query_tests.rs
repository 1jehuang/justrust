use super::*;
use crate::compiler::{frontend, source};
use std::sync::Arc;

fn fixture() -> (frontend::PlanCtx, DeclId) {
    let options = source::SourceOptions::default();
    let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "packed-ir-authority.rs",
            "fn main(){}",
            &options,
        )))
        .unwrap();
    let prepared = loop {
        match step {
            source::AssemblyStep::Done(prepared) => break prepared,
            source::AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None => source::prepare_prelude_unit(&options, &budget),
                    Some(parent) => {
                        source::prepare_child_unit(parent, need.index, &options, &budget)
                    }
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    };
    let parsed = frontend::parse_prepared(&prepared).unwrap();
    let program = frontend::resolve(&parsed).unwrap();
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
        .unwrap();
    let plan = frontend::PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
    let declaration = (0..plan.callable_count())
        .map(|index| plan.callable_decl(index).unwrap())
        .find(|declaration| {
            let info = plan.declaration(*declaration).unwrap();
            info.kind == super::super::DeclKind::Function && info.source_index == source_index
        })
        .unwrap();
    (plan, declaration)
}

#[test]
fn packed_query_canonical_publication_rejects_same_owner_wrong_occurrences() {
    use crate::compiler::{cfg, traits::FactsPhase};
    let (plan, owner) = fixture();
    let budget = &plan.budget;
    let frontend::PublishedCallableBody::Source(publication) =
        plan.publish_callable_body(owner, budget).unwrap()
    else {
        panic!("genuine packed helper must publish canonical source")
    };
    assert_eq!(publication.facts().phase(), FactsPhase::Closed);
    assert!(publication.facts().target().unwrap().type_args.is_empty());
    let source = publication.source();
    publication
        .validate_source(source, publication.facts(), budget)
        .unwrap();
    let (query_index, query) = source
        .exprs
        .iter()
        .enumerate()
        .find_map(|(index, expression)| match &expression.kind {
            super::super::ExprKind::TypeAlignment(query) => Some((index, query.clone())),
            _ => None,
        })
        .expect("canonical checked helper retains its authenticated query");
    query
        .validate(FactsCtx::Validated(publication.facts()), budget)
        .unwrap();
    let other_index = source
        .exprs
        .iter()
        .enumerate()
        .find_map(|(index, expression)| {
            (index != query_index && expression.ty == Ty::Int(super::super::IntTy::Usize))
                .then_some(index)
        })
        .expect("helper has a separate usize expression");
    let mut wrong_expression = source.as_ref().clone();
    wrong_expression.exprs[other_index].kind = super::super::ExprKind::TypeAlignment(query.clone());
    assert!(
        publication
            .validate_source(&Arc::new(wrong_expression), publication.facts(), budget)
            .is_err()
    );

    let function = cfg::lower_source_closed(&publication, budget).unwrap();
    function
        .validate_source_publication(&publication, budget)
        .unwrap();
    let mut wrong_statement = function.clone();
    let assignment = wrong_statement
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find(|statement| {
            matches!(&statement.value, cfg::Statement::Assign { value, .. }
            if !matches!(value, cfg::Rvalue::TypeAlignment(_)))
        })
        .expect("lowered helper has an ordinary assignment");
    let cfg::Statement::Assign { value, .. } = &mut assignment.value else {
        unreachable!()
    };
    *value = cfg::Rvalue::TypeAlignment(query);
    assert!(
        wrong_statement
            .validate_source_publication(&publication, budget)
            .is_err()
    );
    assert!(
        wrong_statement
            .validate_provenance(publication.facts(), budget)
            .is_err()
    );
}

#[test]
fn packed_query_ir_rejects_missing_swapped_kind_type_owner_and_capability() {
    let (plan, owner) = fixture();
    let budget = &plan.budget;
    let facts = plan.callable_facts(owner).unwrap();
    let cap = plan.packed_reserve_authority(owner).unwrap();
    let query =
        TypeAlignmentQuery::issue_packed(Ty::Int(super::super::IntTy::U8), &facts, cap).unwrap();
    query.validate(FactsCtx::Validated(&facts), budget).unwrap();
    assert!(query.same_query(&query.clone(), budget).unwrap());
    let reject = |changed: &TypeAlignmentQuery| {
        assert!(!query.same_query(changed, budget).unwrap());
        assert!(
            changed
                .validate(FactsCtx::Validated(&facts), budget)
                .is_err()
        );
    };
    let mut changed = query.clone();
    changed.packed = None;
    reject(&changed);
    changed = query.clone();
    changed.kind = TypeQueryKind::Alignment;
    reject(&changed);
    changed = query.clone();
    changed.kind = TypeQueryKind::PrivateStorageStride;
    reject(&changed);
    changed = query.clone();
    changed.queried = Ty::Int(super::super::IntTy::U16);
    reject(&changed);
    let foreign = fixture();
    changed = query.clone();
    changed.packed = Some(foreign.0.packed_reserve_authority(foreign.1).unwrap());
    reject(&changed);
    let foreign_facts = foreign.0.callable_facts(foreign.1).unwrap();
    assert!(
        query
            .validate(FactsCtx::Validated(&foreign_facts), budget)
            .is_err()
    );
    changed = query.clone();
    changed.owner = foreign.1;
    reject(&changed);
    assert!(query.validate(FactsCtx::Concrete, budget).is_err());
}
