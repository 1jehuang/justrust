//! Exact usefulness does not grant authority to covered source or checked rows.
//! All metadata comes from real parsed programs and the original canonical world.
use frontend::{Expr, MatchArm as SourceArm, Pattern, PlanCtx, Program};
use ir::{
    CheckedFunction, ExprKind, FactsCtx, MatchPattern, PatternBinding, PatternTest,
    PatternTestKind, Projection, Ty,
};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

const SOURCE: &str = r#"
enum E { A(bool,bool), B }
enum F { C(bool,bool), D }
enum Outer { Some(E), None }
struct Flags { left:bool, right:bool }
fn product(value:E)->i32 {
    match value { E::A(true,_)=>1,E::A(false,true)=>2,E::A(false,false)=>3,E::B=>4 }
}
fn covered(value:E)->i32 {
    macro_rules! marker { () => { 0 }; }
    match value { E::A(true,_)=>1,E::A(true,flag)=>if flag {2} else {3},E::A(false,_)=>4,E::B=>5 }
}
fn other(value:F)->i32 {
    macro_rules! marker { () => { 0 }; }
    match value { F::C(true,_)=>1,F::C(true,flag)=>if flag {2} else {3},F::C(false,_)=>4,F::D=>5 }
}
fn tuples(value:(bool,bool))->i32 {
    match value {(true,_)=>1,(false,true)=>2,(false,false)=>3}
}
fn arrays(value:[bool;2])->i32 {
    match value {[true,_]=>1,[false,true]=>2,[false,false]=>3}
}
fn records(value:Flags)->i32 {
    match value {Flags{left:true,..}=>1,Flags{left:false,right:true}=>2,Flags{left:false,right:false}=>3}
}
fn nested(value:Outer)->i32 {
    match value {Outer::Some(E::A(true,_))=>1,Outer::Some(E::A(false,true))=>2,Outer::Some(E::A(false,false))=>3,Outer::Some(E::B)=>4,Outer::None=>5}
}
fn integers(value:(i8,bool))->i32 {
    match value {(0,_)=>1,(_,true)=>2,(_,false)=>3}
}
fn conditional(value:E)->i32 {
    if let E::A(true,flag)=value {match (flag,true) {(true,_)=>1,(false,true)=>2,(false,false)=>3}} else {0}
}
fn repeated(value:E)->i32 {
    while let E::A(true,flag)=value {return match (flag,true) {(true,_)=>1,(false,true)=>2,(false,false)=>3};}
    0
}
fn escaping(value:E)->i32 {
    let E::A(true,flag)=value else {return 0;};
    match (flag,true) {(true,_)=>1,(false,true)=>2,(false,false)=>3}
}
fn main() {}
"#;
fn source_index(program: &Program, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap()
}
fn source_arms<'a>(program: &'a Program, name: &str) -> &'a [SourceArm] {
    let Expr::Match { arms, .. } = program.functions[source_index(program, name)]
        .body
        .tail
        .as_deref()
        .unwrap()
    else {
        panic!("actual source match required")
    };
    arms
}
fn source_arms_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut Vec<SourceArm> {
    let index = source_index(program, name);
    let Expr::Match { arms, .. } = program.functions[index].body.tail.as_deref_mut().unwrap()
    else {
        panic!("actual source match required")
    };
    arms
}
fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("actual source constructor products");
    frontend::check(&program).expect("complete real products and covered valid rows check");
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
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
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
    let (checked, facts) = typed(&program, &plan, &budget, "covered");
    let (other, _) = typed(&program, &plan, &budget, "other");
    Fixture {
        checked,
        facts,
        other,
        _program: program,
    }
}
fn covered(function: &CheckedFunction) -> (&Ty, &[PatternTest], &[PatternBinding]) {
    let arms = function
        .exprs
        .iter()
        .find_map(|expression| match &expression.kind {
            ExprKind::Match { arms, .. } => Some(arms),
            _ => None,
        })
        .unwrap();
    assert_eq!(arms.len(), 4, "covered source row must remain represented");
    let MatchPattern::Decision {
        ty,
        tests,
        bindings,
    } = &arms[1].pattern
    else {
        panic!("actual covered decision row required")
    };
    (ty, tests, bindings)
}
fn covered_mut(
    function: &mut CheckedFunction,
) -> (&mut Ty, &mut Vec<PatternTest>, &mut Vec<PatternBinding>) {
    let arms = function
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Match { arms, .. } => Some(arms),
            _ => None,
        })
        .unwrap();
    assert_eq!(arms.len(), 4);
    let MatchPattern::Decision {
        ty,
        tests,
        bindings,
    } = &mut arms[1].pattern
    else {
        panic!("actual covered decision row required")
    };
    (ty, tests, bindings)
}
fn root(tests: &[PatternTest]) -> usize {
    tests
        .iter()
        .position(|test| {
            test.projection.is_empty() && matches!(test.kind, PatternTestKind::EnumVariant { .. })
        })
        .unwrap()
}
fn scalar(tests: &[PatternTest]) -> usize {
    tests
        .iter()
        .position(|test| matches!(test.kind, PatternTestKind::Bool(_)))
        .unwrap()
}
fn reject_ir(function: &CheckedFunction, facts: &ValidatedFacts) {
    let error = function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("coverage cannot authorize malformed dead row metadata");
    assert!(!error.is_empty());
    assert!(
        cfg::lower_with_facts(function, facts, facts.budget()).is_err(),
        "malformed covered row bypassed CFG ingress"
    );
}
fn reject_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("covered foreign source gained planning authority"),
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
fn reject_semantic(source: &str) {
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("covered invalid source was silently skipped");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "wrong boundary: {error}"
    );
}
#[test]
fn proven_constructor_products_keep_actual_source_and_checked_authority() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    for name in [
        "product", "tuples", "arrays", "records", "nested", "integers",
    ] {
        let (checked, facts) = typed(&program, &plan, &budget, name);
        checked
            .clone()
            .validate_with_facts(FactsCtx::Validated(&facts))
            .unwrap();
    }
}
#[test]
fn proven_products_inside_iflet_whilelet_and_letelse_use_real_checked_clients() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    for name in ["conditional", "repeated", "escaping"] {
        let _ = typed(&program, &plan, &budget, name);
    }
}
#[test]
fn covered_decision_row_is_retained_and_validated_not_erased() {
    let fixture = fixture();
    let (domain, tests, bindings) = covered(&fixture.checked);
    assert!(matches!(domain, Ty::Enum(_)));
    assert_eq!(tests.len(), 2);
    assert_eq!(bindings.len(), 1);
}
#[test]
fn subsumed_row_still_imposes_ordinary_result_type_obligation() {
    reject_semantic(
        "fn value(v:(bool,bool))->i32{match v{(true,_)=>1,(true,flag)=>flag,(false,_)=>0}}fn main(){}",
    );
}
#[test]
fn subsumed_row_still_checks_actual_constructor_arity() {
    reject_semantic(
        "enum E{A(bool,bool),B}fn value(v:E)->i32{match v{E::A(true,_)=>1,E::A(true,_,_)=>2,E::A(false,_)=>0,E::B=>3}}fn main(){}",
    );
}
#[test]
fn subsumed_row_still_checks_unresolved_body_names() {
    reject_semantic(
        "fn value(v:(bool,bool))->i32{match v{(true,_)=>1,(true,_)=>missing,(false,_)=>0}}fn main(){}",
    );
}
#[test]
fn omitted_product_cell_remains_non_exhaustive_not_resource_failure() {
    let source = "enum E{A(bool,bool),B}fn value(v:E)->i32{match v{E::A(true,_)=>1,E::A(false,true)=>2,E::B=>0}}fn main(){}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("missing false/false cell was invented");
    assert!(error.contains("non-exhaustive"), "{error}");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "{error}"
    );
}
#[test]
fn integer_other_partition_is_not_covered_by_one_observed_literal() {
    let source = "fn value(v:(i8,bool))->i32{match v{(0,_)=>1,(_,true)=>2}}fn main(){}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("integer remainder lost its uncovered false cell");
    assert!(error.contains("non-exhaustive"), "{error}");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "{error}"
    );
}
#[test]
fn malformed_covered_row_cannot_drop_root_tag_prerequisite() {
    let mut fixture = fixture();
    let tests = covered_mut(&mut fixture.checked).1;
    tests.remove(root(tests));
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn malformed_covered_row_cannot_observe_payload_before_tag() {
    let mut fixture = fixture();
    let tests = covered_mut(&mut fixture.checked).1;
    let root = root(tests);
    let scalar = scalar(tests);
    tests.swap(root, scalar);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn malformed_covered_row_cannot_import_other_actual_enum_tag() {
    let mut fixture = fixture();
    let donor = covered(&fixture.other).1;
    let kind = donor[root(donor)].kind.clone();
    let tests = covered_mut(&mut fixture.checked).1;
    let root = root(tests);
    tests[root].kind = kind;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn malformed_covered_row_cannot_import_same_spelling_foreign_snapshot_tag() {
    let mut fixture = fixture();
    let foreign = self::fixture();
    let donor = covered(&foreign.checked).1;
    let kind = donor[root(donor)].kind.clone();
    let tests = covered_mut(&mut fixture.checked).1;
    let root = root(tests);
    tests[root].kind = kind;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn covered_predicate_field_must_exist_in_its_real_payload() {
    let mut fixture = fixture();
    let tests = covered_mut(&mut fixture.checked).1;
    let scalar = scalar(tests);
    *tests[scalar].projection.last_mut().unwrap() = Projection::Field(99);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn covered_predicate_cannot_reinterpret_payload_field_as_array_index() {
    let mut fixture = fixture();
    let tests = covered_mut(&mut fixture.checked).1;
    let scalar = scalar(tests);
    *tests[scalar].projection.last_mut().unwrap() = Projection::ConstantIndex(0);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn covered_binding_cannot_alias_actual_parameter_storage() {
    let mut fixture = fixture();
    let parameter = fixture.checked.params[0];
    covered_mut(&mut fixture.checked).2[0].local = parameter;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn covered_binding_still_requires_exact_projected_local_type() {
    let mut fixture = fixture();
    let local = covered(&fixture.checked).2[0].local;
    fixture.checked.locals[local.0].ty = Ty::Int(ir::IntTy::I32);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn covered_row_cannot_use_scalar_domain_for_nominal_tag() {
    let mut fixture = fixture();
    *covered_mut(&mut fixture.checked).0 = Ty::Bool;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn covered_row_cannot_repeat_a_predicate_projection() {
    let mut fixture = fixture();
    let tests = covered_mut(&mut fixture.checked).1;
    let duplicate = tests[scalar(tests)].clone();
    tests.push(duplicate);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn foreign_actual_predicate_path_is_rejected_even_when_source_row_is_subsumed() {
    let mut program = parsed();
    let foreign = parsed();
    let donor = source_arms(&foreign, "covered")[1].pattern.clone();
    source_arms_mut(&mut program, "covered")[1].pattern = donor;
    reject_source(program);
}
#[test]
fn foreign_actual_selected_binder_is_rejected_inside_subsumed_row() {
    let mut program = parsed();
    let foreign = parsed();
    let Pattern::Variant { fields, .. } = &source_arms(&foreign, "covered")[1].pattern else {
        panic!("actual variant pattern required")
    };
    let donor = fields[1].1.clone();
    let Pattern::Variant { fields, .. } = &mut source_arms_mut(&mut program, "covered")[1].pattern
    else {
        panic!("actual variant pattern required")
    };
    fields[1].1 = donor;
    reject_source(program);
}
#[test]
fn subsumed_row_cannot_borrow_same_owner_other_function_binding_occurrence() {
    let mut program = parsed();
    let donor = source_arms(&program, "other")[1].body.clone();
    source_arms_mut(&mut program, "covered")[1].body = donor;
    reject_source(program);
}
#[test]
fn unused_function_cannot_import_foreign_covered_body() {
    let mut program = parsed();
    let foreign = parsed();
    let index = source_index(&program, "covered");
    program.functions[index].body = foreign.functions[source_index(&foreign, "covered")]
        .body
        .clone();
    reject_source(program);
}
#[test]
fn source_owner_swap_cannot_be_repaired_by_proven_coverage() {
    let mut program = parsed();
    program.names = parsed().names;
    reject_source(program);
}
#[test]
fn malformed_covered_cfg_tag_is_rejected_at_actual_cfg_validation_ingress() {
    let fixture = fixture();
    let donor = covered(&fixture.other).1;
    let PatternTestKind::EnumVariant { ty, variant } = &donor[root(donor)].kind else {
        unreachable!()
    };
    let variant = ir::VariantId {
        owner: ty.def().id,
        index: *variant,
    };
    let mut lowered =
        cfg::lower_with_facts(&fixture.checked, &fixture.facts, fixture.facts.budget()).unwrap();
    lowered
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .unwrap();
    let target = lowered
        .blocks
        .iter_mut()
        .find_map(|block| match &mut block.terminator.value {
            cfg::Terminator::SwitchVariant { variant, .. } => Some(variant),
            _ => None,
        })
        .expect("real enum tag branch");
    *target = variant;
    let error = lowered
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
        .expect_err("forged nominal CFG tag gained authority");
    assert!(!error.is_empty());
}
