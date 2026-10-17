//! Actual parsed/resolved record-rest source mutations at their public boundary.
//! Owner violations belong to PlanCtx validation, semantic pattern errors to check.
use justrust_engine::compiler::frontend::{self, AstPath, Expr, Pattern, PlanCtx, Program, Stmt};
use justrust_engine::compiler::ir::PlanBudget;
use justrust_engine::compiler::semantic_name::AstIdent;
use std::sync::Arc;

const SOURCE: &str = r#"
struct Record { left:i32, right:i32, ignored:i32 }
struct Other { left:i32, right:i32, ignored:i32, absent:i32 }
fn let_value()->i32 {
    let value=Record{left:1,right:2,ignored:3};
    let Record{left:x,right:y,..}=value;
    0
}
fn match_value()->i32 {
    let value=Record{left:1,right:2,ignored:3};
    match value { Record{left:x,right:y,..}=>0 }
}
fn donor(value:Other) {
    let Other{absent:donated,..}=value;
}
fn main() {}
"#;

fn function(program: &Program, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap_or_else(|| panic!("missing parsed function {name}"))
}

fn pattern<'a>(program: &'a Program, name: &str) -> &'a Pattern {
    let body = &program.functions[function(program, name)].body;
    if let Some(pattern) = body
        .statements
        .iter()
        .find_map(|statement| match statement {
            Stmt::LetPattern { pattern, .. } => Some(pattern),
            _ => None,
        })
    {
        return pattern;
    }
    match body.tail.as_deref() {
        Some(Expr::Match { arms, .. }) => &arms[0].pattern,
        _ => panic!("missing parsed record pattern in {name}"),
    }
}

fn pattern_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut Pattern {
    let index = function(program, name);
    let body = &mut program.functions[index].body;
    if let Some(index) = body
        .statements
        .iter()
        .position(|statement| matches!(statement, Stmt::LetPattern { .. }))
    {
        let Stmt::LetPattern { pattern, .. } = &mut body.statements[index] else {
            unreachable!()
        };
        return pattern;
    }
    match body.tail.as_deref_mut() {
        Some(Expr::Match { arms, .. }) => &mut arms[0].pattern,
        _ => panic!("missing parsed record pattern in {name}"),
    }
}

fn fields_mut(pattern: &mut Pattern) -> &mut Vec<(AstIdent, Pattern)> {
    let Pattern::Struct { fields, rest, .. } = pattern else {
        panic!("expected actual resolved record pattern")
    };
    assert!(*rest, "fixture must retain actual source rest flag");
    fields
}

fn name(pattern: &Pattern) -> AstPath {
    let Pattern::Struct { name, rest, .. } = pattern else {
        panic!("expected actual resolved record pattern")
    };
    assert!(*rest);
    name.clone()
}

fn fixture() -> Program {
    let program = frontend::parse(SOURCE).expect("parse and resolve actual record-rest source");
    for function in ["let_value", "match_value", "donor"] {
        let Pattern::Struct { rest, .. } = pattern(&program, function) else {
            panic!("resolved record pattern required")
        };
        assert!(*rest);
    }
    frontend::check(&program).expect("unmodified actual parsed rest patterns must type-check");
    program
}

fn reject_semantic(program: Program, expected: &str) {
    let error =
        frontend::check(&program).expect_err("invalid mutated record-rest pattern type-checked");
    let diagnostic = error.to_ascii_lowercase();
    assert!(
        !diagnostic.contains("unsupported"),
        "semantic invalidity became admission boundary: {error}"
    );
    assert!(
        diagnostic.contains(expected),
        "wrong rejection, expected {expected}: {error}"
    );
}

