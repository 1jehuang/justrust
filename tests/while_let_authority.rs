//! Genuine while-let source and exact-world checked loop authority controls.
//! No nominal factories, synthetic source handles or invented loop IDs are used.
use frontend::{Block, Expr, Pattern, PlanCtx, Program, Stmt};
use ir::{CheckedFunction, ExprKind, FactsCtx, MatchArm, MatchPattern, Projection};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

const SOURCE: &str = r#"
enum E { A(i32,bool,i32), B(i32) }
enum F { C(i32,bool,i32), D(i32) }
fn first(value:E)->i32 {
    macro_rules! marker { () => { 0 }; }
    let mut state:E=value;
    'again: while let E::A(first,..,last)=state {
        first+last;
        state=E::B(0);
        continue 'again;
    }
    'other: while let E::A(first,..,last)=state {
        first+last;
        break 'other;
    }
    0
}
fn second(value:E)->i32 {
    macro_rules! marker { () => { 0 }; }
    while let E::A(first,..,last)=value { first+last; break; }
    0
}
fn other(value:F)->i32 {
    while let F::C(first,..,last)=value { first+last; break; }
    0
}
fn main() {}
"#;

fn index(program: &Program, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap_or_else(|| panic!("missing real source function {name}"))
}
fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("actual resolved while-let source");
    frontend::check(&program).expect("actual unused while-let functions check");
    assert!(
        program.functions[index(&program, "first")]
            .lexical_scope
            .is_some()
    );
    assert!(
        program.functions[index(&program, "second")]
            .lexical_scope
            .is_some()
    );
    program
}
fn loop_statement<'a>(program: &'a Program, name: &str, nth: usize) -> &'a Stmt {
    program.functions[index(program, name)]
        .body
        .statements
        .iter()
        .filter(|statement| matches!(statement, Stmt::WhileLet { .. }))
        .nth(nth)
        .expect("actual honest while-let statement")
}
fn loop_statement_mut<'a>(program: &'a mut Program, name: &str, nth: usize) -> &'a mut Stmt {
    let index = index(program, name);
    program.functions[index]
        .body
        .statements
        .iter_mut()
        .filter(|statement| matches!(statement, Stmt::WhileLet { .. }))
        .nth(nth)
        .expect("actual honest while-let statement")
}
fn body<'a>(program: &'a Program, name: &str, nth: usize) -> &'a Block {
    let Stmt::WhileLet { body, .. } = loop_statement(program, name, nth) else {
        unreachable!()
    };
    body
}
fn body_expression(program: &Program, name: &str, nth: usize) -> Expr {
    let Stmt::Expr(value) = &body(program, name, nth).statements[0] else {
        panic!("actual first+last source expression statement required")
    };
    value.clone()
}
fn reject_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("transplanted while-let source acquired planning authority"),
    };
    let diagnostic = error.to_ascii_lowercase();
    assert!(
        diagnostic.contains("contract") || diagnostic.contains("foreign"),
        "wrong rejection: {error}"
    );
    assert!(
        !diagnostic.contains("unsupported"),
        "wrong boundary: {error}"
    );
    let usage = budget.usage_snapshot();
    assert_eq!(usage.applications, 0);
    assert_eq!(usage.obligations, 0);
    assert!(!usage.poisoned);
}
#[test]
fn actual_scoped_whilelet_clone_retains_plan_authority() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    PlanCtx::from_shared_program(Arc::new(program.clone()), &budget).unwrap();
}
#[test]
fn actual_whilelet_keeps_written_pattern_scrutinee_body_and_label() {
    let program = parsed();
    let Stmt::WhileLet {
        label,
        pattern,
        scrutinee,
        body,
        ..
    } = loop_statement(&program, "first", 0)
    else {
        unreachable!()
    };
    assert!(label.is_some());
    assert!(matches!(scrutinee.as_ref(), Expr::Var(_)));
    let Pattern::VariantRest { fields, .. } = pattern.as_ref() else {
        unreachable!()
    };
    assert_eq!(fields.prefix.len(), 1);
    assert_eq!(fields.suffix.len(), 1);
    assert_eq!(body.statements.len(), 3);
}
#[test]
fn foreign_whilelet_pattern_rejects_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Stmt::WhileLet { pattern, .. } = loop_statement(&foreign, "first", 0) else {
        unreachable!()
    };
    let donor = pattern.clone();
    let Stmt::WhileLet { pattern, .. } = loop_statement_mut(&mut program, "first", 0) else {
        unreachable!()
    };
    *pattern = donor;
    reject_source(program);
}
#[test]
fn foreign_whilelet_scrutinee_rejects_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Stmt::WhileLet { scrutinee, .. } = loop_statement(&foreign, "first", 0) else {
        unreachable!()
    };
    let donor = scrutinee.clone();
    let Stmt::WhileLet { scrutinee, .. } = loop_statement_mut(&mut program, "first", 0) else {
        unreachable!()
    };
    *scrutinee = donor;
    reject_source(program);
}
#[test]
fn foreign_unused_whilelet_body_rejects_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let donor = body(&foreign, "first", 0).clone();
    let Stmt::WhileLet { body, .. } = loop_statement_mut(&mut program, "first", 0) else {
        unreachable!()
    };
    *body = donor;
    reject_source(program);
}
#[test]
fn same_owner_loop_body_cannot_import_other_function_bindings() {
    let mut program = parsed();
    let donor = body(&program, "second", 0).clone();
    let Stmt::WhileLet { body, .. } = loop_statement_mut(&mut program, "first", 0) else {
        unreachable!()
    };
    *body = donor;
    reject_source(program);
}
#[test]
fn whilelet_body_reference_cannot_be_transplanted_into_its_scrutinee() {
    let mut program = parsed();
    let donor = body_expression(&program, "first", 0);
    let Stmt::WhileLet { scrutinee, .. } = loop_statement_mut(&mut program, "first", 0) else {
        unreachable!()
    };
    **scrutinee = donor;
    reject_source(program);
}
#[test]
fn whilelet_body_reference_cannot_be_transplanted_after_loop() {
    let mut program = parsed();
    let donor = body_expression(&program, "first", 0);
    let index = index(&program, "first");
    program.functions[index]
        .body
        .statements
        .push(Stmt::Expr(donor));
    reject_source(program);
}
#[test]
fn whilelet_body_reference_cannot_be_transplanted_into_sibling_loop() {
    let mut program = parsed();
    let donor = body_expression(&program, "first", 0);
    let Stmt::WhileLet { body, .. } = loop_statement_mut(&mut program, "first", 1) else {
        unreachable!()
    };
    body.statements[0] = Stmt::Expr(donor);
    reject_source(program);
}
#[test]
fn whilelet_binder_cannot_import_same_spelling_sibling_occurrence() {
    let mut program = parsed();
    let Stmt::WhileLet { pattern, .. } = loop_statement(&program, "first", 1) else {
        unreachable!()
    };
    let Pattern::VariantRest { fields, .. } = pattern.as_ref() else {
        unreachable!()
    };
    let donor = fields.prefix[0].clone();
    let Stmt::WhileLet { pattern, .. } = loop_statement_mut(&mut program, "first", 0) else {
        unreachable!()
    };
    let Pattern::VariantRest { fields, .. } = pattern.as_mut() else {
        unreachable!()
    };
    fields.prefix[0] = donor;
    reject_source(program);
}
#[test]
fn foreign_names_owner_cannot_authorize_unused_whilelet() {
    let mut program = parsed();
    program.names = parsed().names;
    reject_source(program);
}
#[test]
fn foreign_modules_owner_cannot_authorize_unused_whilelet() {
    let mut program = parsed();
    program.modules = parsed().modules;
    reject_source(program);
}
#[test]
fn foreign_scopes_owner_cannot_authorize_unused_whilelet() {
    let mut program = parsed();
    program.scopes = parsed().scopes;
    reject_source(program);
}

