//! If-let authority controls use only actual parsed source owners and checked plans.
//! Source authority errors are distinct from ordinary branch typing/scope errors.
use frontend::{Block, Expr, Pattern, PlanCtx, Program};
use ir::{CheckedFunction, ExprKind, FactsCtx, MatchPattern, Projection, TypedMatchArm};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

const SOURCE: &str = r#"
enum E { A(i32,bool,i32), B(i32) }
enum F { C(i32,bool,i32), D(i32) }
fn first(value:E)->i32 {
    macro_rules! marker { () => { 0 }; }
    if let E::A(first,..,last)=value { first+last } else { 0 }
}
fn second(value:E)->i32 {
    macro_rules! marker { () => { 0 }; }
    if let E::A(first,..,last)=value { first+last } else { 1 }
}
fn other(value:F)->i32 {
    if let F::C(first,..,last)=value { first+last } else { 0 }
}
fn absent(value:bool) { if let true=value {} }
fn nested(value:bool)->i32 {
    if let true=value { 1 } else if let false=value { 2 } else { 3 }
}
fn main() {}
"#;

fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("actual resolved if-let source");
    frontend::check(&program).expect("all actual unused if-let functions check");
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

fn index(program: &Program, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap_or_else(|| panic!("missing actual source function {name}"))
}