fn reject_foreign_before_authority(program: Program) {
    let budget = PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("foreign occurrence acquired planning authority"),
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
fn exact_resolved_record_rest_clone_retains_owners_and_checks() {
    let original = fixture();
    let cloned = original.clone();
    assert!(Arc::ptr_eq(&original.names, &cloned.names));
    assert!(Arc::ptr_eq(&original.modules, &cloned.modules));
    assert!(Arc::ptr_eq(&original.symbols, &cloned.symbols));
    drop(original);
    PlanCtx::from_shared_program(Arc::new(cloned.clone()), &PlanBudget::default()).unwrap();
    frontend::check(&cloned).unwrap();
}

#[test]
fn let_rest_rejects_duplicate_selector_even_when_duplicate_child_is_wildcard() {
    let mut program = fixture();
    let fields = fields_mut(pattern_mut(&mut program, "let_value"));
    fields.push((fields[0].0, Pattern::Wildcard));
    reject_semantic(program, "duplicate");
}

#[test]
fn match_rest_rejects_duplicate_selector_even_when_duplicate_child_is_wildcard() {
    let mut program = fixture();
    let fields = fields_mut(pattern_mut(&mut program, "match_value"));
    fields.push((fields[0].0, Pattern::Wildcard));
    reject_semantic(program, "duplicate");
}

#[test]
fn let_rest_rejects_same_owner_duplicate_child_binding() {
    let mut program = fixture();
    let fields = fields_mut(pattern_mut(&mut program, "let_value"));
    fields[1].1 = fields[0].1.clone();
    reject_semantic(program, "duplicate pattern binding");
}

#[test]
fn match_rest_rejects_same_owner_duplicate_child_binding() {
    let mut program = fixture();
    let fields = fields_mut(pattern_mut(&mut program, "match_value"));
    fields[1].1 = fields[0].1.clone();
    reject_semantic(program, "duplicate pattern binding");
}

#[test]
fn let_rest_cannot_replace_nominal_target_with_other_resolved_source_path() {
    let mut program = fixture();
    let other = name(pattern(&program, "donor"));
    let Pattern::Struct { name, .. } = pattern_mut(&mut program, "let_value") else {
        unreachable!()
    };
    *name = other;
    reject_semantic(program, "match");
}

#[test]
fn match_rest_cannot_replace_nominal_target_with_other_resolved_source_path() {
    let mut program = fixture();
    let other = name(pattern(&program, "donor"));
    let Pattern::Struct { name, .. } = pattern_mut(&mut program, "match_value") else {
        unreachable!()
    };
    *name = other;
    reject_semantic(program, "match");
}

#[test]
fn let_rest_rejects_real_same_owner_member_absent_from_selected_nominal() {
    let mut program = fixture();
    let Pattern::Struct { fields, .. } = pattern(&program, "donor") else {
        unreachable!()
    };
    let absent = fields[0].0;
    fields_mut(pattern_mut(&mut program, "let_value"))[0].0 = absent;
    reject_semantic(program, "field");
}

#[test]
fn let_rest_rejects_foreign_same_spelling_member_occurrence_before_authority() {
    let mut program = fixture();
    let foreign = fixture();
    let Pattern::Struct { fields, .. } = pattern(&foreign, "let_value") else {
        unreachable!()
    };
    let occurrence = fields[0].0;
    fields_mut(pattern_mut(&mut program, "let_value"))[0].0 = occurrence;
    reject_foreign_before_authority(program);
}

#[test]
fn match_rest_rejects_foreign_same_spelling_member_occurrence_before_authority() {
    let mut program = fixture();
    let foreign = fixture();
    let Pattern::Struct { fields, .. } = pattern(&foreign, "match_value") else {
        unreachable!()
    };
    let occurrence = fields[0].0;
    fields_mut(pattern_mut(&mut program, "match_value"))[0].0 = occurrence;
    reject_foreign_before_authority(program);
}

#[test]
fn let_rest_rejects_foreign_actual_child_binding_before_authority() {
    let mut program = fixture();
    let foreign = fixture();
    let Pattern::Struct { fields, .. } = pattern(&foreign, "let_value") else {
        unreachable!()
    };
    let child = fields[0].1.clone();
    assert!(matches!(child, Pattern::Binding { .. }));
    fields_mut(pattern_mut(&mut program, "let_value"))[0].1 = child;
    reject_foreign_before_authority(program);
}

#[test]
fn match_rest_rejects_foreign_actual_child_binding_before_authority() {
    let mut program = fixture();
    let foreign = fixture();
    let Pattern::Struct { fields, .. } = pattern(&foreign, "match_value") else {
        unreachable!()
    };
    let child = fields[0].1.clone();
    assert!(matches!(child, Pattern::Binding { .. }));
    fields_mut(pattern_mut(&mut program, "match_value"))[0].1 = child;
    reject_foreign_before_authority(program);
}
