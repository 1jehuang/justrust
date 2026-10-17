//! Enum-rest metadata corruption using only real parsed and checked nominal owners.
//! No synthetic nominal factory, source handle, or resolution certificate is used.
use frontend::{Expr, Pattern, PlanCtx, Program, Stmt};
use ir::{
    CheckedFunction, ExprKind, FactsCtx, MatchArm, MatchPattern, PatternBinding, Projection, Ty,
};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

const SOURCE: &str = r#"
enum E { A(i32,bool,i32), B(i32,i32) }
enum F { C(i32,bool,i32), D(i32,i32) }
enum Single { Only(i32,bool,i32) }
fn select(value:E)->i32 {
    match value { E::A(first,..,last)=>first+last, E::B(first,..,last)=>first+last }
}
fn other(value:F)->i32 {
    match value { F::C(first,..,last)=>first+last, F::D(first,..,last)=>first+last }
}
fn single(value:Single)->i32 {let Single::Only(first,..,last)=value;first+last}
fn dead(value:E)->i32 {match value {_=>0,E::A(first,..,last)=>first+last}}
fn main() {}
"#;

fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("actual resolved tuple-payload enum-rest source");
    frontend::check(&program)
        .expect("all actual enum-rest bodies, including unused and dead arms, check");
    program
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
        .unwrap_or_else(|| panic!("missing actual callable {name}"));
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

struct Fixture {
    checked: CheckedFunction,
    facts: Arc<ValidatedFacts>,
    other: CheckedFunction,
    single: CheckedFunction,
    single_facts: Arc<ValidatedFacts>,
    dead: CheckedFunction,
    dead_facts: Arc<ValidatedFacts>,
    // Retain the actual source owner alongside checked nominal metadata.
    _program: Program,
}

fn fixture() -> Fixture {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (checked, facts) = typed(&program, &plan, &budget, "select");
    let (other, _) = typed(&program, &plan, &budget, "other");
    let (single, single_facts) = typed(&program, &plan, &budget, "single");
    let (dead, dead_facts) = typed(&program, &plan, &budget, "dead");
    Fixture {
        checked,
        facts,
        other,
        single,
        single_facts,
        dead,
        dead_facts,
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
        .expect("real checked match")
}

fn arms_mut(function: &mut CheckedFunction) -> &mut Vec<MatchArm> {
    function
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Match { arms, .. } => Some(arms),
            _ => None,
        })
        .expect("real checked match")
}

fn payload(function: &CheckedFunction, index: usize) -> (&ir::EnumTy, usize, &[PatternBinding]) {
    match &arms(function)[index].pattern {
        MatchPattern::EnumVariant {
            ty,
            variant,
            bindings,
        } => (ty, *variant, bindings),
        _ => panic!("actual checked enum variant required"),
    }
}

fn payload_mut(
    function: &mut CheckedFunction,
    index: usize,
) -> (&mut ir::EnumTy, &mut usize, &mut Vec<PatternBinding>) {
    match &mut arms_mut(function)[index].pattern {
        MatchPattern::EnumVariant {
            ty,
            variant,
            bindings,
        } => (ty, variant, bindings),
        _ => panic!("actual checked enum variant required"),
    }
}

fn let_bindings(function: &CheckedFunction) -> (&Ty, &[PatternBinding]) {
    function
        .body
        .statements
        .iter()
        .find_map(|statement| match statement {
            ir::Stmt::LetPattern { ty, bindings, .. } => Some((ty, bindings.as_slice())),
            _ => None,
        })
        .expect("real checked single-variant let")
}

fn let_bindings_mut(function: &mut CheckedFunction) -> &mut Vec<PatternBinding> {
    function
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            ir::Stmt::LetPattern { bindings, .. } => Some(bindings),
            _ => None,
        })
        .expect("real checked single-variant let")
}

fn reject_checked(function: &CheckedFunction, facts: &ValidatedFacts) {
    let error = function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("corrupted enum metadata gained validation authority");
    assert!(!error.is_empty());
    assert!(
        cfg::lower_with_facts(function, facts, facts.budget()).is_err(),
        "malformed enum metadata bypassed CFG ingress validation"
    );
}

#[test]
fn actual_enum_rest_checked_clone_and_facts_validate_and_lower() {
    let fixture = fixture();
    let cloned = fixture.checked.clone();
    cloned
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .unwrap();
    cfg::lower_with_facts(&cloned, &fixture.facts, fixture.facts.budget())
        .unwrap()
        .validate()
        .unwrap();
}