fn typed(
    program: &Program,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    name: &str,
) -> (CheckedFunction, Arc<ValidatedFacts>) {
    let declaration = (0..plan.callable_count())
        .map(|index| plan.callable_decl(index).unwrap())
        .find(|declaration| {
            let function = plan.callable_body(program, *declaration).unwrap();
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap();
    let checked = frontend::type_callable_with_plan(program, declaration, plan, budget).unwrap();
    let facts = plan
        .instance_facts(&ir::CallTarget {
            declaration,
            type_args: [].into(),
        })
        .unwrap();
    checked
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    cfg::lower_with_facts(&checked, &facts, facts.budget())
        .unwrap()
        .validate()
        .unwrap();
    (checked, facts)
}
struct CheckedFixture {
    checked: CheckedFunction,
    facts: Arc<ValidatedFacts>,
    other: CheckedFunction,
    _program: Program,
}
fn checked_fixture() -> CheckedFixture {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (checked, facts) = typed(&program, &plan, &budget, "first");
    let (other, _) = typed(&program, &plan, &budget, "other");
    CheckedFixture {
        checked,
        facts,
        other,
        _program: program,
    }
}
fn arms(function: &CheckedFunction) -> &[MatchArm] {
    function
        .exprs
        .iter()
        .find_map(|expression| match &expression.kind {
            ExprKind::Match { arms, .. } => Some(arms.as_slice()),
            _ => None,
        })
        .expect("actual checked while-let match")
}
fn arms_mut(function: &mut CheckedFunction) -> &mut Vec<MatchArm> {
    function
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Match { arms, .. } => Some(arms),
            _ => None,
        })
        .expect("actual checked while-let match")
}
fn reject_ir(function: &CheckedFunction, facts: &ValidatedFacts) {
    function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("malformed enum branch gained IR authority");
    assert!(cfg::lower_with_facts(function, facts, facts.budget()).is_err());
}
#[test]
fn actual_checked_whilelet_clone_preserves_enum_plan_and_selected_binding_indices() {
    let fixture = checked_fixture();
    let actual = arms(&fixture.checked);
    assert_eq!(actual.len(), 2);
    let MatchPattern::EnumVariant {
        ty,
        variant,
        bindings,
    } = &actual[0].pattern
    else {
        unreachable!()
    };
    assert_eq!(bindings.len(), 2);
    for (binding, field) in bindings.iter().zip([0, 2]) {
        assert_eq!(
            binding.projection,
            vec![
                Projection::Downcast(ir::VariantId {
                    owner: ty.def().id,
                    index: *variant
                }),
                Projection::Field(field)
            ]
        );
        assert!(!fixture.checked.params.contains(&binding.local));
    }
    assert_ne!(bindings[0].local, bindings[1].local);
    assert!(matches!(actual[1].pattern, MatchPattern::Wildcard));
    fixture
        .checked
        .clone()
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .unwrap();
}
#[test]
fn checked_whilelet_variant_cannot_import_other_actual_enum_owner() {
    let mut fixture = checked_fixture();
    let MatchPattern::EnumVariant { ty, .. } = &arms(&fixture.other)[0].pattern else {
        unreachable!()
    };
    let donor = ty.clone();
    let MatchPattern::EnumVariant { ty, .. } = &mut arms_mut(&mut fixture.checked)[0].pattern
    else {
        unreachable!()
    };
    *ty = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_whilelet_downcast_cannot_import_other_actual_enum_owner() {
    let mut fixture = checked_fixture();
    let MatchPattern::EnumVariant { ty, variant, .. } = &arms(&fixture.other)[0].pattern else {
        unreachable!()
    };
    let donor = ir::VariantId {
        owner: ty.def().id,
        index: *variant,
    };
    let MatchPattern::EnumVariant { bindings, .. } = &mut arms_mut(&mut fixture.checked)[0].pattern
    else {
        unreachable!()
    };
    bindings[0].projection[0] = Projection::Downcast(donor);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_whilelet_suffix_field_must_have_actual_selected_type() {
    let mut fixture = checked_fixture();
    let MatchPattern::EnumVariant { bindings, .. } = &mut arms_mut(&mut fixture.checked)[0].pattern
    else {
        unreachable!()
    };
    bindings[1].projection[1] = Projection::Field(1);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_whilelet_bindings_cannot_reuse_parameter_local() {
    let mut fixture = checked_fixture();
    let parameter = fixture.checked.params[0];
    let MatchPattern::EnumVariant { bindings, .. } = &mut arms_mut(&mut fixture.checked)[0].pattern
    else {
        unreachable!()
    };
    bindings[0].local = parameter;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_whilelet_bindings_cannot_share_local() {
    let mut fixture = checked_fixture();
    let MatchPattern::EnumVariant { bindings, .. } = &mut arms_mut(&mut fixture.checked)[0].pattern
    else {
        unreachable!()
    };
    bindings[1].local = bindings[0].local;
    reject_ir(&fixture.checked, &fixture.facts);
}
fn sibling_loop_ids(function: &CheckedFunction) -> (ir::LoopId, ir::LoopId) {
    let ids: Vec<_> = function
        .exprs
        .iter()
        .filter_map(|expression| match expression.kind {
            ExprKind::Loop { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2, "actual two sibling while-let loops required");
    assert_ne!(ids[0], ids[1]);
    (ids[0], ids[1])
}
#[test]
fn actual_false_edge_break_cannot_target_non_enclosing_sibling_loop() {
    let mut fixture = checked_fixture();
    let (first, second) = sibling_loop_ids(&fixture.checked);
    let target = fixture
        .checked
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Break {
                target,
                value: None,
            } if *target == first => Some(target),
            _ => None,
        })
        .expect("actual semantic false-edge break");
    *target = second;
    // Loop IDs are structurally well-formed. Active-target authority is checked
    // during CFG lowering, not invented by the source or signature world.
    fixture
        .checked
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .unwrap();
    let error = cfg::lower_with_facts(&fixture.checked, &fixture.facts, fixture.facts.budget())
        .expect_err("inactive sibling loop acquired false-edge break authority");
    assert!(
        error.contains("missing CFG break target"),
        "wrong boundary: {error}"
    );
}
#[test]
fn actual_continue_cannot_target_non_enclosing_sibling_loop() {
    let mut fixture = checked_fixture();
    let (first, second) = sibling_loop_ids(&fixture.checked);
    let target = fixture
        .checked
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Continue { target } if *target == first => Some(target),
            _ => None,
        })
        .expect("actual source continue target");
    *target = second;
    fixture
        .checked
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .unwrap();
    let error = cfg::lower_with_facts(&fixture.checked, &fixture.facts, fixture.facts.budget())
        .expect_err("inactive sibling loop acquired continue authority");
    assert!(
        error.contains("missing CFG continue target"),
        "wrong boundary: {error}"
    );
}
#[test]
fn ordinary_whilelet_binding_is_not_visible_after_loop() {
    let source =
        "enum E{A(i32),B}fn f(value:E)->i32{while let E::A(local)=value{break;}local}fn main(){}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("ordinary body binding leaked after loop");
    let diagnostic = error.to_ascii_lowercase();
    assert!(
        !diagnostic.contains("unsupported"),
        "wrong boundary: {error}"
    );
    assert!(
        !diagnostic.contains("contract"),
        "ordinary invalid source is not forged authority: {error}"
    );
}
#[test]
fn whilelet_body_requires_unit_not_value_producing_tail() {
    let source = "fn f(value:bool){while let true=value{1i32}}fn main(){}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("non-Unit while-let body was silently discarded");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "wrong boundary: {error}"
    );
}
