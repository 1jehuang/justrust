//! Shared typed-slice authority uses actual source, canonical facts and real loans.
//! Universal templates are never passed to Closed-only CFG entry points.
use frontend::{Expr, PlanCtx, Program, Type};
use ir::{BorrowKind, CheckedFunction, ExprKind, FactsCtx, Ty};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{self, cfg, frontend, ir, ownership};
use std::sync::Arc;

const SOURCE: &str = r#"
struct Token {value:i32}
fn integers(values:&[i32],index:usize,flag:bool)->i32 {if flag {values[index]} else {values[index]}}
fn wide(values:&[i128],index:usize)->i128 {values[index]}
fn floats(values:&[f64],index:usize)->f64 {values[index]}
fn aggregates(values:&[(i32,bool)],index:usize)->(i32,bool) {values[index]}
fn bytes(values:&[u8],index:usize)->u8 {values[index]}
fn generic<T:Copy>(values:&[T],index:usize)->T {values[index]}
fn length<T>(values:&[T])->usize {values.len()}
fn noncopy(values:&[Token],index:usize)->usize {index;values.len();9usize}
fn unsize()->i32 {let values=[1i32,2i32];let slice:&[i32]=&values;slice.len();slice[0usize]}
fn unsize_bool()->bool {let values=[true,false];let slice:&[bool]=&values;slice[0usize]}
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
    let program = frontend::parse(SOURCE).expect("actual resolved typed shared-slice source");
    frontend::check(&program)
        .expect("all genuine typed/generic slices and non-Copy length bodies check");
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
    let facts = plan.callable_facts(declaration).unwrap();
    checked
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    let template = cfg::lower_template(Arc::new(checked.clone()), facts.clone(), budget).unwrap();
    template
        .function()
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    ownership::verify_template(template).expect("same-facts universal slice ownership baseline");
    (checked, facts)
}
struct Fixture {
    checked: CheckedFunction,
    facts: Arc<ValidatedFacts>,
    _program: Program,
}
fn fixture(name: &str) -> Fixture {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (checked, facts) = typed(&program, &plan, &budget, name);
    Fixture {
        checked,
        facts,
        _program: program,
    }
}
fn first_index(function: &CheckedFunction) -> usize {
    function
        .exprs
        .iter()
        .position(|expression| matches!(expression.kind, ExprKind::SliceIndex { .. }))
        .expect("actual typed slice indexing expression")
}
fn first_len(function: &CheckedFunction) -> usize {
    function
        .exprs
        .iter()
        .position(|expression| matches!(expression.kind, ExprKind::SliceLen { .. }))
        .expect("actual len occurrence")
}
fn first_unsize(function: &CheckedFunction) -> usize {
    function
        .exprs
        .iter()
        .position(|expression| matches!(expression.kind, ExprKind::ArrayToSlice { .. }))
        .expect("actual original array unsizing")
}
fn slice_element(ty: &Ty) -> &Ty {
    let Ty::Ref {
        kind: BorrowKind::Shared,
        referent,
    } = ty
    else {
        panic!("real shared slice reference required")
    };
    let Ty::Slice { element } = referent.as_ref() else {
        panic!("real unsized element type required")
    };
    element
}
fn reject_ir(function: &CheckedFunction, facts: &Arc<ValidatedFacts>) {
    let error = function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("malformed slice metadata gained IR authority");
    assert!(!error.is_empty());
    assert!(
        cfg::lower_template(Arc::new(function.clone()), facts.clone(), facts.budget()).is_err(),
        "malformed slice bypassed same-phase CFG ingress"
    );
}
fn reject_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("foreign slice source gained planning authority"),
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
fn element_source_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut Type {
    let index = index(program, name);
    let Type::Ref { referent, .. } = &mut program.functions[index].params[0].1 else {
        panic!("actual reference parameter required")
    };
    let Type::Slice(element) = referent.as_mut() else {
        panic!("actual source slice element required")
    };
    element
}
fn observed_generic_argument_unsupported(source: &str, expected: &str) {
    for check in [true, false] {
        let failure = compiler::compile_observed(source.to_owned(), 1, check)
            .err()
            .expect("closed generic-argument boundary unexpectedly admitted source");
        let error = failure.to_string().to_ascii_lowercase();
        assert!(error.contains(expected), "{error}");
        assert!(
            !failure.resource_usage.poisoned,
            "generic-argument rejection was resource failure"
        );
    }
}
fn observed_unsupported(source: &str) {
    for check in [true, false] {
        let failure = compiler::compile_observed(source.to_owned(), 1, check)
            .err()
            .expect("forbidden concrete element acquired native layout authority");
        let error = failure.to_string().to_ascii_lowercase();
        assert!(error.contains("unsupported"), "{error}");
        assert!(
            error.contains("slice") || error.contains("element"),
            "{error}"
        );
        assert!(
            !failure.resource_usage.poisoned,
            "unsupported element was resource failure"
        );
    }
}
#[test]
fn real_scalar_wide_float_copy_aggregate_and_generic_slice_baselines() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    for name in [
        "integers",
        "wide",
        "floats",
        "aggregates",
        "bytes",
        "generic",
        "length",
        "noncopy",
        "unsize",
        "unsize_bool",
    ] {
        let (checked, facts) = typed(&program, &plan, &budget, name);
        checked
            .clone()
            .validate_with_facts(FactsCtx::Validated(&facts))
            .unwrap();
    }
}
#[test]
fn generic_index_has_actual_rigid_element_and_original_copy_permission() {
    let fixture = fixture("generic");
    let element = slice_element(&fixture.checked.locals[fixture.checked.params[0].0].ty);
    assert!(matches!(element, Ty::Param(_)));
    assert!(fixture.facts.must_copy(element).unwrap());
    assert_eq!(
        &fixture.checked.exprs[first_index(&fixture.checked)].ty,
        element
    );
}
#[test]
fn copy_aggregate_index_returns_full_actual_aggregate_type() {
    let fixture = fixture("aggregates");
    let element = slice_element(&fixture.checked.locals[fixture.checked.params[0].0].ty);
    assert!(matches!(element,Ty::Tuple(fields) if fields.len()==2));
    assert!(fixture.facts.must_copy(element).unwrap());
    assert_eq!(
        &fixture.checked.exprs[first_index(&fixture.checked)].ty,
        element
    );
}
#[test]
fn noncopy_element_length_does_not_require_or_grant_copy() {
    let fixture = fixture("noncopy");
    let element = slice_element(&fixture.checked.locals[fixture.checked.params[0].0].ty);
    assert!(matches!(element, Ty::Struct(_)));
    assert!(!fixture.facts.must_copy(element).unwrap());
    assert_eq!(
        fixture.checked.exprs[first_len(&fixture.checked)].ty,
        Ty::Int(ir::IntTy::Usize)
    );
}
#[test]
fn byte_only_helpers_remain_exact_after_general_slice_admission() {
    let integer = fixture("integers");
    let byte = fixture("bytes");
    let integer_ty = &integer.checked.locals[integer.checked.params[0].0].ty;
    let byte_ty = &byte.checked.locals[byte.checked.params[0].0].ty;
    assert!(!integer_ty.shared_byte_slice());
    assert!(byte_ty.shared_byte_slice());
    assert_eq!(
        integer_ty.shared_slice_element(),
        Some(&Ty::Int(ir::IntTy::I32))
    );
    assert_eq!(
        byte_ty.shared_slice_element(),
        Some(&Ty::Int(ir::IntTy::U8))
    );
}
#[test]
fn array_unsizing_records_element_identity_and_element_count_not_byte_count() {
    let fixture = fixture("unsize");
    let expression = &fixture.checked.exprs[first_unsize(&fixture.checked)];
    let ExprKind::ArrayToSlice { reference } = expression.kind else {
        unreachable!()
    };
    let source = &fixture.checked.exprs[reference.0].ty;
    let (element, len) = source.shared_array_element_len().unwrap();
    assert_eq!(element, &Ty::Int(ir::IntTy::I32));
    assert_eq!(len, 2);
    assert_eq!(source.shared_byte_array_len(), None);
    assert_eq!(expression.ty.shared_slice_element(), Some(element));
}
#[test]
fn array_unsizing_cannot_reinterpret_actual_i32_as_bool_elements() {
    let mut fixture = fixture("unsize");
    let donor = self::fixture("unsize_bool");
    let ty = donor.checked.exprs[first_unsize(&donor.checked)].ty.clone();
    let index = first_unsize(&fixture.checked);
    fixture.checked.exprs[index].ty = ty;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn slice_index_cannot_change_exact_result_to_another_scalar_type() {
    let mut fixture = fixture("integers");
    let index = first_index(&fixture.checked);
    fixture.checked.exprs[index].ty = Ty::Bool;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn wide_index_cannot_truncate_checked_result_to_i32() {
    let mut fixture = fixture("wide");
    let index = first_index(&fixture.checked);
    fixture.checked.exprs[index].ty = Ty::Int(ir::IntTy::I32);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn aggregate_index_cannot_be_replaced_by_slice_descriptor_result() {
    let mut fixture = fixture("aggregates");
    let source = fixture.checked.locals[fixture.checked.params[0].0]
        .ty
        .clone();
    let index = first_index(&fixture.checked);
    fixture.checked.exprs[index].ty = source;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn slice_index_requires_actual_usize_not_an_existing_bool_operand() {
    let mut fixture = fixture("integers");
    let wrong = fixture
        .checked
        .exprs
        .iter()
        .position(|expression| expression.ty == Ty::Bool)
        .unwrap();
    let index = first_index(&fixture.checked);
    let ExprKind::SliceIndex { index, .. } = &mut fixture.checked.exprs[index].kind else {
        unreachable!()
    };
    *index = ir::ExprId(wrong);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn slice_length_result_cannot_be_signed_i32() {
    let mut fixture = fixture("length");
    let index = first_len(&fixture.checked);
    fixture.checked.exprs[index].ty = Ty::Int(ir::IntTy::I32);
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn slice_length_cannot_read_array_reference_as_a_descriptor() {
    let mut fixture = fixture("unsize");
    let ExprKind::ArrayToSlice { reference } =
        fixture.checked.exprs[first_unsize(&fixture.checked)].kind
    else {
        unreachable!()
    };
    let index = first_len(&fixture.checked);
    let ExprKind::SliceLen { slice } = &mut fixture.checked.exprs[index].kind else {
        unreachable!()
    };
    *slice = reference;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn noncopy_len_permission_cannot_authorize_discarded_borrowed_value_extraction() {
    let mut fixture = fixture("noncopy");
    let index = first_len(&fixture.checked);
    let ExprKind::SliceLen { slice } = fixture.checked.exprs[index].kind else {
        unreachable!()
    };
    let element = slice_element(&fixture.checked.exprs[slice.0].ty).clone();
    assert!(!fixture.facts.must_copy(&element).unwrap());
    let actual_index=fixture.checked.exprs.iter().position(|expression|matches!(expression.kind,ExprKind::Local(local) if local==fixture.checked.params[1])).unwrap();
    fixture.checked.exprs[index].kind = ExprKind::SliceIndex {
        slice,
        index: ir::ExprId(actual_index),
    };
    fixture.checked.exprs[index].ty = element;
    let error = match fixture
        .checked
        .validate_with_facts(FactsCtx::Validated(&fixture.facts))
    {
        Err(error) => error,
        Ok(()) => match cfg::lower_template(
            Arc::new(fixture.checked.clone()),
            fixture.facts.clone(),
            fixture.facts.budget(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("len-only non-Copy permission allowed borrowed extraction"),
        },
    };
    assert!(
        error.to_ascii_lowercase().contains("copy"),
        "wrong boundary: {error}"
    );
}
#[test]
fn foreign_caller_param_cannot_replace_original_generic_index_result() {
    let mut fixture = fixture("generic");
    let donor = self::fixture("length");
    let ty = slice_element(&donor.checked.locals[donor.checked.params[0].0].ty).clone();
    let own = slice_element(&fixture.checked.locals[fixture.checked.params[0].0].ty);
    assert_ne!(own, &ty);
    let index = first_index(&fixture.checked);
    fixture.checked.exprs[index].ty = ty;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn foreign_nominal_snapshot_cannot_replace_original_noncopy_slice_parameter() {
    let mut fixture = fixture("noncopy");
    let donor = self::fixture("noncopy");
    let ty = donor.checked.locals[donor.checked.params[0].0].ty.clone();
    let local = fixture.checked.params[0];
    fixture.checked.locals[local.0].ty = ty;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn original_slice_template_cannot_replay_under_foreign_world_facts() {
    let fixture = fixture("generic");
    let foreign = self::fixture("generic");
    assert!(
        fixture
            .checked
            .validate_with_facts(FactsCtx::Validated(&foreign.facts))
            .is_err()
    );
    assert!(
        cfg::lower_template(
            Arc::new(fixture.checked.clone()),
            foreign.facts.clone(),
            foreign.facts.budget()
        )
        .is_err()
    );
}
#[test]
fn foreign_actual_generic_slice_element_is_rejected_before_source_authority() {
    let mut program = parsed();
    let mut foreign = parsed();
    let donor = element_source_mut(&mut foreign, "generic").clone();
    *element_source_mut(&mut program, "generic") = donor;
    reject_source(program);
}
#[test]
fn foreign_actual_nominal_slice_element_is_rejected_before_source_authority() {
    let mut program = parsed();
    let mut foreign = parsed();
    let donor = element_source_mut(&mut foreign, "noncopy").clone();
    *element_source_mut(&mut program, "noncopy") = donor;
    reject_source(program);
}
#[test]
fn foreign_builtin_len_occurrence_cannot_grant_descriptor_capability() {
    let mut program = parsed();
    let foreign = parsed();
    let Expr::MethodCall { name, .. } = foreign.functions[index(&foreign, "length")]
        .body
        .tail
        .as_deref()
        .unwrap()
    else {
        panic!("real source len call required")
    };
    let donor = *name;
    let index = index(&program, "length");
    let Expr::MethodCall { name, .. } = program.functions[index].body.tail.as_deref_mut().unwrap()
    else {
        panic!("real source len call required")
    };
    *name = donor;
    reject_source(program);
}
#[test]
fn unused_slice_body_cannot_import_foreign_source_binding() {
    let mut program = parsed();
    let foreign = parsed();
    let index = index(&program, "generic");
    program.functions[index].body = foreign.functions[self::index(&foreign, "generic")]
        .body
        .clone();
    reject_source(program);
}
#[test]
fn reference_generic_argument_reaches_reference_free_slice_boundary() {
    observed_generic_argument_unsupported(
        "fn length<T>(values:&[T])->usize{values.len()}fn main(){let value=1i32;let values=[&value];length::<&i32>(&values);}",
        "slice elements containing references or slices, or nested slice carriers",
    );
}
#[test]
fn reference_containing_generic_argument_reaches_reference_free_slice_boundary() {
    observed_generic_argument_unsupported(
        "struct Wrap<T>{value:T}fn length<T>(values:&[T])->usize{values.len()}fn main(){let value=1i32;let values=[Wrap::<&i32>{value:&value}];length::<Wrap<&i32>>(&values);}",
        "slice elements containing references or slices, or nested slice carriers",
    );
}
#[test]
fn descriptor_copy_does_not_end_original_backing_array_loan() {
    let source = "fn value()->i32{let mut values=[1i32,2i32];let slice:&[i32]=&values;let copied=slice;values[0usize]=3i32;copied[0usize]}fn main(){}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("descriptor copy erased its backing loan");
    let error = error.to_ascii_lowercase();
    assert!(
        error.contains("borrow") || error.contains("loan"),
        "{error}"
    );
}
#[test]
fn shared_descriptor_keeps_backing_array_mutation_conflict_live() {
    let source = "fn value()->i32{let mut values=[1i32,2i32];let slice:&[i32]=&values;values[0usize]=3i32;slice[0usize]}fn main(){}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("unsizing discarded its backing loan");
    let error = error.to_ascii_lowercase();
    assert!(
        error.contains("borrow") || error.contains("loan"),
        "{error}"
    );
}
#[test]
fn mutable_slice_element_writes_require_and_accept_exclusive_carriers() {
    for workers in [1, 8] {
        let result = compiler::compile(
            "fn change(values:&mut [i32]){values[0usize]=3i32;}fn main(){}".into(),
            workers,
            true,
        )
        .unwrap();
        assert!(result.executable.is_none());
        assert!(result.analyses.iter().all(|analysis| analysis.passed()));
    }
}
#[test]
fn new_slice_element_types_do_not_grant_slice_return_capability() {
    observed_unsupported("fn identity<'a>(values:&'a [i32])->&'a [i32]{values}fn main(){}");
}
#[test]
fn noncopy_borrowed_index_field_read_remains_a_separate_closed_capability() {
    observed_unsupported(
        "struct Token{value:i32}fn read(values:&[Token])->i32{values[0usize].value}fn main(){}",
    );
}
