//! Decision predicates retain real nominal/source owners and exact checked-world facts.
//! Tag observations authorize only their exact successful projection prefixes.
use frontend::{PlanCtx, Program};
use ir::{
    CheckedFunction, ExprKind, FactsCtx, MatchPattern, PatternBinding, PatternTest,
    PatternTestKind, Projection, Ty,
};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

const SOURCE: &str = r#"
enum Inner { I(i32,bool), J }
enum E { A(bool,Inner,i32), B }
enum F { C(bool,Inner,i32), D }
fn select(value:E)->i32 {
    match value {
        E::A(true,Inner::I(7,flag),number)=>if flag {number} else {0},
        E::A(_,_,_)=>1,
        E::B=>2
    }
}
fn other(value:F)->i32 {
    match value {
        F::C(true,Inner::I(7,flag),number)=>if flag {number} else {0},
        F::C(_,_,_)=>1,
        F::D=>2
    }
}
fn dead(value:E)->i32 {
    match value {_=>0,E::A(true,Inner::I(7,flag),number)=>if flag {number} else {0}}
}
fn conditional(value:E)->i32 {
    if let E::A(true,Inner::I(7,flag),number)=value {if flag {number} else {0}} else {1}
}
fn repeated(value:E)->i32 {
    while let E::A(true,Inner::I(7,flag),number)=value {if flag {return number;}break;}
    0
}
fn escaping(value:E)->i32 {
    let E::A(true,Inner::I(7,flag),number)=value else {return 0;};
    if flag {number} else {1}
}
fn main() {}
"#;

fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("real resolved nested refutable source");
    frontend::check(&program)
        .expect("all real decision clients, including unused and dead arms, check");
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
        .unwrap_or_else(|| panic!("missing real callable {name}"));
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
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    (checked, facts)
}
struct Fixture {
    checked: CheckedFunction,
    facts: Arc<ValidatedFacts>,
    other: CheckedFunction,
    dead: CheckedFunction,
    dead_facts: Arc<ValidatedFacts>,
    _program: Program,
}
fn fixture() -> Fixture {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (checked, facts) = typed(&program, &plan, &budget, "select");
    let (other, _) = typed(&program, &plan, &budget, "other");
    let (dead, dead_facts) = typed(&program, &plan, &budget, "dead");
    Fixture {
        checked,
        facts,
        other,
        dead,
        dead_facts,
        _program: program,
    }
}
fn decision(function: &CheckedFunction) -> (&Ty, &[PatternTest], &[PatternBinding]) {
    function
        .exprs
        .iter()
        .find_map(|expression| match &expression.kind {
            ExprKind::Match { arms, .. } => arms.iter().find_map(|arm| match &arm.pattern {
                MatchPattern::Decision {
                    ty,
                    tests,
                    bindings,
                } => Some((ty, tests.as_slice(), bindings.as_slice())),
                _ => None,
            }),
            _ => None,
        })
        .expect("real checked decision match row")
}
fn decision_mut(
    function: &mut CheckedFunction,
) -> (&mut Ty, &mut Vec<PatternTest>, &mut Vec<PatternBinding>) {
    function
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Match { arms, .. } => {
                arms.iter_mut().find_map(|arm| match &mut arm.pattern {
                    MatchPattern::Decision {
                        ty,
                        tests,
                        bindings,
                    } => Some((ty, tests, bindings)),
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("real checked decision match row")
}
fn root_tag(tests: &[PatternTest]) -> usize {
    tests
        .iter()
        .position(|test| {
            test.projection.is_empty() && matches!(test.kind, PatternTestKind::EnumVariant { .. })
        })
        .unwrap()
}
fn nested_tag(tests: &[PatternTest]) -> usize {
    tests
        .iter()
        .position(|test| {
            !test.projection.is_empty() && matches!(test.kind, PatternTestKind::EnumVariant { .. })
        })
        .unwrap()
}
fn boolean(tests: &[PatternTest]) -> usize {
    tests
        .iter()
        .position(|test| matches!(test.kind, PatternTestKind::Bool(_)))
        .unwrap()
}
fn integer(tests: &[PatternTest]) -> usize {
    tests
        .iter()
        .position(|test| matches!(test.kind, PatternTestKind::Int(_)))
        .unwrap()
}
fn reject_ir(function: &CheckedFunction, facts: &ValidatedFacts) {
    let error = function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("forged ordered decision gained IR authority");
    assert!(!error.is_empty());
    assert!(
        cfg::lower_with_facts(function, facts, facts.budget()).is_err(),
        "malformed decision bypassed validated CFG ingress"
    );
}

#[test]
fn real_nested_decision_clone_validates_under_exact_original_facts() {
    let fixture = fixture();
    let (domain, tests, bindings) = decision(&fixture.checked);
    assert!(matches!(domain, Ty::Enum(_)));
    assert_eq!(tests.len(), 4);
    assert_eq!(bindings.len(), 2);
    let root = root_tag(tests);
    let nested = nested_tag(tests);
    assert!(root < nested);
    assert!(nested < integer(tests));
    assert!(root < boolean(tests));
    fixture
        .checked
        .clone()
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .unwrap();
}
#[test]
fn real_decision_clients_share_match_iflet_whilelet_and_letelse_checked_ingress() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    for name in ["select", "conditional", "repeated", "escaping"] {
        let (checked, _) = typed(&program, &plan, &budget, name);
        let expression_decision = checked
            .exprs
            .iter()
            .any(|expression| match &expression.kind {
                ExprKind::Match { arms, .. } => arms
                    .iter()
                    .any(|arm| matches!(arm.pattern, MatchPattern::Decision { .. })),
                _ => false,
            });
        let statement_decision = checked.body.statements.iter().any(|statement| {
            matches!(
                statement,
                ir::Stmt::LetElse {
                    pattern: MatchPattern::Decision { .. },
                    ..
                }
            )
        });
        assert!(
            expression_decision || statement_decision,
            "{name} lost its genuine decision plan"
        );
    }
}
#[test]
fn empty_test_list_cannot_claim_decision_authority() {
    let mut fixture = fixture();
    decision_mut(&mut fixture.checked).1.clear();
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn nested_projection_requires_original_root_tag() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    tests.remove(root_tag(tests));
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn nested_payload_observation_cannot_precede_its_ancestor_tag() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let root = root_tag(tests);
    let nested = nested_tag(tests);
    tests.swap(root, nested);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn scalar_payload_observation_requires_earlier_exact_nested_tag() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let scalar = integer(tests);
    let nested = nested_tag(tests);
    tests.swap(scalar, nested);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn nested_binding_cannot_use_a_removed_tag_even_without_scalar_test() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    tests.remove(integer(tests));
    tests.remove(nested_tag(tests));
    assert!(decision(&fixture.checked).2.iter().any(|binding| {
        binding
            .projection
            .iter()
            .filter(|step| matches!(step, Projection::Downcast(_)))
            .count()
            == 2
    }));
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn sibling_projection_does_not_inherit_another_prefix_tag() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let nested = nested_tag(tests);
    let field = tests[nested]
        .projection
        .iter_mut()
        .find(|step| matches!(step, Projection::Field(_)))
        .unwrap();
    *field = Projection::Field(0);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn same_world_other_enum_tag_cannot_authorize_original_root() {
    let mut fixture = fixture();
    let donor_tests = decision(&fixture.other).1;
    let donor = donor_tests[root_tag(donor_tests)].kind.clone();
    let tests = decision_mut(&mut fixture.checked).1;
    let root = root_tag(tests);
    tests[root].kind = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn same_spelling_foreign_snapshot_tag_cannot_authorize_original_root() {
    let mut fixture = fixture();
    let foreign = self::fixture();
    let donor_tests = decision(&foreign.checked).1;
    let donor = donor_tests[root_tag(donor_tests)].kind.clone();
    let tests = decision_mut(&mut fixture.checked).1;
    let root = root_tag(tests);
    tests[root].kind = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn nested_tag_requires_actual_projected_enum_type() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let root_kind = tests[root_tag(tests)].kind.clone();
    let nested = nested_tag(tests);
    tests[nested].kind = root_kind;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn tag_variant_index_is_checked_even_in_dead_arm() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.dead).1;
    let root = root_tag(tests);
    let PatternTestKind::EnumVariant { ty, variant } = &mut tests[root].kind else {
        unreachable!()
    };
    *variant = ty.def().variants.len();
    reject_ir(&fixture.dead, &fixture.dead_facts);
}
#[test]
fn boolean_predicate_cannot_observe_integer_payload() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let kind = tests[boolean(tests)].kind.clone();
    let index = integer(tests);
    tests[index].kind = kind;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn integer_predicate_cannot_observe_boolean_payload() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let kind = tests[integer(tests)].kind.clone();
    let index = boolean(tests);
    tests[index].kind = kind;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn repeated_test_projection_is_not_a_second_tag_certificate() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let duplicate = tests[nested_tag(tests)].clone();
    tests.push(duplicate);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn observation_field_index_must_exist_in_the_actual_payload() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let index = integer(tests);
    *tests[index].projection.last_mut().unwrap() = Projection::Field(999);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn observation_constant_index_cannot_replace_nominal_payload_field() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let index = integer(tests);
    *tests[index].projection.last_mut().unwrap() = Projection::ConstantIndex(0);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn dereference_is_not_a_decision_observation_projection() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let index = boolean(tests);
    tests[index].projection.push(Projection::Deref);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn downcast_variant_must_match_its_actual_tag_prerequisite() {
    let mut fixture = fixture();
    let tests = decision_mut(&mut fixture.checked).1;
    let index = integer(tests);
    let downcast = tests[index]
        .projection
        .iter_mut()
        .rev()
        .find_map(|step| match step {
            Projection::Downcast(variant) => Some(variant),
            _ => None,
        })
        .unwrap();
    downcast.index = 1;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn decision_domain_cannot_be_replaced_by_scalar_metadata() {
    let mut fixture = fixture();
    *decision_mut(&mut fixture.checked).0 = Ty::Bool;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn decision_binding_must_be_a_fresh_nonparameter_local() {
    let mut fixture = fixture();
    let parameter = fixture.checked.params[0];
    decision_mut(&mut fixture.checked).2[0].local = parameter;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn decision_bindings_cannot_publish_the_same_local_twice() {
    let mut fixture = fixture();
    let bindings = decision_mut(&mut fixture.checked).2;
    bindings[1].local = bindings[0].local;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn decision_bindings_cannot_reuse_another_selected_field_type() {
    let mut fixture = fixture();
    let bindings = decision_mut(&mut fixture.checked).2;
    bindings[1].projection = bindings[0].projection.clone();
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn selected_local_metadata_must_match_actual_projected_payload_type() {
    let mut fixture = fixture();
    let selected = decision(&fixture.checked).2[0].local;
    let ty = &mut fixture.checked.locals[selected.0].ty;
    *ty = if *ty == Ty::Bool {
        Ty::Int(ir::IntTy::I32)
    } else {
        Ty::Bool
    };
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn unused_function_cannot_import_a_foreign_decision_source_body() {
    let mut program = parsed();
    let foreign = parsed();
    let index = program
        .functions
        .iter()
        .position(|function| program.names.spelling(function.name.segments()[0]).unwrap() == "dead")
        .unwrap();
    let donor = foreign
        .functions
        .iter()
        .find(|function| foreign.names.spelling(function.name.segments()[0]).unwrap() == "dead")
        .unwrap();
    program.functions[index].body = donor.body.clone();
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("foreign unused pattern source gained authority"),
    };
    let error = error.to_ascii_lowercase();
    assert!(
        error.contains("contract") || error.contains("foreign"),
        "{error}"
    );
    assert!(!error.contains("unsupported"), "{error}");
    assert_eq!(budget.usage_snapshot().applications, 0);
    assert_eq!(budget.usage_snapshot().obligations, 0);
}
#[test]
fn cfg_tag_test_cannot_import_other_actual_enum_owner() {
    let fixture = fixture();
    let donor_tests = decision(&fixture.other).1;
    let PatternTestKind::EnumVariant { ty, variant } = &donor_tests[root_tag(donor_tests)].kind
    else {
        unreachable!()
    };
    let donor = ir::VariantId {
        owner: ty.def().id,
        index: *variant,
    };
    let mut lowered =
        cfg::lower_with_facts(&fixture.checked, &fixture.facts, fixture.facts.budget()).unwrap();
    lowered
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .unwrap();
    let variant = lowered
        .blocks
        .iter_mut()
        .find_map(|block| match &mut block.terminator.value {
            cfg::Terminator::SwitchVariant { variant, .. } => Some(variant),
            _ => None,
        })
        .expect("real short-circuit enum tag branch");
    *variant = donor;
    let error = lowered
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .expect_err("foreign nominal tag gained CFG observation authority");
    assert!(!error.is_empty());
}
