//! Equality authority is obtained from real resolved source and finalized PlanCtx facts.
//! No synthetic fact, proof, declaration ID, or implementation assignment is constructed.
use frontend::{GenericPath, PlanCtx, Program, TraitPath, Type};
use ir::{
    AssociatedTypeProjection, Callee, CheckedFunction, ExprKind, FactsCtx, Predicate,
    SignatureSubject, Ty,
};
use justrust_engine::compiler::traits::{FactsPhase, ValidatedFacts};
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

const SOURCE: &str = r#"
struct S;
struct Token {value:i32}
trait Source<A,B>:Sized {type Out; fn get(self)->Self::Out;}
trait Other<A,B>:Sized {type Out; fn get(self)->Self::Out;}
impl Source<i32,bool> for S {type Out=i32; fn get(self)->Self::Out {41i32}}
impl Source<bool,i32> for S {type Out=bool; fn get(self)->Self::Out {true}}
impl Source<u8,bool> for S {type Out=Token; fn get(self)->Self::Out {Token{value:43i32}}}
impl Other<i32,bool> for S {type Out=i32; fn get(self)->Self::Out {47i32}}
fn integer<T:Source<i32,bool,Out=i32>>(x:T)-><T as Source<i32,bool>>::Out {
    let value:<T as Source<i32,bool>>::Out=<T as Source<i32,bool>>::get(x);value
}
fn peer<U:Source<i32,bool,Out=i32>>(x:U)->i32 {<U as Source<i32,bool>>::get(x)}
fn token<T:Source<u8,bool,Out=Token>>(x:T)->Token {<T as Source<u8,bool>>::get(x)}
fn both<T:Source<i32,bool,Out=i32>+Source<bool,i32,Out=bool>>(x:T,y:T)->(i32,bool) {
    (<T as Source<i32,bool>>::get(x),<T as Source<bool,i32>>::get(y))
}
fn alternate<T:Other<i32,bool,Out=i32>>(x:T)->i32 {<T as Other<i32,bool>>::get(x)}
fn erased<T:Source<i32,bool,Out=i32>>() -> i32 {53i32}
fn unused<T:Source<i32,bool,Out=i32>>() {}
fn wrong_erased<T:Source<i32,bool,Out=bool>>()->bool {true}
fn wrong_unused<T:Source<i32,bool,Out=bool>>() {}
fn body_only<T:Source<i32,bool,Out=i32>>(x:T)->i32 {<T as Source<i32,bool>>::get(x)}
fn forward<T:Source<i32,bool,Out=i32>>(x:T)->i32 {integer::<T>(x)}
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
    let program = frontend::parse(SOURCE).expect("resolved equality source");
    frontend::check(&program).expect("all equality-constrained universal bodies check");
    program
}
fn typed(program: &Program, plan: &PlanCtx, name: &str) -> (CheckedFunction, Arc<ValidatedFacts>) {
    let declaration = declaration(program, plan, name);
    let facts = plan.callable_facts(declaration).unwrap();
    let checked =
        frontend::type_callable_with_plan(program, declaration, plan, facts.budget()).unwrap();
    checked
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    let template =
        cfg::lower_template(Arc::new(checked.clone()), facts.clone(), facts.budget()).unwrap();
    assert!(
        ownership::verify_template(template)
            .unwrap()
            .analysis()
            .passed()
    );
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
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let (checked, facts) = typed(&program, &plan, name);
    Fixture {
        program,
        plan,
        checked,
        facts,
    }
}
fn equalities(facts: &ValidatedFacts) -> Vec<(AssociatedTypeProjection, Ty)> {
    facts
        .predicates()
        .predicates()
        .iter()
        .filter_map(|predicate| match predicate {
            Predicate::ProjectionEq { projection, ty } => Some((projection.clone(), ty.clone())),
            Predicate::Implements(_) => None,
        })
        .collect()
}
fn required_calls(checked: &CheckedFunction) -> Vec<ir::TraitCall> {
    checked
        .exprs
        .iter()
        .filter_map(|expression| match &expression.kind {
            ExprKind::Call {
                target: Callee::TraitItem(call),
                ..
            } => Some(call.clone()),
            _ => None,
        })
        .collect()
}
fn bound_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut GenericPath {
    let index = index(program, name);
    let TraitPath::Source(path) = &mut program.functions[index].bounds[0].traits[0] else {
        panic!("source trait application required")
    };
    path
}
fn reject_source(program: Program) {
    let error = match PlanCtx::from_shared_program(Arc::new(program), &ir::PlanBudget::default()) {
        Ok(_) => panic!("changed equality source gained original-use authority"),
        Err(error) => error,
    };
    let error = error.to_ascii_lowercase();
    assert!(
        !error.contains("unsupported"),
        "metadata must precede capability: {error}"
    );
    assert!(
        error.contains("contract")
            || error.contains("foreign")
            || error.contains("source")
            || error.contains("owner"),
        "{error}"
    );
}
fn reject_retained_source(fixture: &Fixture, name: &str) {
    // Primitive RHSs and ordered arguments can describe a different legitimate
    // program. Their authority is tested against the pre-mutation snapshot,
    // rather than granting the changed AST a fresh planning declaration.
    let declaration = declaration(&fixture.program, &fixture.plan, name);
    let error = frontend::type_callable_with_plan(
        &fixture.program,
        declaration,
        &fixture.plan,
        fixture.facts.budget(),
    )
    .expect_err("changed equality reused the immutable planning declaration");
    let error = error.to_ascii_lowercase();
    assert!(!error.contains("unsupported"), "{error}");
    assert!(
        error.contains("original")
            || error.contains("retained")
            || error.contains("immutable")
            || error.contains("contract")
            || error.contains("source"),
        "{error}"
    );
}
fn invalid(source: &str) {
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("invalid equality program admitted");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "{error}"
    );
}

