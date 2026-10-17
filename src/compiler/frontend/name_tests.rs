//! Owner isolation checks for the actual typed-label AST lane.
use super::*;

fn loop_label(program: &Program) -> AstIdent {
    let Stmt::Expr(Expr::Loop {
        label: Some(label), ..
    }) = &program.functions[0].body.statements[0]
    else {
        panic!("expected labeled loop fixture");
    };
    *label
}

#[test]
fn separately_minted_label_occurrences_use_semantic_keys_not_spans() {
    let program = parse("fn main(){'outer: loop { break 'outer; };}").unwrap();
    let Stmt::Expr(Expr::Loop {
        label: Some(declaration),
        body,
    }) = &program.functions[0].body.statements[0]
    else {
        panic!("expected labeled loop");
    };
    let Stmt::Expr(Expr::Break {
        label: Some(reference),
        ..
    }) = &body.statements[0]
    else {
        panic!("expected labeled break");
    };
    assert_ne!(
        program.names.span(*declaration).unwrap(),
        program.names.span(*reference).unwrap()
    );
    assert_eq!(
        program.names.key(*declaration, LookupClass::Label).unwrap(),
        program.names.key(*reference, LookupClass::Label).unwrap()
    );
    type_env(&program).unwrap();
}

#[test]
fn same_spelling_foreign_label_fails_even_in_an_unused_body() {
    let foreign = parse("fn foreign(){'outer: loop { break 'outer; };}").unwrap();
    let mut program = parse("fn unused(){'outer: loop { break 'outer; };}fn main(){}").unwrap();
    let foreign_label = loop_label(&foreign);
    let Stmt::Expr(Expr::Loop {
        label: Some(label), ..
    }) = &mut program.functions[0].body.statements[0]
    else {
        panic!("expected labeled loop");
    };
    *label = foreign_label;
    assert!(program.names.check(foreign_label).is_err());
    assert!(PlanCtx::from_program(&program, &ir::PlanBudget::default()).is_err());
}

#[test]
fn cloned_program_retains_the_exact_name_store_and_source() {
    let program = parse("fn main(){'outer: loop { break 'outer; };}").unwrap();
    let label = loop_label(&program);
    let cloned = program.clone();
    assert!(Arc::ptr_eq(&program.names, &cloned.names));
    drop(program);
    assert_eq!(cloned.names.spelling(label).unwrap(), "outer");
    let signatures = type_env(&cloned).unwrap();
    type_function_with_signatures(&cloned, 0, &signatures).unwrap();
}

#[test]
fn fixture_builder_does_not_adopt_a_foreign_label() {
    let mut foreign = ProgramBuilder::new(
        std::path::Path::new("foreign-fixture"),
        "",
        &super::super::expansion_budget::ExpansionBudget::default(),
    )
    .unwrap();
    let label = foreign.ident("outer").unwrap();
    let mut program = parse("fn main(){'outer: loop { break 'outer; };}").unwrap();
    let Stmt::Expr(Expr::Loop {
        label: Some(target),
        ..
    }) = &mut program.functions[0].body.statements[0]
    else {
        panic!("expected labeled loop");
    };
    *target = label;
    assert!(program.validate_name_handles().is_err());
}

#[test]
fn unlabeled_same_spelling_program_cannot_reuse_a_foreign_plan() {
    let original = parse("fn main(){}").unwrap();
    let foreign = parse("fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&original, &budget).unwrap();
    assert!(type_function_with_plan(&foreign, 0, &plan, &budget).is_err());
    type_function_with_plan(&original.clone(), 0, &plan, &budget).unwrap();
    let mut replaced = original.clone();
    replaced.names = foreign.names.clone();
    assert!(type_function_with_plan(&replaced, 0, &plan, &budget).is_err());
}

#[test]
fn public_flat_ast_validation_stops_on_work_denial_and_releases_stack() {
    use super::super::expansion_budget::{BudgetFailureKind, BudgetResource, Counter};
    let mut program = parse("fn main(){}").unwrap();
    program.functions[0].body.statements = vec![Stmt::Return(None); 512];
    let budget = program.names.budget().clone();
    let before = budget.snapshot();
    let remaining =
        budget.limits().counters[Counter::Work as usize] - before.counters[Counter::Work as usize];
    budget.charge(Counter::Work, remaining - 8).unwrap();
    let error = program.validate_name_handles().unwrap_err();
    assert!(
        error.contains("limit") || error.contains("Limit"),
        "{error}"
    );
    let after = budget.snapshot();
    let failure = after.failure.unwrap();
    assert_eq!(failure.kind, BudgetFailureKind::Limit);
    assert_eq!(failure.resource, BudgetResource::Counter(Counter::Work));
    assert_eq!(after.temporary_bytes, before.temporary_bytes);
}

#[test]
fn public_child_stack_is_budgeted_before_bulk_admission() {
    use super::super::expansion_budget::{BudgetFailureKind, BudgetResource, Counter};
    let mut program = parse("fn main(){}").unwrap();
    program.functions[0].body.statements = vec![Stmt::Expr(Expr::Bool(true)); 512];
    let budget = program.names.budget().clone();
    let before = budget.snapshot();
    let remaining =
        budget.limits().counters[Counter::Work as usize] - before.counters[Counter::Work as usize];
    budget.charge(Counter::Work, remaining - 16).unwrap();
    program.validate_name_handles().unwrap_err();
    let after = budget.snapshot();
    let failure = after.failure.unwrap();
    assert_eq!(failure.kind, BudgetFailureKind::Limit);
    assert_eq!(failure.resource, BudgetResource::Counter(Counter::Work));
    assert_eq!(after.temporary_bytes, before.temporary_bytes);
    assert!(after.peak_bytes - before.peak_bytes < 4096);
}
