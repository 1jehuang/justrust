//! Real generic tuple declarations and application-qualified nominal authority.
//! No source identifier, ParamId, nominal definition or canonical fact is fabricated.
use frontend::{Expr, FieldName, PlanCtx, Program, StructShape, Type};
use ir::{CheckedFunction, ExprKind, FactsCtx, Ty};
use justrust_engine::compiler::semantic_name::{LookupClass, Namespace};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

const SOURCE: &str = r#"
struct Pair<T>(T,bool);
struct Other<U>(U,bool);
struct Record<R>{field:R}
fn ints(value:i32)->Pair<i32>{Pair::<i32>(value,true)}
fn bools(value:bool)->Pair<bool>{Pair::<bool>(value,false)}
fn other(value:i32)->Other<i32>{Other::<i32>(value,true)}
fn wrap<T>(value:T)->Pair<T>{Pair::<T>(value,true)}
fn donor<Pair>(value:Pair)->Pair{value}
fn selected(value:Pair<i32>)->i32{let Pair(first,..)=value;first}
fn called()->Pair<i32>{wrap::<i32>(4i32)}
fn main(){}
"#;
fn structure(program: &Program, name: &str) -> usize {
    program
        .structs
        .iter()
        .position(|declaration| {
            declaration.name.len() == 1
                && program
                    .names
                    .spelling(declaration.name.segments()[0])
                    .unwrap()
                    == name
        })
        .unwrap()
}
fn function(program: &Program, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|declaration| {
            declaration.name.len() == 1
                && program
                    .names
                    .spelling(declaration.name.segments()[0])
                    .unwrap()
                    == name
        })
        .unwrap()
}
fn parsed() -> Program {
    let program =
        frontend::parse(SOURCE).expect("actual resolved generic tuple declarations and uses");
    frontend::check(&program).expect("all actual generic bodies check universally");
    let pair = &program.structs[structure(&program, "Pair")];
    assert_eq!(pair.shape, StructShape::Tuple);
    assert_eq!(pair.type_params.len(), 1);
    assert!(matches!(pair.fields[0].ty, Type::Param(_)));
    program
}
fn reject_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("malformed generic tuple source gained planning authority"),
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
fn application_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut frontend::GenericPath {
    let index = function(program, name);
    let Some(Expr::Apply { path, .. }) = program.functions[index].body.tail.as_deref_mut() else {
        panic!("actual constructor turbofish source required")
    };
    path
}
#[test]
fn exact_generic_tuple_clone_retains_source_owners_and_universal_plan() {
    let program = parsed();
    let cloned = program.clone();
    assert!(Arc::ptr_eq(&program.names, &cloned.names));
    assert!(Arc::ptr_eq(&program.modules, &cloned.modules));
    PlanCtx::from_shared_program(Arc::new(cloned), &ir::PlanBudget::default()).unwrap();
}
#[test]
fn generic_tuple_shape_cannot_import_record_field_identity() {
    let mut program = parsed();
    let pair = structure(&program, "Pair");
    let record = structure(&program, "Record");
    program.structs[pair].fields[0].name = program.structs[record].fields[0].name;
    reject_source(program);
}
#[test]
fn generic_tuple_shape_cannot_become_record_by_field_count() {
    let mut program = parsed();
    let pair = structure(&program, "Pair");
    program.structs[pair].shape = StructShape::Record;
    reject_source(program);
}
#[test]
fn generic_tuple_shape_requires_contiguous_physical_indices() {
    let mut program = parsed();
    let pair = structure(&program, "Pair");
    program.structs[pair].fields[1].name = FieldName::Tuple(2);
    reject_source(program);
}
#[test]
fn generic_tuple_shape_rejects_duplicate_physical_indices() {
    let mut program = parsed();
    let pair = structure(&program, "Pair");
    program.structs[pair].fields[1].name = FieldName::Tuple(0);
    reject_source(program);
}
#[test]
fn generic_tuple_fields_cannot_be_reordered_under_original_schema() {
    let mut program = parsed();
    let pair = structure(&program, "Pair");
    program.structs[pair].fields.swap(0, 1);
    reject_source(program);
}
#[test]
fn generic_tuple_cannot_import_foreign_parameter_declaration() {
    let mut program = parsed();
    let foreign = parsed();
    let pair = structure(&program, "Pair");
    program.structs[pair].type_params[0] =
        foreign.structs[structure(&foreign, "Pair")].type_params[0];
    reject_source(program);
}
#[test]
fn generic_tuple_cannot_import_foreign_parameter_field_type() {
    let mut program = parsed();
    let foreign = parsed();
    let pair = structure(&program, "Pair");
    program.structs[pair].fields[0].ty = foreign.structs[structure(&foreign, "Pair")].fields[0]
        .ty
        .clone();
    reject_source(program);
}
#[test]
fn generic_tuple_application_cannot_import_foreign_type_argument_occurrence() {
    let mut program = parsed();
    let foreign = parsed();
    let donor = foreign.functions[function(&foreign, "wrap")].params[0]
        .1
        .clone();
    assert!(matches!(donor, Type::Param(_)));
    application_mut(&mut program, "wrap")
        .segments
        .last_mut()
        .unwrap()
        .args[0] = donor;
    reject_source(program);
}
#[test]
fn generic_tuple_target_certificate_binds_occurrence_not_only_semantic_key() {
    let mut program = parsed();
    let donor = program.functions[function(&program, "donor")].type_params[0];
    let original = application_mut(&mut program, "ints")
        .segments
        .last()
        .unwrap()
        .name;
    assert_eq!(program.names.spelling(original).unwrap(), "Pair");
    assert_eq!(program.names.spelling(donor).unwrap(), "Pair");
    assert_eq!(
        program
            .names
            .key(original, LookupClass::Item(Namespace::Type))
            .unwrap(),
        program
            .names
            .key(donor, LookupClass::Item(Namespace::Type))
            .unwrap()
    );
    assert_ne!(
        program.names.span(original).unwrap(),
        program.names.span(donor).unwrap()
    );
    application_mut(&mut program, "ints")
        .segments
        .last_mut()
        .unwrap()
        .name = donor;
    reject_source(program);
}
#[test]
fn foreign_names_owner_cannot_repair_unused_generic_tuple() {
    let mut program = parsed();
    program.names = parsed().names;
    reject_source(program);
}
#[test]
fn foreign_modules_owner_cannot_repair_unused_generic_tuple() {
    let mut program = parsed();
    program.modules = parsed().modules;
    reject_source(program);
}