fn source_expr<'a>(program: &'a Program, name: &str) -> &'a Expr {
    program.functions[index(program, name)]
        .body
        .tail
        .as_deref()
        .expect("actual if-let tail")
}
fn source_expr_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut Expr {
    let index = index(program, name);
    program.functions[index]
        .body
        .tail
        .as_deref_mut()
        .expect("actual if-let tail")
}
fn pattern<'a>(program: &'a Program, name: &str) -> &'a Pattern {
    let Expr::IfLet { pattern, .. } = source_expr(program, name) else {
        panic!("honest if-let source node required")
    };
    pattern
}
fn pattern_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut Pattern {
    let Expr::IfLet { pattern, .. } = source_expr_mut(program, name) else {
        panic!("honest if-let source node required")
    };
    pattern
}
fn then_block<'a>(program: &'a Program, name: &str) -> &'a Block {
    let Expr::IfLet { then_block, .. } = source_expr(program, name) else {
        unreachable!()
    };
    then_block
}
fn reject_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("transplanted if-let source acquired planning authority"),
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
fn exact_parsed_clone_retains_real_iflet_scope_and_plan_authority() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    PlanCtx::from_shared_program(Arc::new(program.clone()), &budget).unwrap();
}
#[test]
fn honest_iflet_retains_absent_else_and_actual_else_iflet_node() {
    let program = parsed();
    let Expr::IfLet { else_branch, .. } = source_expr(&program, "absent") else {
        unreachable!()
    };
    assert!(else_branch.is_none());
    let Expr::IfLet { else_branch, .. } = source_expr(&program, "nested") else {
        unreachable!()
    };
    assert!(matches!(else_branch.as_deref(), Some(Expr::IfLet { .. })));
}
#[test]
fn foreign_pattern_child_rejects_before_source_authority() {
    let mut program = parsed();
    let foreign = parsed();
    *pattern_mut(&mut program, "first") = pattern(&foreign, "first").clone();
    reject_source(program);
}
#[test]
fn foreign_scrutinee_rejects_before_source_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Expr::IfLet { scrutinee, .. } = source_expr(&foreign, "first") else {
        unreachable!()
    };
    let donor = scrutinee.clone();
    let Expr::IfLet { scrutinee, .. } = source_expr_mut(&mut program, "first") else {
        unreachable!()
    };
    *scrutinee = donor;
    reject_source(program);
}
#[test]
fn foreign_then_body_rejects_before_source_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let donor = then_block(&foreign, "first").clone();
    let Expr::IfLet { then_block, .. } = source_expr_mut(&mut program, "first") else {
        unreachable!()
    };
    *then_block = donor;
    reject_source(program);
}
#[test]
fn same_owner_same_spelling_then_body_cannot_change_function_binding_association() {
    let mut program = parsed();
    let donor = then_block(&program, "second").clone();
    let Expr::IfLet { then_block, .. } = source_expr_mut(&mut program, "first") else {
        unreachable!()
    };
    *then_block = donor;
    reject_source(program);
}
#[test]
fn same_owner_pattern_binder_cannot_import_other_function_occurrence() {
    let mut program = parsed();
    let Pattern::VariantRest { fields, .. } = pattern(&program, "second") else {
        unreachable!()
    };
    let donor = fields.prefix[0].clone();
    let Pattern::VariantRest { fields, .. } = pattern_mut(&mut program, "first") else {
        unreachable!()
    };
    fields.prefix[0] = donor;
    reject_source(program);
}
#[test]
fn then_binding_reference_cannot_be_transplanted_into_else() {
    let mut program = parsed();
    let donor = then_block(&program, "first").clone();
    let Expr::IfLet { else_branch, .. } = source_expr_mut(&mut program, "first") else {
        unreachable!()
    };
    *else_branch = Some(Box::new(Expr::Block(donor)));
    reject_source(program);
}
#[test]
fn then_binding_reference_cannot_be_transplanted_after_iflet() {
    let mut program = parsed();
    let index = index(&program, "first");
    let donor = then_block(&program, "first").tail.clone().unwrap();
    let original = program.functions[index].body.tail.take().unwrap();
    program.functions[index]
        .body
        .statements
        .push(frontend::Stmt::Expr(*original));
    program.functions[index].body.tail = Some(donor);
    reject_source(program);
}
#[test]
fn foreign_name_owner_cannot_repair_an_unused_iflet_function() {
    let mut program = parsed();
    program.names = parsed().names;
    reject_source(program);
}
#[test]
fn foreign_module_owner_cannot_repair_an_unused_iflet_function() {
    let mut program = parsed();
    program.modules = parsed().modules;
    reject_source(program);
}
#[test]
fn foreign_scope_owner_cannot_repair_an_unused_iflet_function() {
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
fn checked_fixture() -> (
    CheckedFunction,
    Arc<ValidatedFacts>,
    CheckedFunction,
    Program,
) {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (checked, facts) = typed(&program, &plan, &budget, "first");
    let (other, _) = typed(&program, &plan, &budget, "other");
    (checked, facts, other, program)
}
fn arms(function: &CheckedFunction) -> &[TypedMatchArm] {
    function
        .exprs
        .iter()
        .find_map(|expression| match &expression.kind {
            ExprKind::Match { arms, .. } => Some(arms.as_slice()),
            _ => None,
        })
        .expect("actual checked if-let Match IR")
}
fn arms_mut(function: &mut CheckedFunction) -> &mut Vec<TypedMatchArm> {
    function
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Match { arms, .. } => Some(arms),
            _ => None,
        })
        .expect("actual checked if-let Match IR")
}
fn reject_checked(function: &CheckedFunction, facts: &ValidatedFacts) {
    function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("malformed if-let branch plan gained structural authority");
    assert!(cfg::lower_with_facts(function, facts, facts.budget()).is_err());
}
#[test]
fn actual_checked_iflet_has_real_enum_downcasts_then_only_bindings_and_wildcard_fallback() {
    let (checked, facts, _, _program) = checked_fixture();
    let actual = arms(&checked);
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
        assert!(!checked.params.contains(&binding.local));
    }
    assert_ne!(bindings[0].local, bindings[1].local);
    assert!(matches!(actual[1].pattern, MatchPattern::Wildcard));
    checked
        .clone()
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
}
#[test]
fn checked_iflet_variant_cannot_import_another_actual_enum_owner() {
    let (mut checked, facts, other, _program) = checked_fixture();
    let MatchPattern::EnumVariant { ty: donor, .. } = &arms(&other)[0].pattern else {
        unreachable!()
    };
    let donor = donor.clone();
    let MatchPattern::EnumVariant { ty, .. } = &mut arms_mut(&mut checked)[0].pattern else {
        unreachable!()
    };
    *ty = donor;
    reject_checked(&checked, &facts);
}
#[test]
fn checked_iflet_downcast_cannot_import_other_actual_variant_owner() {
    let (mut checked, facts, other, _program) = checked_fixture();
    let MatchPattern::EnumVariant { ty, variant, .. } = &arms(&other)[0].pattern else {
        unreachable!()
    };
    let donor = ir::VariantId {
        owner: ty.def().id,
        index: *variant,
    };
    let MatchPattern::EnumVariant { bindings, .. } = &mut arms_mut(&mut checked)[0].pattern else {
        unreachable!()
    };
    bindings[0].projection[0] = Projection::Downcast(donor);
    reject_checked(&checked, &facts);
}
#[test]
fn checked_iflet_branch_binding_cannot_be_parameter_local() {
    let (mut checked, facts, _, _program) = checked_fixture();
    let parameter = checked.params[0];
    let MatchPattern::EnumVariant { bindings, .. } = &mut arms_mut(&mut checked)[0].pattern else {
        unreachable!()
    };
    bindings[0].local = parameter;
    reject_checked(&checked, &facts);
}
#[test]
fn checked_iflet_branch_bindings_cannot_share_local() {
    let (mut checked, facts, _, _program) = checked_fixture();
    let MatchPattern::EnumVariant { bindings, .. } = &mut arms_mut(&mut checked)[0].pattern else {
        unreachable!()
    };
    bindings[1].local = bindings[0].local;
    reject_checked(&checked, &facts);
}
#[test]
fn checked_iflet_suffix_binding_cannot_select_wrong_physical_payload_type() {
    let (mut checked, facts, _, _program) = checked_fixture();
    let MatchPattern::EnumVariant { bindings, .. } = &mut arms_mut(&mut checked)[0].pattern else {
        unreachable!()
    };
    bindings[1].projection[1] = Projection::Field(1);
    reject_checked(&checked, &facts);
}
#[test]
fn ordinary_then_only_name_error_is_not_source_owner_or_unsupported_error() {
    let source = "enum E{A(i32),B} fn f(value:E)->i32{if let E::A(local)=value{local}else{local}}fn main(){}";
    let result = frontend::parse(source).and_then(|program| frontend::check(&program));
    let error = result.expect_err("then-only local leaked into else");
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
fn ordinary_omitted_else_still_requires_unit_branch_join() {
    let source = "fn f(value:bool)->i32{if let true=value{1i32}}fn main(){}";
    let result = frontend::parse(source).and_then(|program| frontend::check(&program));
    let error = result.expect_err("omitted else fabricated non-Unit result");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "wrong boundary: {error}"
    );
}