#[test]
fn actual_suffix_bindings_retain_owner_variant_and_physical_payload_indices() {
    let fixture = fixture();
    for (arm, last_field) in [(0, 2), (1, 1)] {
        let (owner, variant, bindings) = payload(&fixture.checked, arm);
        assert_eq!(bindings.len(), 2, "omitted fields must not invent binders");
        for (binding, field) in bindings.iter().zip([0, last_field]) {
            assert_eq!(
                binding.projection,
                vec![
                    Projection::Downcast(ir::VariantId {
                        owner: owner.def().id,
                        index: variant
                    }),
                    Projection::Field(field)
                ]
            );
            assert!(!fixture.checked.params.contains(&binding.local));
        }
        assert_ne!(bindings[0].local, bindings[1].local);
    }
}

#[test]
fn actual_single_variant_let_retains_enum_downcast_not_plain_aggregate_fields() {
    let fixture = fixture();
    let (ty, bindings) = let_bindings(&fixture.single);
    let Ty::Enum(owner) = ty else {
        panic!("single enum nominal required")
    };
    assert_eq!(owner.def().variants.len(), 1);
    assert_eq!(bindings.len(), 2);
    for (binding, field) in bindings.iter().zip([0, 2]) {
        assert_eq!(
            binding.projection,
            vec![
                Projection::Downcast(ir::VariantId {
                    owner: owner.def().id,
                    index: 0
                }),
                Projection::Field(field)
            ]
        );
        assert!(!fixture.single.params.contains(&binding.local));
    }
}