fn callable(program: &Program, plan: &PlanCtx, name: &str) -> ir::DeclId {
    (0..plan.callable_count())
        .map(|index| plan.callable_decl(index).unwrap())
        .find(|declaration| {
            let source = plan.callable_body(program, *declaration).unwrap();
            source.name.len() == 1
                && program.names.spelling(source.name.segments()[0]).unwrap() == name
        })
        .unwrap()
}
fn typed(
    program: &Program,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    name: &str,
) -> (CheckedFunction, Arc<ValidatedFacts>) {
    let declaration = callable(program, plan, name);
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
    ints: CheckedFunction,
    ints_facts: Arc<ValidatedFacts>,
    bools: CheckedFunction,
    other: CheckedFunction,
    _program: Program,
}
fn fixture() -> Fixture {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (ints, ints_facts) = typed(&program, &plan, &budget, "ints");
    let (bools, _) = typed(&program, &plan, &budget, "bools");
    let (other, _) = typed(&program, &plan, &budget, "other");
    Fixture {
        ints,
        ints_facts,
        bools,
        other,
        _program: program,
    }
}
fn constructor(function: &CheckedFunction) -> &ir::StructTy {
    function
        .exprs
        .iter()
        .find_map(|expression| match &expression.kind {
            ExprKind::Struct { ty, .. } => Some(ty),
            _ => None,
        })
        .expect("real checked tuple constructor")
}
fn constructor_mut(
    function: &mut CheckedFunction,
) -> (&mut ir::StructTy, &mut Vec<(usize, ir::ExprId)>) {
    function
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Struct { ty, fields } => Some((ty, fields)),
            _ => None,
        })
        .expect("real checked tuple constructor")
}
fn reject_constructor_lowering(fixture: &Fixture, expected: &str) {
    fixture
        .ints
        .validate_with_facts(FactsCtx::Validated(&fixture.ints_facts))
        .expect("field-list corruption retains valid nominal IR metadata");
    let error = cfg::lower_with_facts(
        &fixture.ints,
        &fixture.ints_facts,
        fixture.ints_facts.budget(),
    )
    .expect_err("malformed constructor field coverage reached CFG publication");
    assert_eq!(error, expected);
}
fn reject_ir(function: &CheckedFunction, facts: &ValidatedFacts) {
    function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("corrupted tuple application gained structural authority");
    assert!(cfg::lower_with_facts(function, facts, facts.budget()).is_err());
}
#[test]
fn real_concrete_applications_share_declaration_not_application_identity_or_field_type() {
    let fixture = fixture();
    let ints = constructor(&fixture.ints);
    let bools = constructor(&fixture.bools);
    assert_eq!(ints.def().source_decl, bools.def().source_decl);
    assert_ne!(ints.def().id, bools.def().id);
    assert_eq!(ints.def().type_args.as_ref(), &[Ty::Int(ir::IntTy::I32)]);
    assert_eq!(bools.def().type_args.as_ref(), &[Ty::Bool]);
    assert_eq!(ints.def().fields[0].ty, Ty::Int(ir::IntTy::I32));
    assert_eq!(bools.def().fields[0].ty, Ty::Bool);
    assert_eq!(ints.def().fields[1].ty, Ty::Bool);
    assert_eq!(bools.def().fields[1].ty, Ty::Bool);
    fixture
        .ints
        .clone()
        .validate_with_facts(FactsCtx::Validated(&fixture.ints_facts))
        .unwrap();
}
#[test]
fn real_universal_wrapper_retains_actual_param_id_in_nominal_arguments_and_field() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = callable(&program, &plan, "wrap");
    let environment = plan.callable_env(declaration).unwrap();
    let parameters: Vec<_> = environment.ordered_params().collect();
    assert_eq!(parameters.len(), 1);
    let checked = frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    checked
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    let nominal = constructor(&checked);
    assert_eq!(
        nominal.def().type_args.as_ref(),
        &[Ty::Param(parameters[0])]
    );
    assert_eq!(nominal.def().fields[0].ty, Ty::Param(parameters[0]));
    assert_eq!(
        checked.locals[checked.params[0].0].ty,
        Ty::Param(parameters[0])
    );
}
#[test]
fn wrong_concrete_application_cannot_replace_constructor_under_original_signature_facts() {
    let mut fixture = fixture();
    let donor = constructor(&fixture.bools).clone();
    *constructor_mut(&mut fixture.ints).0 = donor;
    reject_ir(&fixture.ints, &fixture.ints_facts);
}
#[test]
fn same_arguments_do_not_authorize_another_actual_nominal_declaration() {
    let mut fixture = fixture();
    let donor = constructor(&fixture.other).clone();
    assert_ne!(
        constructor(&fixture.ints).def().source_decl,
        donor.def().source_decl
    );
    assert_eq!(
        constructor(&fixture.ints).def().type_args,
        donor.def().type_args
    );
    *constructor_mut(&mut fixture.ints).0 = donor;
    reject_ir(&fixture.ints, &fixture.ints_facts);
}
#[test]
fn substituted_tuple_fields_require_their_actual_concrete_operand_types() {
    let mut fixture = fixture();
    let fields = constructor_mut(&mut fixture.ints).1;
    let (first, rest) = fields.split_at_mut(1);
    std::mem::swap(&mut first[0].1, &mut rest[0].1);
    fixture
        .ints
        .validate_with_facts(FactsCtx::Validated(&fixture.ints_facts))
        .expect("operand references retain valid IR metadata");
    let lowered = cfg::lower_with_facts(
        &fixture.ints,
        &fixture.ints_facts,
        fixture.ints_facts.budget(),
    )
    .expect("well-indexed constructor reaches aggregate operand validation");
    let error = lowered
        .validate_with_facts(FactsCtx::Validated(&fixture.ints_facts))
        .expect_err("swapped concrete operands gained aggregate type authority");
    assert_eq!(error, "CFG operand type mismatch");
}
#[test]
fn substituted_tuple_constructor_rejects_duplicate_physical_fields() {
    let mut fixture = fixture();
    constructor_mut(&mut fixture.ints).1[1].0 = 0;
    reject_constructor_lowering(&fixture, "invalid checked struct initializer field");
}
#[test]
fn substituted_tuple_constructor_rejects_missing_physical_field() {
    let mut fixture = fixture();
    let _ = constructor_mut(&mut fixture.ints).1.pop();
    reject_constructor_lowering(&fixture, "missing checked struct initializer field");
}
#[test]
fn substituted_tuple_constructor_rejects_out_of_bounds_physical_field() {
    let mut fixture = fixture();
    constructor_mut(&mut fixture.ints).1[1].0 = 2;
    reject_constructor_lowering(&fixture, "invalid checked struct initializer field");
}
#[test]
fn universal_typecheck_cannot_use_favorable_i32_call_to_copy_arbitrary_t() {
    let source = "struct Wrap<T>(T);fn bad<T>(value:T)->i32{let first=Wrap::<T>(value);let second=Wrap::<T>(value);0}fn main(){bad::<i32>(1i32);}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("favorable concrete i32 use hid universal duplicate move");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "wrong boundary: {error}"
    );
}
#[test]
fn unused_generic_duplicate_move_body_is_not_skipped() {
    let source = "struct Wrap<T>(T);fn bad<T>(value:T)->i32{let first=Wrap::<T>(value);let second=Wrap::<T>(value);0}fn main(){}";
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("unused universally invalid generic tuple body was skipped");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "wrong boundary: {error}"
    );
}
#[test]
fn explicit_constructor_argument_count_remains_semantically_checked() {
    let mut program = parsed();
    application_mut(&mut program, "ints")
        .segments
        .last_mut()
        .unwrap()
        .args
        .push(Type::Bool);
    let error =
        frontend::check(&program).expect_err("wrong generic arity gained constructor authority");
    assert!(
        error.contains("wrong generic arity"),
        "wrong boundary: {error}"
    );
}