#[test]
fn rigid_equality_normalizes_signature_local_and_required_result_without_erasing_self() {
    let fixture = fixture("integer");
    assert_eq!(fixture.facts.phase(), FactsPhase::Universal);
    let rows = equalities(&fixture.facts);
    assert_eq!(rows.len(), 1);
    let (projection, expected) = &rows[0];
    assert!(matches!(projection.trait_ref.self_ty, Ty::Param(_)));
    assert_eq!(expected, &Ty::Int(ir::IntTy::I32));
    assert_eq!(&fixture.checked.result, expected);
    assert_eq!(
        fixture.checked.locals[fixture.checked.params[0].0].ty,
        projection.trait_ref.self_ty
    );
    let proof = fixture
        .facts
        .world()
        .normalize_projection_equality(projection, &fixture.facts)
        .unwrap();
    assert_eq!(proof.ty(), expected);
    proof.validate_for(projection, &fixture.facts).unwrap();
    assert!(
        fixture
            .facts
            .world()
            .normalize_associated_type(projection, &fixture.facts)
            .is_err(),
        "a declared equality must not become implementation-assignment evidence"
    );
    assert_eq!(required_calls(&fixture.checked).len(), 1);
    let schema = fixture
        .facts
        .world()
        .normalized_callable_schema(declaration(&fixture.program, &fixture.plan, "integer"))
        .unwrap();
    assert_eq!(schema.signature.result, *expected);
    assert_eq!(schema.signature.params[0], projection.trait_ref.self_ty);
}

#[test]
fn same_self_and_item_keep_ordered_trait_arguments_in_equality_memo_keys() {
    let fixture = fixture("both");
    let rows = equalities(&fixture.facts);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0.trait_ref.self_ty, rows[1].0.trait_ref.self_ty);
    assert_eq!(rows[0].0.item, rows[1].0.item);
    assert_ne!(rows[0].0.trait_ref.args, rows[1].0.trait_ref.args);
    for _ in 0..2 {
        for (projection, expected) in rows.iter().rev() {
            let proof = fixture
                .facts
                .world()
                .normalize_projection_equality(projection, &fixture.facts)
                .unwrap();
            assert_eq!(proof.ty(), expected);
            proof.validate_for(projection, &fixture.facts).unwrap();
            let other = rows
                .iter()
                .find(|(candidate, _)| candidate != projection)
                .unwrap();
            assert!(proof.validate_for(&other.0, &fixture.facts).is_err());
        }
    }
    assert_eq!(required_calls(&fixture.checked).len(), 2);
}

