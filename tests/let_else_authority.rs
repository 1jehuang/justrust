//! Let-else keeps genuine escaping source bindings and exact checked-world facts.
//! Structural Never annotations are not themselves a proof of a diverging CFG.
use frontend::{Expr, Pattern, PlanCtx, Program, Stmt};
use ir::{CheckedFunction, ExprKind, FactsCtx, MatchPattern, Projection};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

const SOURCE: &str = r#"
enum E { A(i32,bool,i32), B(i32) }
enum F { C(i32,bool,i32), D(i32) }
fn first(value:E)->i32 {
    macro_rules! marker { () => { 0 }; }
    let E::A(first,..,last)=value else {return 0;};
    first+last
}
fn second(value:E)->i32 {
    macro_rules! marker { () => { 0 }; }
    let E::A(first,..,last)=value else {return 1;};
    first+last
}
fn other(value:F)->i32 {
    let F::C(first,..,last)=value else {return 0;};
    first+last
}
fn shadow(value:E,first:i32)->i32 {
    macro_rules! marker { () => { 0 }; }
    let E::A(first,..,last)=value else {return first;};
    first+last
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
        .unwrap()
}
fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("actual resolved let-else source");
    frontend::check(&program).expect("all actual unused escaping binding functions check");
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
fn statement<'a>(program: &'a Program, name: &str) -> &'a Stmt {
    program.functions[index(program, name)]
        .body
        .statements
        .iter()
        .find(|statement| matches!(statement, Stmt::LetElse { .. }))
        .expect("honest source let-else statement")
}
fn statement_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut Stmt {
    let index = index(program, name);
    program.functions[index]
        .body
        .statements
        .iter_mut()
        .find(|statement| matches!(statement, Stmt::LetElse { .. }))
        .expect("honest source let-else statement")
}
fn post_expression(program: &Program, name: &str) -> Expr {
    program.functions[index(program, name)]
        .body
        .tail
        .as_deref()
        .unwrap()
        .clone()
}
fn reject_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("transplanted escaping binding gained source authority"),
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
fn actual_scoped_letelse_clone_retains_planning_authority() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    PlanCtx::from_shared_program(Arc::new(program.clone()), &budget).unwrap();
}
#[test]
fn source_retains_only_written_pattern_children_and_actual_else_block() {
    let program = parsed();
    let Stmt::LetElse {
        pattern,
        value,
        else_block,
        ..
    } = statement(&program, "first")
    else {
        unreachable!()
    };
    let Pattern::VariantRest { fields, .. } = pattern.as_ref() else {
        unreachable!()
    };
    assert_eq!(fields.prefix.len(), 1);
    assert_eq!(fields.suffix.len(), 1);
    assert!(matches!(value.as_ref(), Expr::Var(_)));
    assert!(matches!(else_block.statements[0], Stmt::Return(Some(_))));
}
#[test]
fn escaped_binding_reference_cannot_move_into_initializer_old_scope() {
    let mut program = parsed();
    let donor = post_expression(&program, "first");
    let Stmt::LetElse { value, .. } = statement_mut(&mut program, "first") else {
        unreachable!()
    };
    **value = donor;
    reject_source(program);
}
#[test]
fn escaped_binding_reference_cannot_move_into_else_old_scope() {
    let mut program = parsed();
    let donor = post_expression(&program, "first");
    let Stmt::LetElse { else_block, .. } = statement_mut(&mut program, "first") else {
        unreachable!()
    };
    else_block.statements[0] = Stmt::Return(Some(donor));
    reject_source(program);
}
#[test]
fn escaped_binding_reference_cannot_move_before_letelse_statement() {
    let mut program = parsed();
    let donor = post_expression(&program, "first");
    let index = index(&program, "first");
    program.functions[index]
        .body
        .statements
        .insert(0, Stmt::Expr(donor));
    reject_source(program);
}
#[test]
fn same_owner_post_statement_use_cannot_import_other_function_binding() {
    let mut program = parsed();
    let donor = post_expression(&program, "second");
    let index = index(&program, "first");
    program.functions[index].body.tail = Some(Box::new(donor));
    reject_source(program);
}
#[test]
fn same_owner_pattern_binder_cannot_import_other_function_occurrence() {
    let mut program = parsed();
    let Stmt::LetElse { pattern, .. } = statement(&program, "second") else {
        unreachable!()
    };
    let Pattern::VariantRest { fields, .. } = pattern.as_ref() else {
        unreachable!()
    };
    let donor = fields.prefix[0].clone();
    let Stmt::LetElse { pattern, .. } = statement_mut(&mut program, "first") else {
        unreachable!()
    };
    let Pattern::VariantRest { fields, .. } = pattern.as_mut() else {
        unreachable!()
    };
    fields.prefix[0] = donor;
    reject_source(program);
}
#[test]
fn foreign_unused_pattern_rejects_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Stmt::LetElse { pattern, .. } = statement(&foreign, "first") else {
        unreachable!()
    };
    let donor = pattern.clone();
    let Stmt::LetElse { pattern, .. } = statement_mut(&mut program, "first") else {
        unreachable!()
    };
    *pattern = donor;
    reject_source(program);
}
#[test]
fn foreign_unused_initializer_rejects_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Stmt::LetElse { value, .. } = statement(&foreign, "first") else {
        unreachable!()
    };
    let donor = value.clone();
    let Stmt::LetElse { value, .. } = statement_mut(&mut program, "first") else {
        unreachable!()
    };
    *value = donor;
    reject_source(program);
}
#[test]
fn foreign_unused_else_block_rejects_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Stmt::LetElse { else_block, .. } = statement(&foreign, "shadow") else {
        unreachable!()
    };
    let donor = else_block.clone();
    let Stmt::LetElse { else_block, .. } = statement_mut(&mut program, "shadow") else {
        unreachable!()
    };
    *else_block = donor;
    reject_source(program);
}
#[test]
fn foreign_names_owner_rejects_unused_letelse() {
    let mut program = parsed();
    program.names = parsed().names;
    reject_source(program);
}
#[test]
fn foreign_modules_owner_rejects_unused_letelse() {
    let mut program = parsed();
    program.modules = parsed().modules;
    reject_source(program);
}
#[test]
fn foreign_scopes_owner_rejects_unused_letelse() {
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
    let lowered = cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap();
    lowered
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    assert!(
        ownership::analyze(&lowered).passed(),
        "actual escaping bindings fail baseline ownership"
    );
    (checked, facts)
}
struct Fixture {
    checked: CheckedFunction,
    facts: Arc<ValidatedFacts>,
    other: CheckedFunction,
    _program: Program,
}
fn fixture() -> Fixture {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (checked, facts) = typed(&program, &plan, &budget, "first");
    let (other, _) = typed(&program, &plan, &budget, "other");
    Fixture {
        checked,
        facts,
        other,
        _program: program,
    }
}
fn checked_statement(function: &CheckedFunction) -> &ir::TypedStmt {
    function
        .body
        .statements
        .iter()
        .find(|statement| matches!(statement, ir::TypedStmt::LetElse { .. }))
        .expect("actual checked persistent let-else statement")
}
fn checked_statement_mut(function: &mut CheckedFunction) -> &mut ir::TypedStmt {
    function
        .body
        .statements
        .iter_mut()
        .find(|statement| matches!(statement, ir::TypedStmt::LetElse { .. }))
        .expect("actual checked persistent let-else statement")
}
fn checked_pattern(function: &CheckedFunction) -> &MatchPattern {
    let ir::TypedStmt::LetElse { pattern, .. } = checked_statement(function) else {
        unreachable!()
    };
    pattern
}
fn checked_pattern_mut(function: &mut CheckedFunction) -> &mut MatchPattern {
    let ir::TypedStmt::LetElse { pattern, .. } = checked_statement_mut(function) else {
        unreachable!()
    };
    pattern
}
fn else_id(function: &CheckedFunction) -> ir::ExprId {
    let ir::TypedStmt::LetElse { else_branch, .. } = checked_statement(function) else {
        unreachable!()
    };
    *else_branch
}
fn reject_ir(function: &CheckedFunction, facts: &ValidatedFacts) {
    function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("malformed let-else acquired structural authority");
    assert!(cfg::lower_with_facts(function, facts, facts.budget()).is_err());
}
#[test]
fn genuine_letelse_bindings_persist_for_post_statement_read_and_end_scope_cleanup() {
    let fixture = fixture();
    let MatchPattern::EnumVariant {
        ty,
        variant,
        bindings,
    } = checked_pattern(&fixture.checked)
    else {
        unreachable!()
    };
    assert_eq!(bindings.len(), 2);
    assert_ne!(bindings[0].local, bindings[1].local);
    let lowered =
        cfg::lower_with_facts(&fixture.checked, &fixture.facts, fixture.facts.budget()).unwrap();
    for (binding, field) in bindings.iter().zip([0, 2]) {
        assert!(!fixture.checked.params.contains(&binding.local));
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
        assert!(fixture.checked.exprs.iter().any(|expression|matches!(expression.kind,ExprKind::Local(local) if local==binding.local)),"genuine continuation must read actual binding");
        assert!(lowered.blocks.iter().flat_map(|block|&block.statements).any(|statement|matches!(statement.value,cfg::Statement::StorageLive(local) if local==binding.local)));
        assert!(lowered.blocks.iter().flat_map(|block|&block.statements).any(|statement|matches!(statement.value,cfg::Statement::StorageDead(local) if local==binding.local)));
    }
    assert!(
        ownership::analyze(&lowered).passed(),
        "match-arm cleanup killed escaping bindings"
    );
}
#[test]
fn checked_letelse_domain_cannot_import_other_actual_enum_owner() {
    let mut fixture = fixture();
    let ir::TypedStmt::LetElse { ty, .. } = checked_statement(&fixture.other) else {
        unreachable!()
    };
    let donor = ty.clone();
    let ir::TypedStmt::LetElse { ty, .. } = checked_statement_mut(&mut fixture.checked) else {
        unreachable!()
    };
    *ty = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_letelse_pattern_cannot_import_other_actual_enum_owner() {
    let mut fixture = fixture();
    let MatchPattern::EnumVariant { ty, .. } = checked_pattern(&fixture.other) else {
        unreachable!()
    };
    let donor = ty.clone();
    let MatchPattern::EnumVariant { ty, .. } = checked_pattern_mut(&mut fixture.checked) else {
        unreachable!()
    };
    *ty = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_letelse_downcast_cannot_import_other_actual_owner() {
    let mut fixture = fixture();
    let MatchPattern::EnumVariant { ty, variant, .. } = checked_pattern(&fixture.other) else {
        unreachable!()
    };
    let donor = ir::VariantId {
        owner: ty.def().id,
        index: *variant,
    };
    let MatchPattern::EnumVariant { bindings, .. } = checked_pattern_mut(&mut fixture.checked)
    else {
        unreachable!()
    };
    bindings[0].projection[0] = Projection::Downcast(donor);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_letelse_selected_field_cannot_have_wrong_actual_type() {
    let mut fixture = fixture();
    let MatchPattern::EnumVariant { bindings, .. } = checked_pattern_mut(&mut fixture.checked)
    else {
        unreachable!()
    };
    bindings[1].projection[1] = Projection::Field(1);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_letelse_enum_payload_cannot_import_array_index_projection() {
    let mut fixture = fixture();
    let MatchPattern::EnumVariant { bindings, .. } = checked_pattern_mut(&mut fixture.checked)
    else {
        unreachable!()
    };
    bindings[0].projection[1] = Projection::ConstantIndex(0);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_letelse_binding_cannot_alias_parameter() {
    let mut fixture = fixture();
    let parameter = fixture.checked.params[0];
    let MatchPattern::EnumVariant { bindings, .. } = checked_pattern_mut(&mut fixture.checked)
    else {
        unreachable!()
    };
    bindings[0].local = parameter;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_letelse_bindings_cannot_share_one_local() {
    let mut fixture = fixture();
    let MatchPattern::EnumVariant { bindings, .. } = checked_pattern_mut(&mut fixture.checked)
    else {
        unreachable!()
    };
    bindings[1].local = bindings[0].local;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_letelse_requires_actual_never_else_type() {
    let mut fixture = fixture();
    let id = else_id(&fixture.checked);
    fixture.checked.exprs[id.0] = ir::TypedExpr {
        ty: ir::Ty::Unit,
        completion: ir::Completion::Returns,
        kind: ExprKind::Block(ir::TypedBlock {
            statements: vec![],
            tail: None,
        }),
    };
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn checked_letelse_requires_never_completion_not_only_never_type() {
    let mut fixture = fixture();
    let id = else_id(&fixture.checked);
    fixture.checked.exprs[id.0].completion = ir::Completion::Returns;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn forged_never_annotation_cannot_make_empty_else_block_diverge() {
    let mut fixture = fixture();
    let id = else_id(&fixture.checked);
    let otherwise = &mut fixture.checked.exprs[id.0];
    assert_eq!(otherwise.ty, ir::Ty::Never);
    assert_eq!(otherwise.completion, ir::Completion::Never);
    // Preserve the forged claim deliberately. The block has no return, loop,
    // break, continue or call and really reaches its continuation.
    otherwise.kind = ExprKind::Block(ir::TypedBlock {
        statements: vec![],
        tail: None,
    });
    if fixture
        .checked
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .is_err()
    {
        return;
    }
    let lowered =
        match cfg::lower_with_facts(&fixture.checked, &fixture.facts, fixture.facts.budget()) {
            Err(_) => return,
            Ok(lowered) => lowered,
        };
    if lowered
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .is_err()
    {
        return;
    }
    let analysis = ownership::analyze(&lowered);
    assert!(
        !analysis.passed(),
        "a fake typed Never empty block gained a real divergence proof"
    );
}
#[test]
fn ordinary_else_cannot_use_new_escaping_binding() {
    let source = "enum E{A(i32),B}fn f(value:E)->i32{let E::A(local)=value else{return local;};local}fn main(){}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("new binding leaked into else old scope");
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
fn ordinary_nondivergent_else_is_invalid_even_with_irrefutable_pattern() {
    let source = "fn f(value:i32)->i32{let local=value else{};local}fn main(){}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("irrefutable pattern suppressed invalid else divergence");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "wrong boundary: {error}"
    );
}
