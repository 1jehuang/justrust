//! Honest prefix/suffix positional-rest AST controls using real parsed handles.
//! Representation retains the rest position rather than fabricating omitted children.
use justrust_engine::compiler::frontend::{
    self, AstPath, Pattern, PlanCtx, PositionalRest, Program, Stmt,
};
use justrust_engine::compiler::ir::PlanBudget;
use std::sync::Arc;

const SOURCE: &str = r#"
struct Triple(i32,bool,i32);
struct Other(i32,bool,i32);
fn tuple_leading()->i32 {let (..,last)=(true,2i32,3i32);last}
fn tuple_middle()->i32 {let (first,..,last)=(1i32,true,3i32);first+last}
fn tuple_trailing()->i32 {let (first,..)=(1i32,true,3i32);first}
fn tuple_zero_omission()->i32 {let (first,..,last)=(1i32,3i32);first+last}
fn nominal_leading()->i32 {let Triple(..,last)=Triple(1,true,3);last}
fn nominal_middle()->i32 {let Triple(first,..,last)=Triple(1,true,3);first+last}
fn nominal_trailing()->i32 {let Triple(first,..)=Triple(1,true,3);first}
fn nominal_zero_omission()->i32 {let Triple(first,middle,..,last)=Triple(1,true,3);first+last}
fn nested()->i32 {let ((..,tail),..)=((true,5i32),false);tail}
fn donor(value:Other)->i32 {let Other(first,..,last)=value;first+last}
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
    program.functions[function(program, name)]
        .body
        .statements
        .iter()
        .find_map(|statement| match statement {
            Stmt::LetPattern { pattern, .. } => Some(pattern),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing actual parsed positional pattern in {name}"))
}

fn pattern_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut Pattern {
    let index = function(program, name);
    program.functions[index]
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            Stmt::LetPattern { pattern, .. } => Some(pattern),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing actual parsed positional pattern in {name}"))
}

fn positional(pattern: &Pattern) -> &PositionalRest {
    match pattern {
        Pattern::TupleRest(fields) | Pattern::VariantRest { fields, .. } => fields,
        _ => panic!("expected honest positional-rest representation"),
    }
}

fn positional_mut(pattern: &mut Pattern) -> &mut PositionalRest {
    match pattern {
        Pattern::TupleRest(fields) | Pattern::VariantRest { fields, .. } => fields,
        _ => panic!("expected honest positional-rest representation"),
    }
}

fn target(pattern: &Pattern) -> AstPath {
    let Pattern::VariantRest { name, .. } = pattern else {
        panic!("expected actual tuple-struct rest pattern")
    };
    name.clone()
}

fn assert_binding_name(program: &Program, pattern: &Pattern, expected: &str) {
    let Pattern::Binding { name, .. } = pattern else {
        panic!("expected actual parsed binder {expected}")
    };
    assert_eq!(program.names.spelling(*name).unwrap(), expected);
}

fn fixture() -> Program {
    let program = frontend::parse(SOURCE).expect("parse and resolve actual positional-rest source");
    frontend::check(&program).expect("actual unmodified positional-rest source must check");
    program
}

fn reject_semantic(program: Program, expected: Option<&str>) {
    let error =
        frontend::check(&program).expect_err("invalid mutated positional-rest source checked");
    let diagnostic = error.to_ascii_lowercase();
    assert!(
        !diagnostic.contains("unsupported"),
        "semantic error became admission boundary: {error}"
    );
    if let Some(expected) = expected {
        assert!(diagnostic.contains(expected), "wrong rejection: {error}");
    }
}

fn reject_foreign_before_authority(program: Program) {
    let budget = PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("foreign positional-rest occurrence acquired planning authority"),
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
fn exact_positional_rest_clone_retains_owners_and_checks() {
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
fn actual_structural_rest_positions_are_retained_without_omitted_wildcards() {
    let program = fixture();
    for (name, prefix, suffix) in [
        ("tuple_leading", 0, 1),
        ("tuple_middle", 1, 1),
        ("tuple_trailing", 1, 0),
        ("tuple_zero_omission", 1, 1),
    ] {
        let pattern = pattern(&program, name);
        assert!(matches!(pattern, Pattern::TupleRest(_)));
        let fields = positional(pattern);
        assert_eq!(fields.prefix.len(), prefix);
        assert_eq!(fields.suffix.len(), suffix);
        for child in &fields.prefix {
            assert_binding_name(&program, child, "first");
        }
        for child in &fields.suffix {
            assert_binding_name(&program, child, "last");
        }
        assert!(
            fields
                .prefix
                .iter()
                .chain(&fields.suffix)
                .all(|child| matches!(child, Pattern::Binding { .. }))
        );
    }
}

#[test]
fn actual_nominal_rest_positions_are_retained_without_fake_selectors() {
    let program = fixture();
    for (name, prefix, suffix) in [
        ("nominal_leading", 0, 1),
        ("nominal_middle", 1, 1),
        ("nominal_trailing", 1, 0),
        ("nominal_zero_omission", 2, 1),
    ] {
        let pattern = pattern(&program, name);
        assert!(matches!(pattern, Pattern::VariantRest { .. }));
        let fields = positional(pattern);
        assert_eq!(fields.prefix.len(), prefix);
        assert_eq!(fields.suffix.len(), suffix);
        for (index, child) in fields.prefix.iter().enumerate() {
            assert_binding_name(&program, child, if index == 0 { "first" } else { "middle" });
        }
        for child in &fields.suffix {
            assert_binding_name(&program, child, "last");
        }
        assert!(
            fields
                .prefix
                .iter()
                .chain(&fields.suffix)
                .all(|child| matches!(child, Pattern::Binding { .. }))
        );
    }
}

#[test]
fn nested_rest_keeps_each_independent_prefix_suffix_boundary() {
    let program = fixture();
    let outer = positional(pattern(&program, "nested"));
    assert_eq!(outer.prefix.len(), 1);
    assert!(outer.suffix.is_empty());
    let inner = positional(&outer.prefix[0]);
    assert!(inner.prefix.is_empty());
    assert_eq!(inner.suffix.len(), 1);
    assert!(matches!(inner.suffix[0], Pattern::Binding { .. }));
    assert_binding_name(&program, &inner.suffix[0], "tail");
}

#[test]
fn structural_rest_rejects_too_many_explicit_children_without_fake_bindings() {
    let mut program = fixture();
    let fields = positional_mut(pattern_mut(&mut program, "tuple_middle"));
    fields.prefix.extend([Pattern::Wildcard, Pattern::Wildcard]);
    reject_semantic(program, None);
}

#[test]
fn nominal_rest_rejects_too_many_explicit_children_without_fake_bindings() {
    let mut program = fixture();
    let fields = positional_mut(pattern_mut(&mut program, "nominal_middle"));
    fields.suffix.extend([Pattern::Wildcard, Pattern::Wildcard]);
    reject_semantic(program, None);
}

#[test]
fn structural_rest_rejects_duplicate_real_binder_across_rest_boundary() {
    let mut program = fixture();
    let fields = positional_mut(pattern_mut(&mut program, "tuple_middle"));
    fields.suffix[0] = fields.prefix[0].clone();
    reject_semantic(program, Some("duplicate"));
}

#[test]
fn nominal_rest_rejects_duplicate_real_binder_across_rest_boundary() {
    let mut program = fixture();
    let fields = positional_mut(pattern_mut(&mut program, "nominal_middle"));
    fields.suffix[0] = fields.prefix[0].clone();
    reject_semantic(program, Some("duplicate"));
}

#[test]
fn nominal_rest_cannot_swap_to_another_real_resolved_nominal_target() {
    let mut program = fixture();
    let other = target(pattern(&program, "donor"));
    let Pattern::VariantRest { name, .. } = pattern_mut(&mut program, "nominal_middle") else {
        unreachable!()
    };
    *name = other;
    reject_semantic(program, Some("match"));
}

#[test]
fn structural_prefix_rejects_foreign_real_same_spelling_child_before_authority() {
    let mut program = fixture();
    let foreign = fixture();
    let child = positional(pattern(&foreign, "tuple_middle")).prefix[0].clone();
    positional_mut(pattern_mut(&mut program, "tuple_middle")).prefix[0] = child;
    reject_foreign_before_authority(program);
}

#[test]
fn structural_suffix_rejects_foreign_real_same_spelling_child_before_authority() {
    let mut program = fixture();
    let foreign = fixture();
    let child = positional(pattern(&foreign, "tuple_middle")).suffix[0].clone();
    positional_mut(pattern_mut(&mut program, "tuple_middle")).suffix[0] = child;
    reject_foreign_before_authority(program);
}

#[test]
fn nominal_prefix_rejects_foreign_real_same_spelling_child_before_authority() {
    let mut program = fixture();
    let foreign = fixture();
    let child = positional(pattern(&foreign, "nominal_middle")).prefix[0].clone();
    positional_mut(pattern_mut(&mut program, "nominal_middle")).prefix[0] = child;
    reject_foreign_before_authority(program);
}

#[test]
fn nominal_suffix_rejects_foreign_real_same_spelling_child_before_authority() {
    let mut program = fixture();
    let foreign = fixture();
    let child = positional(pattern(&foreign, "nominal_middle")).suffix[0].clone();
    positional_mut(pattern_mut(&mut program, "nominal_middle")).suffix[0] = child;
    reject_foreign_before_authority(program);
}

#[test]
fn nominal_rest_rejects_foreign_actual_resolved_path_before_authority() {
    let mut program = fixture();
    let foreign = fixture();
    let foreign_target = target(pattern(&foreign, "nominal_middle"));
    let Pattern::VariantRest { name, .. } = pattern_mut(&mut program, "nominal_middle") else {
        unreachable!()
    };
    *name = foreign_target;
    reject_foreign_before_authority(program);
}