#[test]
fn equality_proof_rejects_real_other_item_world_environment_and_phase() {
    let fixture = fixture("integer");
    let (projection, _) = equalities(&fixture.facts).remove(0);
    let proof = fixture
        .facts
        .world()
        .normalize_projection_equality(&projection, &fixture.facts)
        .unwrap();
    let (_, peer) = typed(&fixture.program, &fixture.plan, "peer");
    assert!(proof.validate_for(&projection, &peer).is_err());
    let foreign = self::fixture("integer");
    assert!(proof.validate_for(&projection, &foreign.facts).is_err());
    assert!(
        foreign
            .facts
            .world()
            .normalize_projection_equality(&projection, &foreign.facts)
            .is_err()
    );
    let (_, alternate) = typed(&fixture.program, &fixture.plan, "alternate");
    let (other, _) = equalities(&alternate).remove(0);
    let mut wrong_item = projection.clone();
    wrong_item.item = other.item;
    assert!(proof.validate_for(&wrong_item, &fixture.facts).is_err());
    assert!(
        fixture
            .facts
            .world()
            .normalize_projection_equality(&wrong_item, &fixture.facts)
            .is_err()
    );
    let actual = fixture
        .facts
        .world()
        .impl_headers()
        .iter()
        .find(|header| {
            header.trait_ref.declaration == projection.trait_ref.declaration
                && header.trait_ref.args == projection.trait_ref.args
        })
        .unwrap();
    let closed = fixture
        .plan
        .instance_facts(&ir::CallTarget {
            declaration: declaration(&fixture.program, &fixture.plan, "integer"),
            type_args: vec![actual.trait_ref.self_ty.clone()].into(),
        })
        .unwrap();
    assert_eq!(closed.phase(), FactsPhase::Closed);
    assert!(proof.validate_for(&projection, &closed).is_err());
    let required = required_calls(&fixture.checked).remove(0);
    let schema = fixture.facts.world().schema_facts(required.item).unwrap();
    assert_eq!(schema.phase(), FactsPhase::Schema);
    assert!(proof.validate_for(&projection, &schema).is_err());
}

#[test]
fn required_carrier_view_preserves_real_caller_param_tail_and_rejects_replay() {
    let fixture = fixture("integer");
    let call = Callee::TraitItem(required_calls(&fixture.checked).remove(0));
    let view = fixture
        .facts
        .carrier_contract(SignatureSubject::Call(&call))
        .unwrap();
    view.validate_for(&fixture.facts).unwrap();
    let parameter = match &equalities(&fixture.facts)[0].0.trait_ref.self_ty {
        Ty::Param(parameter) => *parameter,
        _ => panic!("caller must stay rigid"),
    };
    assert!(
        view.contract().inputs.iter().any(|input| {
            input.param == 0
                && matches!(input.location.channel,
            ir::CarrierChannel::ParamTail { parameter: actual, .. } if actual == parameter)
        }),
        "reference-capable Self lost its actual caller carrier"
    );
    assert!(
        view.contract().results.is_empty(),
        "closed scalar Out invented a carrier"
    );
    let (_, peer) = typed(&fixture.program, &fixture.plan, "peer");
    assert!(view.validate_for(&peer).is_err());
    let foreign = self::fixture("integer");
    assert!(view.validate_for(&foreign.facts).is_err());
    assert!(
        fixture
            .checked
            .validate_with_facts(FactsCtx::Validated(&foreign.facts))
            .is_err()
    );
}

