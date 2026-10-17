//! Closed projections retain real source declarations, assignments and caller facts.
//! No invented proof, fact, callable or associated-item identity is used here.
use frontend::{PlanCtx, Program, SourceProjection, Type};
use ir::{AssociatedTypeProjection, CheckedFunction, FactsCtx, Ty};
use justrust_engine::compiler::traits::{FactsErrorKind, FactsPhase, ValidatedFacts};
use justrust_engine::compiler::{backend, cfg, frontend, ir, ownership};
use std::sync::Arc;

const SOURCE: &str = r#"
struct S;
struct Token {value:i32}
struct Wrap<T> {value:T}
trait Has<A> {type Out;}
trait Other<A> {type Out;}
trait Duo<A,B> {type Out;}
trait Probe {fn probe(self);}
impl Probe for S {fn probe(self){}}
impl Has<i32> for S {type Out=i32;}
impl Has<bool> for S {type Out=bool;}
impl Has<u8> for S {type Out=Token;}
impl Has<u16> for S {type Out=(i32,bool);}
impl Has<u32> for S {type Out=i128;}
impl Has<u64> for S {type Out=<S as Has<i32>>::Out;}
impl<T> Has<bool> for Wrap<T> {type Out=T;}
impl Other<i32> for S {type Out=i32;}
impl Duo<i32,bool> for S {type Out=i32;}
impl Duo<bool,i32> for S {type Out=bool;}
type Alias=<S as Has<i32>>::Out;
fn integer(value:<S as Has<i32>>::Out)-><S as Has<i32>>::Out {value}
fn boolean(value:<S as Has<bool>>::Out)-><S as Has<bool>>::Out {value}
fn token(value:<S as Has<u8>>::Out)-><S as Has<u8>>::Out {value}
fn tuple(value:<S as Has<u16>>::Out)-><S as Has<u16>>::Out {value}
fn wide(value:<S as Has<u32>>::Out)-><S as Has<u32>>::Out {value}
fn wrapped(value:<Wrap<i32> as Has<bool>>::Out)-><Wrap<i32> as Has<bool>>::Out {value}
fn alternate(value:<S as Other<i32>>::Out)-><S as Other<i32>>::Out {value}
fn pair(value:<S as Duo<i32,bool>>::Out)-><S as Duo<i32,bool>>::Out {value}
fn reversed(value:<S as Duo<bool,i32>>::Out)-><S as Duo<bool,i32>>::Out {value}
fn alias(value:Alias)->Alias {let local:<S as Has<i32>>::Out=value;local}
fn nested(value:<S as Has<u64>>::Out)-><S as Has<u64>>::Out {value}
fn generic<T:Has<i32>>(value:T)->T {value}
fn foreign_generic<U:Has<i32>>(value:U)->U {value}
fn dead()->i32 {return 1i32;let dormant=2i32;3i32}
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
fn declaration(program: &Program, plan: &PlanCtx, name: &str) -> ir::DeclId {
    plan.function_decl(index(program, name)).unwrap()
}
fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("resolved associated-only source");
    frontend::check(&program).expect("all original bodies and unused assignments check");
    program
}
fn typed(
    program: &Program,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    name: &str,
) -> (CheckedFunction, Arc<ValidatedFacts>) {
    let declaration = declaration(program, plan, name);
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
    ownership::verify_template(template).expect("real same-facts ownership baseline");
    (checked, facts)
}
struct Fixture {
    program: Program,
    plan: PlanCtx,
    checked: CheckedFunction,
    facts: Arc<ValidatedFacts>,
}
fn fixture(name: &str) -> Fixture {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (checked, facts) = typed(&program, &plan, &budget, name);
    Fixture {
        program,
        plan,
        checked,
        facts,
    }
}
/// Both identities come from a published real impl, not ordinal fabrication.
fn projection(fixture: &Fixture, args: &[Ty], rhs: &Ty) -> AssociatedTypeProjection {
    let header = fixture
        .facts
        .world()
        .impl_headers()
        .iter()
        .find(|header| {
            header.trait_ref.args.as_ref() == args
                && header
                    .associated_types
                    .iter()
                    .any(|value| &value.rhs == rhs)
                && header.schema_env.total_arity() == 0
        })
        .expect("actual closed implementation header");
    let assignment = header
        .associated_types
        .iter()
        .find(|value| &value.rhs == rhs)
        .unwrap();
    assert_eq!(assignment.implementation, header.declaration);
    AssociatedTypeProjection {
        trait_ref: header.trait_ref.clone(),
        item: assignment.associated,
    }
}
fn integer_projection(fixture: &Fixture) -> AssociatedTypeProjection {
    projection(
        fixture,
        &[Ty::Int(ir::IntTy::I32)],
        &Ty::Int(ir::IntTy::I32),
    )
}
fn reject_ir(function: &CheckedFunction, facts: &Arc<ValidatedFacts>) {
    let error = function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("residual associated projection gained executable IR authority");
    assert!(!error.is_empty());
    assert!(
        cfg::lower_template(Arc::new(function.clone()), facts.clone(), facts.budget()).is_err(),
        "residual projection bypassed original universal CFG ingress"
    );
}
fn dead_literal(function: &mut CheckedFunction) -> &mut ir::TypedExpr {
    function.exprs.iter_mut().find(|expression| {
        matches!(&expression.kind, ir::ExprKind::Int(value) if *value == ir::IntValue::i32(3))
    }).expect("actual unreachable source tail literal retained in typed arena")
}
fn result_projection_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut SourceProjection {
    let index = index(program, name);
    let Type::Projection(projection) = &mut program.functions[index].result else {
        panic!("actual retained qualified result projection required")
    };
    projection
}
fn reject_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("foreign projection occurrence gained source authority"),
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
fn normalized_signatures_accept_general_noncopy_aggregate_wide_alias_and_local_types() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    for name in [
        "integer",
        "boolean",
        "token",
        "tuple",
        "wide",
        "wrapped",
        "alternate",
        "pair",
        "reversed",
        "alias",
        "nested",
    ] {
        let (checked, facts) = typed(&program, &plan, &budget, name);
        let schema = facts
            .world()
            .normalized_callable_schema(declaration(&program, &plan, name))
            .unwrap();
        assert_eq!(schema.signature.result, checked.result, "{name}");
        assert_eq!(
            schema.signature.params[0], checked.locals[checked.params[0].0].ty,
            "{name}"
        );
        assert_eq!(
            checked.locals[checked.params[0].0].ty, checked.result,
            "{name}"
        );
        match name {
            "integer" | "wrapped" | "alias" | "alternate" | "pair" | "nested" => {
                assert_eq!(checked.result, Ty::Int(ir::IntTy::I32))
            }
            "boolean" | "reversed" => assert_eq!(checked.result, Ty::Bool),
            "wide" => assert_eq!(checked.result, Ty::Int(ir::IntTy::I128)),
            "tuple" => assert_eq!(
                checked.result,
                Ty::Tuple(vec![Ty::Int(ir::IntTy::I32), Ty::Bool].into())
            ),
            "token" => {
                assert!(matches!(checked.result, Ty::Struct(_)));
                assert!(
                    !facts.must_copy(&checked.result).unwrap(),
                    "normalization invented Copy"
                );
            }
            _ => unreachable!(),
        }
    }
}
#[test]
fn closed_projection_normalizes_under_original_universal_facts() {
    let fixture = fixture("integer");
    let projection = integer_projection(&fixture);
    assert_eq!(fixture.facts.phase(), FactsPhase::Universal);
    let result = fixture
        .facts
        .world()
        .normalize_associated_type(&projection, &fixture.facts)
        .unwrap();
    assert_eq!(result.ty(), &Ty::Int(ir::IntTy::I32));
    assert_eq!(fixture.facts.phase(), FactsPhase::Universal);
}
#[test]
fn same_self_distinct_trait_arguments_do_not_share_assignment_results() {
    let fixture = fixture("integer");
    let integer = integer_projection(&fixture);
    let boolean = projection(&fixture, &[Ty::Bool], &Ty::Bool);
    assert_eq!(integer.trait_ref.self_ty, boolean.trait_ref.self_ty);
    assert_eq!(integer.item, boolean.item);
    for (projection, expected) in [(integer, Ty::Int(ir::IntTy::I32)), (boolean, Ty::Bool)] {
        let result = fixture
            .facts
            .world()
            .normalize_associated_type(&projection, &fixture.facts)
            .unwrap();
        assert_eq!(result.ty(), &expected);
    }
}
#[test]
fn ordered_trait_arguments_select_distinct_actual_assignments() {
    let fixture = fixture("pair");
    let mut projection = projection(
        &fixture,
        &[Ty::Int(ir::IntTy::I32), Ty::Bool],
        &Ty::Int(ir::IntTy::I32),
    );
    assert_eq!(
        fixture
            .facts
            .world()
            .normalize_associated_type(&projection, &fixture.facts)
            .unwrap()
            .ty(),
        &Ty::Int(ir::IntTy::I32)
    );
    let mut args = projection.trait_ref.args.to_vec();
    args.swap(0, 1);
    projection.trait_ref.args = args.into();
    // The permutation is another valid application, not permission to reuse i32.
    assert_eq!(
        fixture
            .facts
            .world()
            .normalize_associated_type(&projection, &fixture.facts)
            .unwrap()
            .ty(),
        &Ty::Bool
    );
}
#[test]
fn actual_other_trait_item_cannot_keep_original_projection_owner() {
    let fixture = fixture("integer");
    let mut original = integer_projection(&fixture);
    fixture
        .facts
        .world()
        .normalize_associated_type(&original, &fixture.facts)
        .unwrap();
    let foreign = fixture
        .facts
        .world()
        .impl_headers()
        .iter()
        .find(|header| {
            header.trait_ref.declaration != original.trait_ref.declaration
                && header.trait_ref.args.as_ref() == [Ty::Int(ir::IntTy::I32)]
        })
        .unwrap();
    original.item = foreign.associated_types[0].associated;
    assert!(
        fixture
            .facts
            .world()
            .normalize_associated_type(&original, &fixture.facts)
            .is_err()
    );
}
#[test]
fn actual_other_owner_cannot_keep_original_associated_item() {
    let fixture = fixture("integer");
    let mut original = integer_projection(&fixture);
    fixture
        .facts
        .world()
        .normalize_associated_type(&original, &fixture.facts)
        .unwrap();
    let foreign = fixture
        .facts
        .world()
        .impl_headers()
        .iter()
        .find(|header| {
            header.trait_ref.declaration != original.trait_ref.declaration
                && header.trait_ref.args.as_ref() == [Ty::Int(ir::IntTy::I32)]
        })
        .unwrap();
    original.trait_ref.declaration = foreign.trait_ref.declaration;
    assert!(
        fixture
            .facts
            .world()
            .normalize_associated_type(&original, &fixture.facts)
            .is_err()
    );
}
#[test]
fn foreign_world_projection_and_foreign_facts_cannot_replay_normalization() {
    let original = fixture("integer");
    let foreign = fixture("integer");
    let projection = integer_projection(&original);
    let evidence = original
        .facts
        .world()
        .normalize_associated_type(&projection, &original.facts)
        .unwrap();
    evidence.validate_for(&projection, &original.facts).unwrap();
    assert!(evidence.validate_for(&projection, &foreign.facts).is_err());
    assert!(
        foreign
            .facts
            .world()
            .normalize_associated_type(&projection, &foreign.facts)
            .is_err()
    );
    assert!(
        original
            .facts
            .world()
            .normalize_associated_type(&projection, &foreign.facts)
            .is_err()
    );
    assert!(
        {
            let mut injected = original.checked.clone();
            injected.result = Ty::AssociatedType(Arc::new(integer_projection(&foreign)));
            injected
        }
        .validate_with_facts(FactsCtx::Validated(&original.facts))
        .is_err()
    );
}
#[test]
fn malformed_projection_arguments_do_not_hit_a_successful_normalization_cache() {
    for extra in [false, true] {
        let fixture = fixture("integer");
        let mut projection = integer_projection(&fixture);
        fixture
            .facts
            .world()
            .normalize_associated_type(&projection, &fixture.facts)
            .unwrap();
        projection.trait_ref.args = if extra {
            vec![Ty::Int(ir::IntTy::I32), Ty::Bool].into()
        } else {
            Arc::from([])
        };
        assert!(
            fixture
                .facts
                .world()
                .normalize_associated_type(&projection, &fixture.facts)
                .is_err()
        );
    }
}
#[test]
fn opaque_normalization_evidence_rejects_argument_permutation_replay() {
    let fixture = fixture("pair");
    let mut projection = projection(
        &fixture,
        &[Ty::Int(ir::IntTy::I32), Ty::Bool],
        &Ty::Int(ir::IntTy::I32),
    );
    let evidence = fixture
        .facts
        .world()
        .normalize_associated_type(&projection, &fixture.facts)
        .unwrap();
    evidence.validate_for(&projection, &fixture.facts).unwrap();
    let mut args = projection.trait_ref.args.to_vec();
    args.swap(0, 1);
    projection.trait_ref.args = args.into();
    assert!(evidence.validate_for(&projection, &fixture.facts).is_err());
}
#[test]
fn opaque_normalization_evidence_binds_actual_phase_and_environment() {
    let fixture = fixture("integer");
    let projection = integer_projection(&fixture);
    let declaration = declaration(&fixture.program, &fixture.plan, "integer");
    let item = fixture
        .facts
        .world()
        .impl_headers()
        .iter()
        .find_map(|header| header.items.first().map(|(item, _)| *item))
        .expect("actual nonprojected source trait method for Schema facts");
    let schema = fixture.facts.world().schema_facts(item).unwrap();
    let closed = fixture
        .plan
        .instance_facts(&ir::CallTarget {
            declaration,
            type_args: Arc::from([]),
        })
        .unwrap();
    let (_, other) = typed(
        &fixture.program,
        &fixture.plan,
        fixture.facts.budget(),
        "boolean",
    );
    let contexts = [&schema, &fixture.facts, &closed];
    assert_eq!(
        contexts.map(|facts| facts.phase()),
        [
            FactsPhase::Schema,
            FactsPhase::Universal,
            FactsPhase::Closed
        ]
    );
    for (index, facts) in contexts.iter().enumerate() {
        let evidence = facts
            .world()
            .normalize_associated_type(&projection, facts)
            .unwrap();
        assert_eq!(evidence.ty(), &Ty::Int(ir::IntTy::I32));
        evidence.validate_for(&projection, facts).unwrap();
        for (other_index, other_phase) in contexts.iter().enumerate() {
            if index != other_index {
                assert!(evidence.validate_for(&projection, other_phase).is_err());
            }
        }
        assert!(evidence.validate_for(&projection, &other).is_err());
    }
}
#[test]
fn original_env_rejects_actual_parameter_from_another_callable() {
    let fixture = fixture("generic");
    let (_, other) = typed(
        &fixture.program,
        &fixture.plan,
        fixture.facts.budget(),
        "foreign_generic",
    );
    let mut projection = integer_projection(&fixture);
    fixture
        .facts
        .world()
        .normalize_associated_type(&projection, &fixture.facts)
        .unwrap();
    let parameter = other.env().ordered_params().next().unwrap();
    assert!(
        !fixture
            .facts
            .env()
            .ordered_params()
            .any(|original| original == parameter)
    );
    projection.trait_ref.self_ty = Ty::Param(parameter);
    let error = fixture
        .facts
        .world()
        .normalize_associated_type(&projection, &fixture.facts)
        .unwrap_err();
    assert_eq!(error.kind(), FactsErrorKind::InvalidMetadata);
}
#[test]
fn membership_assumption_does_not_supply_an_associated_assignment_witness() {
    let fixture = fixture("generic");
    let mut projection = integer_projection(&fixture);
    fixture
        .facts
        .world()
        .normalize_associated_type(&projection, &fixture.facts)
        .unwrap();
    projection.trait_ref.self_ty = Ty::Param(fixture.facts.env().ordered_params().next().unwrap());
    let error = fixture
        .facts
        .world()
        .normalize_associated_type(&projection, &fixture.facts)
        .unwrap_err();
    assert_eq!(error.kind(), FactsErrorKind::Unsupported);
}
#[test]
fn cached_normalization_and_evidence_cannot_bypass_exhausted_original_ledger() {
    let fixture = fixture("integer");
    let projection = integer_projection(&fixture);
    let evidence = fixture
        .facts
        .world()
        .normalize_associated_type(&projection, &fixture.facts)
        .unwrap();
    evidence.validate_for(&projection, &fixture.facts).unwrap();
    let budget = fixture.facts.budget();
    let remaining = budget.limits().work - budget.usage_snapshot().work;
    budget.charge_work(remaining).unwrap();
    assert!(budget.charge_work(1).is_err());
    let error = fixture
        .facts
        .world()
        .normalize_associated_type(&projection, &fixture.facts)
        .unwrap_err();
    assert!(error.is_resource());
    assert!(
        evidence
            .validate_for(&projection, &fixture.facts)
            .unwrap_err()
            .is_resource()
    );
}
#[test]
fn recursive_type_normalization_preserves_tuple_and_zero_array_structure() {
    let fixture = fixture("integer");
    let raw = Ty::AssociatedType(Arc::new(integer_projection(&fixture)));
    let nested = Ty::Tuple(
        vec![
            raw.clone(),
            Ty::Array {
                element: Arc::new(raw),
                len: 0,
            },
        ]
        .into(),
    );
    let normal = fixture
        .facts
        .world()
        .normalize_ty(&nested, &fixture.facts)
        .unwrap();
    assert_eq!(
        normal,
        Ty::Tuple(
            vec![
                Ty::Int(ir::IntTy::I32),
                Ty::Array {
                    element: Arc::new(Ty::Int(ir::IntTy::I32)),
                    len: 0
                }
            ]
            .into()
        )
    );
}
#[test]
fn residual_projection_rejected_in_result_local_dead_and_nested_metadata() {
    for location in 0..5 {
        let mut fixture = fixture("dead");
        let raw = Ty::AssociatedType(Arc::new(integer_projection(&fixture)));
        fixture
            .facts
            .world()
            .normalize_associated_type(
                match &raw {
                    Ty::AssociatedType(projection) => projection,
                    _ => unreachable!(),
                },
                &fixture.facts,
            )
            .unwrap();
        match location {
            0 => fixture.checked.result = raw,
            1 => {
                fixture
                    .checked
                    .locals
                    .iter_mut()
                    .find(|local| local.name == "dormant")
                    .unwrap()
                    .ty = raw
            }
            2 => dead_literal(&mut fixture.checked).ty = raw,
            3 => dead_literal(&mut fixture.checked).ty = Ty::Tuple(vec![Ty::Bool, raw].into()),
            4 => {
                dead_literal(&mut fixture.checked).ty = Ty::Array {
                    element: Arc::new(raw),
                    len: 0,
                }
            }
            _ => unreachable!(),
        }
        reject_ir(&fixture.checked, &fixture.facts);
    }
}
#[test]
fn closed_backend_gate_rejects_residual_projection_even_in_zero_length_array() {
    let mut fixture = fixture("dead");
    let target = ir::CallTarget {
        declaration: declaration(&fixture.program, &fixture.plan, "dead"),
        type_args: Arc::from([]),
    };
    let closed = fixture.plan.instance_facts(&target).unwrap();
    assert_eq!(closed.phase(), FactsPhase::Closed);
    fixture
        .checked
        .validate_with_facts(FactsCtx::Validated(&closed))
        .unwrap();
    cfg::lower_with_facts(&fixture.checked, &closed, closed.budget()).unwrap();
    backend::emit_checked_function(&fixture.checked, closed.clone())
        .expect("real call-free closed backend baseline");
    let raw = Ty::AssociatedType(Arc::new(integer_projection(&fixture)));
    dead_literal(&mut fixture.checked).ty = Ty::Array {
        element: Arc::new(raw),
        len: 0,
    };
    assert!(cfg::lower_with_facts(&fixture.checked, &closed, closed.budget()).is_err());
    assert!(backend::emit_checked_function(&fixture.checked, closed).is_err());
}
#[test]
fn foreign_source_associated_item_occurrence_cannot_keep_original_trait_path() {
    let mut original = parsed();
    let mut foreign = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&original, &budget).unwrap();
    typed(&original, &plan, &budget, "integer");
    let SourceProjection::Qualified { item: donor, .. } =
        result_projection_mut(&mut foreign, "integer")
    else {
        panic!("qualified projection")
    };
    let donor = *donor;
    let SourceProjection::Qualified { item, .. } = result_projection_mut(&mut original, "integer")
    else {
        panic!("qualified projection")
    };
    *item = donor;
    reject_source(original);
}
#[test]
fn foreign_source_trait_path_cannot_keep_original_associated_item() {
    let mut original = parsed();
    let mut foreign = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&original, &budget).unwrap();
    typed(&original, &plan, &budget, "integer");
    let SourceProjection::Qualified {
        trait_path: donor, ..
    } = result_projection_mut(&mut foreign, "integer")
    else {
        panic!("qualified projection")
    };
    let donor = donor.clone();
    let SourceProjection::Qualified { trait_path, .. } =
        result_projection_mut(&mut original, "integer")
    else {
        panic!("qualified projection")
    };
    *trait_path = donor;
    reject_source(original);
}
#[test]
fn foreign_impl_assignment_member_occurrence_cannot_replace_original_row() {
    let mut original = parsed();
    let foreign = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&original, &budget).unwrap();
    typed(&original, &plan, &budget, "integer");
    let implementation = original
        .impls
        .iter()
        .position(|implementation| !implementation.associated_types.is_empty())
        .unwrap();
    let donor = foreign
        .impls
        .iter()
        .find(|implementation| !implementation.associated_types.is_empty())
        .unwrap();
    original.impls[implementation].associated_types[0].name = donor.associated_types[0].name;
    reject_source(original);
}
#[test]
fn unused_generic_assignment_rhs_cannot_import_foreign_parameter_occurrence() {
    let mut original = parsed();
    let foreign = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&original, &budget).unwrap();
    typed(&original, &plan, &budget, "wrapped");
    let implementation = original
        .impls
        .iter()
        .position(|implementation| !implementation.type_params.is_empty())
        .unwrap();
    let donor = foreign
        .impls
        .iter()
        .find(|implementation| !implementation.type_params.is_empty())
        .unwrap();
    original.impls[implementation].associated_types[0].value =
        donor.associated_types[0].value.clone();
    reject_source(original);
}
#[test]
fn missing_duplicate_extra_and_unused_invalid_assignments_fail_semantically() {
    for source in [
        "struct S;trait Has<A>{type Out;}impl Has<i32> for S {}fn main(){}",
        "struct S;trait Has<A>{type Out;}impl Has<i32> for S {type Out=i32;type Out=bool;}fn main(){}",
        "struct S;trait Has<A>{type Out;}impl Has<i32> for S {type Out=i32;type Extra=bool;}fn main(){}",
        "struct S;trait Has<A>{type Out;}impl Has<i32> for S {type Out=Missing;}fn main(){}",
        "struct S;trait Has<A>{type Out;}impl Has<i32> for S {type Out=<S as Has<bool>>::Out;}fn main(){}",
        "struct S;trait Has<A>{type Out;}impl Has<i32> for S {type Out=<S as Has<i32>>::Out;}fn main(){}",
        "struct S;trait Has<A>{type Out;}impl Has<i32> for S {type Out=<S as Has<bool>>::Out;}impl Has<bool> for S {type Out=<S as Has<i32>>::Out;}fn main(){}",
    ] {
        let error = frontend::parse(source)
            .and_then(|program| frontend::check(&program))
            .expect_err("invalid unused associated assignment was admitted");
        assert!(
            !error.to_ascii_lowercase().contains("unsupported"),
            "wrong validation boundary: {error}"
        );
    }
}
#[test]
fn rigid_projection_and_projected_trait_method_positive_remain_explicitly_unsupported() {
    for source in [
        "trait Has<A>{type Out;}fn rigid<T:Has<i32>>(value:<T as Has<i32>>::Out)-><T as Has<i32>>::Out{value}fn main(){}",
        "trait Has<A>{type Out;fn project(value:Self::Out)->Self::Out;}fn main(){}",
    ] {
        let error = frontend::parse(source)
            .and_then(|program| frontend::check(&program))
            .expect_err("first slice accidentally admitted symbolic associated projection");
        assert!(
            error.to_ascii_lowercase().contains("unsupported"),
            "{error}"
        );
    }
}