#[test]
fn enum_rest_target_cannot_be_another_actual_nominal_owner() {
    let mut fixture = fixture();
    let foreign_owner = payload(&fixture.other, 0).0.clone();
    *payload_mut(&mut fixture.checked, 0).0 = foreign_owner;
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_variant_target_cannot_disagree_with_retained_payload_downcast() {
    let mut fixture = fixture();
    let other_variant = payload(&fixture.checked, 1).1;
    *payload_mut(&mut fixture.checked, 0).1 = other_variant;
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_downcast_cannot_use_another_actual_owner() {
    let mut fixture = fixture();
    let (other, variant, _) = payload(&fixture.other, 0);
    let wrong = ir::VariantId {
        owner: other.def().id,
        index: variant,
    };
    payload_mut(&mut fixture.checked, 0).2[0].projection[0] = Projection::Downcast(wrong);
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_downcast_cannot_use_another_existing_variant() {
    let mut fixture = fixture();
    let (owner, variant, _) = payload(&fixture.checked, 1);
    let wrong = ir::VariantId {
        owner: owner.def().id,
        index: variant,
    };
    payload_mut(&mut fixture.checked, 0).2[0].projection[0] = Projection::Downcast(wrong);
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_suffix_field_must_have_exact_selected_payload_type() {
    let mut fixture = fixture();
    payload_mut(&mut fixture.checked, 0).2[1].projection[1] = Projection::Field(1);
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_suffix_field_must_be_within_actual_payload() {
    let mut fixture = fixture();
    payload_mut(&mut fixture.checked, 0).2[1].projection[1] = Projection::Field(usize::MAX);
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_cannot_skip_genuine_enum_downcast() {
    let mut fixture = fixture();
    payload_mut(&mut fixture.checked, 0).2[0]
        .projection
        .remove(0);
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_cannot_reverse_downcast_and_field_order() {
    let mut fixture = fixture();
    payload_mut(&mut fixture.checked, 0).2[0]
        .projection
        .swap(0, 1);
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_cannot_continue_field_projection_through_scalar_leaf() {
    let mut fixture = fixture();
    payload_mut(&mut fixture.checked, 0).2[0]
        .projection
        .push(Projection::Field(0));
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_payload_local_type_cannot_disagree_with_actual_field() {
    let mut fixture = fixture();
    let local = payload(&fixture.checked, 0).2[0].local;
    fixture.checked.locals[local.0].ty = Ty::Bool;
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_payload_binding_cannot_reuse_actual_parameter() {
    let mut fixture = fixture();
    let parameter = fixture.checked.params[0];
    payload_mut(&mut fixture.checked, 0).2[0].local = parameter;
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_payload_binding_requires_valid_local() {
    let mut fixture = fixture();
    payload_mut(&mut fixture.checked, 0).2[0].local = ir::LocalId(usize::MAX);
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_payload_bindings_cannot_share_one_local() {
    let mut fixture = fixture();
    let bindings = payload_mut(&mut fixture.checked, 0).2;
    bindings[1].local = bindings[0].local;
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn enum_rest_payload_bindings_cannot_share_local_across_arms() {
    let mut fixture = fixture();
    let local = payload(&fixture.checked, 0).2[0].local;
    payload_mut(&mut fixture.checked, 1).2[0].local = local;
    reject_checked(&fixture.checked, &fixture.facts);
}

#[test]
fn actual_unreachable_enum_rest_arm_still_validates_payload_metadata() {
    let mut fixture = fixture();
    assert!(matches!(
        arms(&fixture.dead)[0].pattern,
        MatchPattern::Wildcard
    ));
    payload_mut(&mut fixture.dead, 1).2[0].projection[1] = Projection::Field(usize::MAX);
    reject_checked(&fixture.dead, &fixture.dead_facts);
}

#[test]
fn single_variant_let_cannot_borrow_actual_other_enum_downcast_owner() {
    let mut fixture = fixture();
    let (other, variant, _) = payload(&fixture.other, 0);
    let wrong = ir::VariantId {
        owner: other.def().id,
        index: variant,
    };
    let_bindings_mut(&mut fixture.single)[0].projection[0] = Projection::Downcast(wrong);
    reject_checked(&fixture.single, &fixture.single_facts);
}

#[test]
fn single_variant_let_cannot_claim_out_of_range_payload_field() {
    let mut fixture = fixture();
    let_bindings_mut(&mut fixture.single)[1].projection[1] = Projection::Field(usize::MAX);
    reject_checked(&fixture.single, &fixture.single_facts);
}

#[test]
fn single_variant_let_payload_cannot_reuse_actual_parameter_local() {
    let mut fixture = fixture();
    let parameter = fixture.single.params[0];
    let_bindings_mut(&mut fixture.single)[0].local = parameter;
    reject_checked(&fixture.single, &fixture.single_facts);
}

fn source_function(program: &Program, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap()
}

fn source_pattern<'a>(program: &'a Program, name: &str) -> &'a Pattern {
    let body = &program.functions[source_function(program, name)].body;
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
    let Some(Expr::Match { arms, .. }) = body.tail.as_deref() else {
        panic!("actual source match required")
    };
    &arms[0].pattern
}

fn source_pattern_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut Pattern {
    let index = source_function(program, name);
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
    let Some(Expr::Match { arms, .. }) = body.tail.as_deref_mut() else {
        panic!("actual source match required")
    };
    &mut arms[0].pattern
}

fn reject_foreign_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("foreign enum-rest source occurrence acquired planning authority"),
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
fn source_enum_rest_prefix_rejects_foreign_actual_child_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Pattern::VariantRest { fields, .. } = source_pattern(&foreign, "select") else {
        panic!("honest enum-rest source required")
    };
    let child = fields.prefix[0].clone();
    let Pattern::VariantRest { fields, .. } = source_pattern_mut(&mut program, "select") else {
        unreachable!()
    };
    fields.prefix[0] = child;
    reject_foreign_source(program);
}

#[test]
fn source_enum_rest_suffix_rejects_foreign_actual_child_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Pattern::VariantRest { fields, .. } = source_pattern(&foreign, "select") else {
        panic!("honest enum-rest source required")
    };
    let child = fields.suffix[0].clone();
    let Pattern::VariantRest { fields, .. } = source_pattern_mut(&mut program, "select") else {
        unreachable!()
    };
    fields.suffix[0] = child;
    reject_foreign_source(program);
}

#[test]
fn source_enum_rest_rejects_foreign_actual_target_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Pattern::VariantRest { name, .. } = source_pattern(&foreign, "select") else {
        panic!("honest enum-rest source required")
    };
    let foreign_name = name.clone();
    let Pattern::VariantRest { name, .. } = source_pattern_mut(&mut program, "select") else {
        unreachable!()
    };
    *name = foreign_name;
    reject_foreign_source(program);
}

#[test]
fn source_single_variant_let_rejects_foreign_actual_suffix_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Pattern::VariantRest { fields, .. } = source_pattern(&foreign, "single") else {
        panic!("honest single-variant rest source required")
    };
    let child = fields.suffix[0].clone();
    let Pattern::VariantRest { fields, .. } = source_pattern_mut(&mut program, "single") else {
        unreachable!()
    };
    fields.suffix[0] = child;
    reject_foreign_source(program);
}

#[test]
fn source_single_variant_let_rejects_foreign_actual_target_before_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let Pattern::VariantRest { name, .. } = source_pattern(&foreign, "single") else {
        panic!("honest single-variant rest source required")
    };
    let foreign_name = name.clone();
    let Pattern::VariantRest { name, .. } = source_pattern_mut(&mut program, "single") else {
        unreachable!()
    };
    *name = foreign_name;
    reject_foreign_source(program);
}

#[test]
fn source_enum_rest_actual_other_nominal_target_is_semantic_not_plan_contract() {
    let mut program = parsed();
    let Pattern::VariantRest { name, .. } = source_pattern(&program, "other") else {
        panic!("honest enum-rest source required")
    };
    let other_name = name.clone();
    let Pattern::VariantRest { name, .. } = source_pattern_mut(&mut program, "select") else {
        unreachable!()
    };
    *name = other_name;
    let error = frontend::check(&program).expect_err("wrong actual enum source target checked");
    let diagnostic = error.to_ascii_lowercase();
    assert!(
        !diagnostic.contains("unsupported"),
        "wrong boundary: {error}"
    );
    assert!(diagnostic.contains("match"), "wrong rejection: {error}");
}