#[test]
fn noncopy_normal_form_retains_ordinary_move_rules() {
    let fixture = fixture("token");
    assert!(matches!(fixture.checked.result, Ty::Struct(_)));
    assert!(!fixture.facts.must_copy(&fixture.checked.result).unwrap());
    let (projection, expected) = equalities(&fixture.facts).remove(0);
    assert_eq!(
        fixture
            .facts
            .world()
            .normalize_projection_equality(&projection, &fixture.facts)
            .unwrap()
            .ty(),
        &expected
    );
    invalid(
        r#"
struct Token {value:i32}
trait Source {type Out;fn get(self)->Self::Out;}
fn duplicate<T:Source<Out=Token>>(x:T)->(Token,Token) {let value=x.get();(value,value)}
fn main() {}
"#,
    );
}

#[test]
fn raw_equalities_survive_erased_signature_unused_and_body_only_requirements() {
    let fixture = fixture("erased");
    let source_projection = &equalities(&fixture.facts)[0].0;
    let matching = fixture
        .facts
        .world()
        .impl_headers()
        .iter()
        .find(|header| {
            header.trait_ref.declaration == source_projection.trait_ref.declaration
                && header.trait_ref.args == source_projection.trait_ref.args
        })
        .unwrap()
        .trait_ref
        .self_ty
        .clone();
    for name in ["erased", "unused", "body_only", "forward"] {
        let (_, facts) = typed(&fixture.program, &fixture.plan, name);
        assert_eq!(
            equalities(&facts).len(),
            1,
            "raw equality erased from {name}"
        );
        let closed = fixture
            .plan
            .instance_facts(&ir::CallTarget {
                declaration: declaration(&fixture.program, &fixture.plan, name),
                type_args: vec![matching.clone()].into(),
            })
            .unwrap();
        assert_eq!(closed.phase(), FactsPhase::Closed);
    }
    for name in ["wrong_erased", "wrong_unused"] {
        let (_, facts) = typed(&fixture.program, &fixture.plan, name);
        assert_eq!(equalities(&facts).len(), 1);
        let error = fixture
            .plan
            .instance_facts(&ir::CallTarget {
                declaration: declaration(&fixture.program, &fixture.plan, name),
                type_args: vec![matching.clone()].into(),
            })
            .expect_err("membership alone discharged the original concrete equality");
        assert!(
            !error
                .to_string()
                .to_ascii_lowercase()
                .contains("unsupported"),
            "{error}"
        );
    }
    for body in ["53i32", "{let _unused=53i32;53i32}"] {
        invalid(&format!(
            r#"
struct S;
trait Source {{type Out;}}
impl Source for S {{type Out=bool;}}
fn erased<T:Source<Out=i32>>()->i32 {{{body}}}
fn main() {{let _value=erased::<S>();}}
"#
        ));
    }
    invalid(
        r#"
struct S;
trait Source {type Out;}
impl Source for S {type Out=bool;}
fn unused<T:Source<Out=i32>>() {}
fn main() {unused::<S>();}
"#,
    );
}

#[test]
fn unused_universal_bad_body_is_not_hidden_by_a_suitable_implementation() {
    invalid(
        r#"
struct S;
trait Source {type Out;fn get(self)->Self::Out;}
impl Source for S {type Out=i32;fn get(self)->Self::Out {1i32}}
fn never_called<T:Source<Out=i32>>(x:T)->bool {x.get()}
fn main() {}
"#,
    );
}

#[test]
fn exact_source_clone_preserves_every_equality_binding_occurrence() {
    let program = parsed();
    PlanCtx::from_shared_program(Arc::new(program.clone()), &ir::PlanBudget::default()).unwrap();
    let repeated = frontend::parse(
        r#"
trait Source {type Out;}
fn unused<T:Source<Out=i32,Out=i32>+Source<Out=i32>>() {}
fn main() {}
"#,
    )
    .unwrap();
    frontend::check(&repeated).unwrap();
    PlanCtx::from_shared_program(Arc::new(repeated.clone()), &ir::PlanBudget::default()).unwrap();
    let mut repeated = repeated;
    assert_eq!(
        bound_mut(&mut repeated, "unused")
            .segments
            .last()
            .unwrap()
            .associated_types
            .len(),
        2
    );
}

#[test]
fn equality_evidence_and_required_carriers_reject_poisoned_shared_budget() {
    for poison_bytes in [false, true] {
        let fixture = fixture("integer");
        let world = fixture.facts.world();
        let (projection, _) = equalities(&fixture.facts).remove(0);
        let proof = world
            .normalize_projection_equality(&projection, &fixture.facts)
            .unwrap();
        let call = required_calls(&fixture.checked).remove(0);
        let view = fixture
            .facts
            .carrier_contract(SignatureSubject::Call(&Callee::TraitItem(call)))
            .unwrap();
        proof.validate_for(&projection, &fixture.facts).unwrap();
        view.validate_for(&fixture.facts).unwrap();
        let budget = fixture.facts.budget();
        if poison_bytes {
            assert!(budget.retain_bytes(usize::MAX).is_err());
        } else {
            assert!(budget.check_depth(usize::MAX).is_err());
        }
        let failure = budget.first_resource_failure().unwrap();
        assert_eq!(
            proof
                .validate_for(&projection, &fixture.facts)
                .unwrap_err()
                .kind(),
            justrust_engine::compiler::traits::FactsErrorKind::Resource
        );
        assert_eq!(
            view.validate_for(&fixture.facts).unwrap_err().kind(),
            justrust_engine::compiler::traits::FactsErrorKind::Resource
        );
        assert_eq!(
            world
                .normalize_projection_equality(&projection, &fixture.facts)
                .unwrap_err()
                .kind(),
            justrust_engine::compiler::traits::FactsErrorKind::Resource
        );
        assert_eq!(budget.first_resource_failure().unwrap(), failure);
    }
}

#[test]
fn same_owner_binding_transplant_and_rhs_change_do_not_reuse_source_certificates() {
    let mut fixture = fixture("integer");
    let donor = bound_mut(&mut fixture.program, "peer")
        .segments
        .last()
        .unwrap()
        .associated_types[0]
        .clone();
    bound_mut(&mut fixture.program, "integer")
        .segments
        .last_mut()
        .unwrap()
        .associated_types[0] = donor;
    reject_retained_source(&fixture, "integer");
    let mut fixture = self::fixture("integer");
    bound_mut(&mut fixture.program, "integer")
        .segments
        .last_mut()
        .unwrap()
        .associated_types[0]
        .value = Type::Bool;
    reject_retained_source(&fixture, "integer");
}

#[test]
fn foreign_binding_name_and_rhs_handles_are_rejected_independently() {
    for change_name in [false, true] {
        let mut program = parsed();
        let mut foreign = parsed();
        let donor = bound_mut(&mut foreign, "token")
            .segments
            .last()
            .unwrap()
            .associated_types[0]
            .clone();
        let binding = &mut bound_mut(&mut program, "token")
            .segments
            .last_mut()
            .unwrap()
            .associated_types[0];
        if change_name {
            binding.name = donor.name;
        } else {
            binding.value = donor.value;
        }
        reject_source(program);
    }
}

#[test]
fn ordered_trait_actual_mutation_cannot_keep_original_equality_authority() {
    let mut fixture = fixture("integer");
    bound_mut(&mut fixture.program, "integer")
        .segments
        .last_mut()
        .unwrap()
        .args
        .swap(0, 1);
    reject_retained_source(&fixture, "integer");
}

#[test]
fn late_foreign_binding_is_checked_before_earlier_parameter_rhs_boundary() {
    let source = r#"
trait Source {type Out;type Extra;}
fn unused<T:Source<Out=U,Extra=i32>,U>() {}
fn main() {}
"#;
    let mut program = frontend::parse(source).unwrap();
    let mut donor = frontend::parse(source).unwrap();
    let foreign_name = bound_mut(&mut donor, "unused")
        .segments
        .last()
        .unwrap()
        .associated_types[1]
        .name;
    bound_mut(&mut program, "unused")
        .segments
        .last_mut()
        .unwrap()
        .associated_types[1]
        .name = foreign_name;
    reject_source(program);
}
