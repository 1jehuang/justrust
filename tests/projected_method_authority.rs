//! Required projected methods use actual source owners, complete applications and facts.
//! Every malformed control starts from a checked PlanCtx baseline. No facts, declaration
//! identities, source certificates or implementation assignments are invented here.
use frontend::{PlanCtx, Program, SourceProjection, Type};
use ir::{Callee, CheckedFunction, ExprKind, FactsCtx, Signature, SignatureSubject, TraitCall, Ty};
use justrust_engine::compiler::traits::{FactsPhase, ValidatedFacts};
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

const SOURCE: &str = r#"
struct S;
struct Token {value:i32}
struct Wrap<T> {value:T}
trait Project<A,B>:Sized {
    type Out;
    fn get(self)->Self::Out;
    fn relay(self,value:Self::Out)->Self::Out;
}
trait Other<A,B>:Sized {
    type Out;
    fn relay(self,value:Self::Out)->Self::Out;
}
impl Project<i32,bool> for S {
    type Out=i32;
    fn get(self)->Self::Out {41i32}
    fn relay(self,value:Self::Out)->Self::Out {value}
}
impl Project<bool,i32> for S {
    type Out=bool;
    fn get(self)->Self::Out {true}
    fn relay(self,value:Self::Out)->Self::Out {value}
}
impl Project<u8,bool> for S {
    type Out=Token;
    fn get(self)->Self::Out {Token{value:43i32}}
    fn relay(self,value:Self::Out)->Self::Out {value}
}
impl Project<u16,bool> for S {
    type Out=(i32,bool);
    fn get(self)->Self::Out {(47i32,true)}
    fn relay(self,value:Self::Out)->Self::Out {value}
}
impl Project<u32,bool> for S {
    type Out=i128;
    fn get(self)->Self::Out {170141183460469231731687303715884105727i128}
    fn relay(self,value:Self::Out)->Self::Out {value}
}
impl<T> Project<i32,bool> for Wrap<T> {
    type Out=T;
    fn get(self)->Self::Out {let moved:Self::Out=self.value;moved}
    fn relay(self,value:Self::Out)->Self::Out {let moved:Self::Out=value;moved}
}
impl Other<i32,bool> for S {
    type Out=i32;
    fn relay(self,value:Self::Out)->Self::Out {value}
}
fn integer()->i32 {<S as Project<i32,bool>>::get(S)}
fn boolean()->bool {<S as Project<bool,i32>>::get(S)}
fn relay_integer()->i32 {<S as Project<i32,bool>>::relay(S,41i32)}
fn relay_boolean()->bool {<S as Project<bool,i32>>::relay(S,true)}
fn token()->Token {<S as Project<u8,bool>>::get(S)}
fn tuple()->(i32,bool) {<S as Project<u16,bool>>::get(S)}
fn wide()->i128 {<S as Project<u32,bool>>::get(S)}
fn wrapped()->i32 {<Wrap<i32> as Project<i32,bool>>::get(Wrap::<i32>{value:53i32})}
fn wrapped_token()->Token {<Wrap<Token> as Project<i32,bool>>::get(Wrap::<Token>{value:Token{value:59i32}})}
fn method()->i32 {Wrap::<i32>{value:61i32}.get()}
fn alternate()->i32 {<S as Other<i32,bool>>::relay(S,67i32)}
fn main() {}
"#;

fn name_is(program: &Program, path: &frontend::AstPath, name: &str) -> bool {
    path.len() == 1 && program.names.spelling(path.segments()[0]).unwrap() == name
}

fn index(program: &Program, name: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| name_is(program, &function.name, name))
        .unwrap()
}

fn trait_index(program: &Program, name: &str) -> usize {
    program
        .traits
        .iter()
        .position(|declaration| name_is(program, &declaration.name, name))
        .unwrap()
}

fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("real resolved projected required-method source");
    frontend::check(&program).expect("all projected signatures and unused impl bodies check");
    program
}

fn typed_decl(
    program: &Program,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    declaration: ir::DeclId,
) -> (CheckedFunction, Arc<ValidatedFacts>) {
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
    assert!(
        ownership::verify_template(template)
            .expect("same-facts universal ownership baseline")
            .analysis()
            .passed()
    );
    (checked, facts)
}

fn typed(
    program: &Program,
    plan: &PlanCtx,
    budget: &ir::PlanBudget,
    name: &str,
) -> (CheckedFunction, Arc<ValidatedFacts>) {
    typed_decl(
        program,
        plan,
        budget,
        plan.function_decl(index(program, name)).unwrap(),
    )
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

fn direct(function: &CheckedFunction) -> (&ir::CallTarget, &Signature) {
    function
        .exprs
        .iter()
        .find_map(|expression| match &expression.kind {
            ExprKind::Call {
                target: Callee::Direct(target),
                signature,
                ..
            } => Some((target, signature)),
            _ => None,
        })
        .expect("real source required-method call selected its actual implementation")
}

fn call_mut(function: &mut CheckedFunction) -> (&mut Callee, &mut Signature) {
    function
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Call {
                target, signature, ..
            } => Some((target, signature)),
            _ => None,
        })
        .unwrap()
}

/// Recover the actual required item from its selected published implementation row.
/// This fixture's trait arguments are closed constants. Its receiver comes from
/// the checked source signature and its impl actuals precede any method actuals.
fn required_call(function: &CheckedFunction, facts: &ValidatedFacts) -> TraitCall {
    let (target, signature) = direct(function);
    let (header, item) = facts
        .world()
        .impl_headers()
        .iter()
        .find_map(|header| {
            header
                .items
                .iter()
                .find(|(_, implementation)| *implementation == target.declaration)
                .map(|(item, _)| (header, *item))
        })
        .expect("selected real impl retains its original required member identity");
    let impl_arity = header.schema_env.total_arity();
    assert!(target.type_args.len() >= impl_arity);
    let mut trait_ref = header.trait_ref.clone();
    trait_ref.self_ty = signature.params[0].clone();
    let call = TraitCall {
        trait_ref,
        item,
        method_args: target.type_args[impl_arity..].to_vec().into(),
    };
    let required = facts
        .service()
        .instantiate_trait_signature(&call, facts)
        .expect("actual required application normalizes before any mutation");
    assert_eq!(&required, signature);
    let resolved = facts
        .service()
        .resolve_trait_call(&call, facts)
        .expect("actual required application selects its original implementation");
    assert_eq!(resolved.callee(), &Callee::Direct(target.clone()));
    assert_eq!(resolved.signature(), signature);
    call
}

fn reject_ir(function: &CheckedFunction, facts: &Arc<ValidatedFacts>) {
    let error = function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("changed projected call gained original signature authority");
    assert!(!error.is_empty());
    assert!(
        cfg::lower_template(Arc::new(function.clone()), facts.clone(), facts.budget()).is_err(),
        "changed projected call bypassed original-facts CFG ingress"
    );
}

fn reject_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error.to_ascii_lowercase(),
        Ok(_) => panic!("foreign projected-method source gained authority"),
    };
    assert!(
        error.contains("contract") || error.contains("foreign"),
        "{error}"
    );
    assert!(!error.contains("unsupported"), "{error}");
    assert_eq!(budget.usage_snapshot().applications, 0);
    assert_eq!(budget.usage_snapshot().obligations, 0);
}

fn semantic_rejection(source: &str) {
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("invalid unused projected-method body was admitted");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "wrong validation boundary: {error}"
    );
}

#[test]
fn required_projected_results_accept_exact_closed_full_trait_applications() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    for (name, expected) in [
        ("integer", Ty::Int(ir::IntTy::I32)),
        ("boolean", Ty::Bool),
        ("relay_integer", Ty::Int(ir::IntTy::I32)),
        ("relay_boolean", Ty::Bool),
        ("wrapped", Ty::Int(ir::IntTy::I32)),
        ("method", Ty::Int(ir::IntTy::I32)),
        ("alternate", Ty::Int(ir::IntTy::I32)),
        ("wide", Ty::Int(ir::IntTy::I128)),
        (
            "tuple",
            Ty::Tuple(vec![Ty::Int(ir::IntTy::I32), Ty::Bool].into()),
        ),
    ] {
        let (checked, _) = typed(&program, &plan, &budget, name);
        assert_eq!(checked.result, expected, "{name}");
        assert_eq!(direct(&checked).1.result, expected, "{name}");
    }
}

#[test]
fn projected_noncopy_result_moves_as_its_actual_record_not_a_scalar_or_copy_value() {
    let fixture = fixture("token");
    let (wrapped, wrapped_facts) = typed(
        &fixture.program,
        &fixture.plan,
        fixture.facts.budget(),
        "wrapped_token",
    );
    assert!(matches!(fixture.checked.result, Ty::Struct(_)));
    assert_eq!(fixture.checked.result, wrapped.result);
    assert!(!fixture.facts.must_copy(&fixture.checked.result).unwrap());
    assert!(!wrapped_facts.must_copy(&wrapped.result).unwrap());
    assert_eq!(direct(&fixture.checked).1.result, fixture.checked.result);
    assert_eq!(direct(&wrapped).1.result, wrapped.result);
}

#[test]
fn generic_impl_projection_preserves_the_real_universal_parameter_and_carrier() {
    let fixture = fixture("wrapped");
    let header = fixture
        .facts
        .world()
        .impl_headers()
        .iter()
        .find(|header| header.schema_env.total_arity() == 1)
        .unwrap();
    let declaration = header
        .items
        .iter()
        .map(|(_, declaration)| *declaration)
        .find(|declaration| {
            name_is(
                &fixture.program,
                &fixture
                    .plan
                    .callable_body(&fixture.program, *declaration)
                    .unwrap()
                    .name,
                "get",
            )
        })
        .unwrap();
    let (checked, facts) = typed_decl(
        &fixture.program,
        &fixture.plan,
        fixture.facts.budget(),
        declaration,
    );
    let parameters: Vec<_> = fixture
        .plan
        .callable_env(declaration)
        .unwrap()
        .ordered_params()
        .collect();
    assert_eq!(parameters.len(), 1);
    assert_eq!(facts.phase(), FactsPhase::Universal);
    assert_eq!(checked.result, Ty::Param(parameters[0]));
    assert!(!facts.must_copy(&checked.result).unwrap());
    let projection = Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
        trait_ref: header.trait_ref.clone(),
        item: header.associated_types[0].associated,
    }));
    assert_eq!(
        facts.world().normalize_ty(&projection, &facts).unwrap(),
        checked.result
    );
    assert!(
        fixture
            .facts
            .world()
            .normalize_ty(&projection, &fixture.facts)
            .is_err()
    );
    let schema = facts
        .world()
        .normalized_callable_schema(declaration)
        .unwrap();
    assert_eq!(schema.signature.result, checked.result);
    let view = facts.carrier_contract(SignatureSubject::Body).unwrap();
    view.validate_for(&facts).unwrap();
    assert!(view.contract().results.iter().any(|result| {
        matches!(
            result.location.channel,
            ir::CarrierChannel::ParamTail { parameter, .. } if parameter == parameters[0]
        )
    }));
    assert!(!view.original_contract().results.is_empty());
}

#[test]
fn required_signature_and_carrier_view_keep_exact_item_and_ordered_trait_arguments() {
    let fixture = fixture("relay_integer");
    let (boolean, _) = typed(
        &fixture.program,
        &fixture.plan,
        fixture.facts.budget(),
        "relay_boolean",
    );
    let integer_call = required_call(&fixture.checked, &fixture.facts);
    let boolean_call = required_call(&boolean, &fixture.facts);
    assert_eq!(integer_call.item, boolean_call.item);
    assert_eq!(
        integer_call.trait_ref.self_ty,
        boolean_call.trait_ref.self_ty
    );
    assert_eq!(
        integer_call.trait_ref.args.as_ref(),
        &[Ty::Int(ir::IntTy::I32), Ty::Bool]
    );
    assert_eq!(
        boolean_call.trait_ref.args.as_ref(),
        &[Ty::Bool, Ty::Int(ir::IntTy::I32)]
    );
    assert_ne!(
        direct(&fixture.checked).0.declaration,
        direct(&boolean).0.declaration
    );
    for (call, function) in [(&integer_call, &fixture.checked), (&boolean_call, &boolean)] {
        let required = fixture
            .facts
            .service()
            .instantiate_trait_signature(call, &fixture.facts)
            .unwrap();
        assert_eq!(&required, direct(function).1);
        let resolved = fixture
            .facts
            .service()
            .resolve_trait_call(call, &fixture.facts)
            .unwrap();
        assert_eq!(resolved.signature(), &required);
        assert_eq!(
            resolved.callee(),
            &Callee::Direct(direct(function).0.clone())
        );
        let callee = Callee::TraitItem(call.clone());
        let view = fixture
            .facts
            .carrier_contract(SignatureSubject::Call(&callee))
            .unwrap();
        view.validate_for(&fixture.facts).unwrap();
        assert_eq!(view.declaration(), call.item);
        assert_ne!(view.declaration(), direct(function).0.declaration);
        // Carrier substitution is rooted in the selected owning impl, not in
        // synthetic trait-Self/A/B parameters. Exact trait identity is private.
        assert!(view.arguments().is_empty());
        assert_eq!(view.original_env().total_arity(), 0);
        assert_eq!(view.source_region_env().declaration, call.item);
        assert!(view.contract().results.is_empty());
        assert!(view.original_contract().results.is_empty());
    }
}

#[test]
fn generic_closed_required_view_retains_real_impl_parameter_before_expansion() {
    let fixture = fixture("wrapped");
    for name in ["wrapped", "wrapped_token"] {
        let (checked, facts) = typed(
            &fixture.program,
            &fixture.plan,
            fixture.facts.budget(),
            name,
        );
        let call = required_call(&checked, &facts);
        let callee = Callee::TraitItem(call.clone());
        let view = facts
            .carrier_contract(SignatureSubject::Call(&callee))
            .unwrap();
        view.validate_for(&facts).unwrap();
        assert_eq!(view.declaration(), call.item);
        assert_eq!(view.arguments(), direct(&checked).0.type_args.as_ref());
        assert_eq!(view.arguments(), std::slice::from_ref(&checked.result));
        let parameters: Vec<_> = view.original_env().ordered_params().collect();
        assert_eq!(parameters.len(), 1);
        assert!(view.original_contract().results.iter().any(|result| {
            matches!(
                result.location.channel,
                ir::CarrierChannel::ParamTail { parameter, .. } if parameter == parameters[0]
            )
        }));
        assert!(view.contract().results.is_empty());
        assert!(view.contract().inputs.is_empty());
        assert_eq!(view.source_region_env().declaration, call.item);
    }
}

#[test]
fn cross_application_required_signature_cannot_reuse_the_other_result_or_value_argument() {
    let fixture = fixture("relay_integer");
    let (boolean, _) = typed(
        &fixture.program,
        &fixture.plan,
        fixture.facts.budget(),
        "relay_boolean",
    );
    let call = required_call(&fixture.checked, &fixture.facts);
    let callee = Callee::TraitItem(call);
    let original = fixture
        .facts
        .service()
        .instantiate_trait_signature(
            match &callee {
                Callee::TraitItem(call) => call,
                _ => unreachable!(),
            },
            &fixture.facts,
        )
        .unwrap();
    fixture
        .facts
        .validate_signature(SignatureSubject::Call(&callee), &original)
        .unwrap();
    for result in [false, true] {
        let mut signature = original.clone();
        if result {
            signature.result = direct(&boolean).1.result.clone();
        } else {
            signature.params[1] = direct(&boolean).1.params[1].clone();
        }
        assert!(
            fixture
                .facts
                .validate_signature(SignatureSubject::Call(&callee), &signature)
                .is_err()
        );
        let mut changed = fixture.checked.clone();
        let (_, actual) = call_mut(&mut changed);
        *actual = signature;
        reject_ir(&changed, &fixture.facts);
    }
}

#[test]
fn changed_full_trait_application_cannot_keep_the_original_method_signature() {
    let fixture = fixture("relay_integer");
    let mut call = required_call(&fixture.checked, &fixture.facts);
    let original = direct(&fixture.checked).1.clone();
    let mut arguments = call.trait_ref.args.to_vec();
    arguments.swap(0, 1);
    call.trait_ref.args = arguments.into();
    let changed_signature = fixture
        .facts
        .service()
        .instantiate_trait_signature(&call, &fixture.facts)
        .unwrap();
    assert_eq!(changed_signature.result, Ty::Bool);
    assert_ne!(changed_signature, original);
    assert!(
        fixture
            .facts
            .validate_signature(SignatureSubject::Call(&Callee::TraitItem(call)), &original)
            .is_err()
    );
    // Retain the real Direct call and all original source evidence in IR.
    let mut changed = fixture.checked.clone();
    *call_mut(&mut changed).1 = changed_signature;
    reject_ir(&changed, &fixture.facts);
}

#[test]
fn nongeneric_projected_method_cannot_accept_extra_type_arguments() {
    let fixture = fixture("relay_integer");
    let (boolean, _) = typed(
        &fixture.program,
        &fixture.plan,
        fixture.facts.budget(),
        "relay_boolean",
    );
    let mut call = required_call(&fixture.checked, &fixture.facts);
    assert!(call.method_args.is_empty());
    call.method_args = vec![direct(&boolean).1.result.clone()].into();
    assert!(
        fixture
            .facts
            .service()
            .instantiate_trait_signature(&call, &fixture.facts)
            .is_err()
    );
    let mut changed = fixture.checked.clone();
    let Callee::Direct(target) = call_mut(&mut changed).0 else {
        panic!("original direct call required")
    };
    target.type_args = call.method_args;
    reject_ir(&changed, &fixture.facts);
}

#[test]
fn actual_foreign_required_member_or_owner_cannot_replace_only_half_of_a_call() {
    let fixture = fixture("relay_integer");
    let (alternate, _) = typed(
        &fixture.program,
        &fixture.plan,
        fixture.facts.budget(),
        "alternate",
    );
    let original = required_call(&fixture.checked, &fixture.facts);
    let donor = required_call(&alternate, &fixture.facts);
    assert_ne!(original.item, donor.item);
    assert_ne!(original.trait_ref.declaration, donor.trait_ref.declaration);
    for member in [false, true] {
        let mut call = original.clone();
        if member {
            call.item = donor.item;
        } else {
            call.trait_ref.declaration = donor.trait_ref.declaration;
        }
        assert!(
            fixture
                .facts
                .service()
                .resolve_trait_call(&call, &fixture.facts)
                .is_err()
        );
        let callee = Callee::TraitItem(call);
        assert!(
            fixture
                .facts
                .carrier_contract(SignatureSubject::Call(&callee))
                .is_err()
        );
    }
}

#[test]
fn per_application_carrier_evidence_cannot_replay_under_other_caller_or_world_facts() {
    let fixture = fixture("relay_integer");
    let (boolean, boolean_facts) = typed(
        &fixture.program,
        &fixture.plan,
        fixture.facts.budget(),
        "relay_boolean",
    );
    let call = Callee::TraitItem(required_call(&fixture.checked, &fixture.facts));
    let view = fixture
        .facts
        .carrier_contract(SignatureSubject::Call(&call))
        .unwrap();
    view.validate_for(&fixture.facts).unwrap();
    assert!(view.validate_for(&boolean_facts).is_err());
    let foreign = self::fixture("relay_integer");
    assert!(view.validate_for(&foreign.facts).is_err());
    assert!(
        foreign
            .facts
            .carrier_contract(SignatureSubject::Call(&call))
            .is_err()
    );
    assert!(
        fixture
            .checked
            .validate_with_facts(FactsCtx::Validated(&foreign.facts))
            .is_err()
    );
    // The other application is valid in its own context, not permission to reuse this view.
    let other = Callee::TraitItem(required_call(&boolean, &boolean_facts));
    boolean_facts
        .carrier_contract(SignatureSubject::Call(&other))
        .unwrap()
        .validate_for(&boolean_facts)
        .unwrap();
}

#[test]
fn foreign_source_required_member_cannot_keep_original_projected_signature() {
    let mut fixture = fixture("integer");
    let foreign = parsed();
    let original = trait_index(&fixture.program, "Project");
    let donor = trait_index(&foreign, "Project");
    fixture.program.traits[original].items[0].signature.name =
        foreign.traits[donor].items[0].signature.name;
    reject_source(fixture.program);
}

#[test]
fn foreign_source_required_owner_cannot_keep_original_members() {
    let mut fixture = fixture("integer");
    let foreign = parsed();
    let original = trait_index(&fixture.program, "Project");
    let donor = trait_index(&foreign, "Project");
    fixture.program.traits[original].name = foreign.traits[donor].name.clone();
    reject_source(fixture.program);
}

#[test]
fn foreign_source_impl_owner_cannot_keep_original_projected_body() {
    let mut fixture = fixture("integer");
    let foreign = parsed();
    fixture.program.impls[0].trait_path = foreign.impls[0].trait_path.clone();
    reject_source(fixture.program);
}

#[test]
fn foreign_source_self_projection_item_cannot_keep_original_required_owner() {
    let mut fixture = fixture("integer");
    let foreign = parsed();
    let original = trait_index(&fixture.program, "Project");
    let donor = trait_index(&foreign, "Project");
    let Type::Projection(projection) = &foreign.traits[donor].items[0].signature.result else {
        panic!("retained source Self::Out projection required")
    };
    let SourceProjection::SelfItem { item: donor } = projection.as_ref() else {
        panic!("actual Self::Out occurrence required")
    };
    let donor = *donor;
    let Type::Projection(projection) =
        &mut fixture.program.traits[original].items[0].signature.result
    else {
        panic!("retained source Self::Out projection required")
    };
    let SourceProjection::SelfItem { item } = projection.as_mut() else {
        panic!("actual Self::Out occurrence required")
    };
    *item = donor;
    reject_source(fixture.program);
}

#[test]
fn unused_generic_projected_impl_body_cannot_import_foreign_source() {
    let mut fixture = fixture("integer");
    let foreign = parsed();
    let original = fixture
        .program
        .impls
        .iter()
        .position(|implementation| !implementation.type_params.is_empty())
        .unwrap();
    let donor = foreign
        .impls
        .iter()
        .find(|implementation| !implementation.type_params.is_empty())
        .unwrap();
    fixture.program.impls[original].items[0].function.body = donor.items[0].function.body.clone();
    reject_source(fixture.program);
}

#[test]
fn unused_generic_impl_body_is_checked_universally_not_at_a_favorable_closed_call() {
    semantic_rejection(
        r#"
        struct Wrap<T>{value:T}
        trait Get:Sized{type Out;fn get(self)->Self::Out;}
        impl<T> Get for Wrap<T>{type Out=T;fn get(self)->Self::Out{7i32}}
        fn favorable()->i32{<Wrap<i32> as Get>::get(Wrap::<i32>{value:1i32})}
        fn main(){}
    "#,
    );
    semantic_rejection(
        r#"
        struct Wrap<T>{value:T}
        trait Get:Sized{type Out;fn get(self)->Self::Out;}
        impl<T> Get for Wrap<T>{type Out=T;fn get(self)->Self::Out{let first=self.value;let second=self.value;second}}
        fn main(){}
    "#,
    );
}

#[test]
fn another_valid_application_cannot_authorize_the_provided_projected_signature() {
    semantic_rejection(
        r#"
        struct S;
        trait Project<A,B>:Sized{type Out;fn relay(self,value:Self::Out)->Self::Out;}
        impl Project<i32,bool> for S{type Out=i32;fn relay(self,value:bool)->bool{value}}
        impl Project<bool,i32> for S{type Out=bool;fn relay(self,value:bool)->bool{value}}
        fn main(){}
    "#,
    );
}

#[test]
fn owning_impl_projection_does_not_authorize_a_different_item_on_the_same_trait() {
    semantic_rejection(
        r#"
        struct S;
        trait Project:Sized{type Out;type Other;fn get(self)->Self::Out;}
        impl Project for S{type Out=i32;type Other=bool;fn get(self)->Self::Other{true}}
        fn main(){}
    "#,
    );
}

#[test]
fn rigid_callers_and_projected_defaults_match_stock() {
    #[path = "support/iterator_oracle.rs"]
    mod oracle;
    for source in [
        r#"struct S;trait Get:Sized{type Out;fn get(self)->Self::Out;}impl Get for S{type Out=i32;fn get(self)->Self::Out{7i32}}fn forward<T:Get>(value:T)-><T as Get>::Out{<T as Get>::get(value)}fn main(){let value=forward::<S>(S);}"#,
        r#"trait Get:Sized{type Out;fn relay(self,value:Self::Out)->Self::Out{value}}fn main(){}"#,
        r#"struct S;trait Get:Sized{type Out;fn get(self)->Self::Out;}impl Get for S{type Out=i32;fn get(self)->Self::Out{7i32}}fn forward<T:Get>(value:T)-><T as Get>::Out{<T as Get>::get(value)}fn main(){println!("{}",forward::<S>(S));}"#,
        r#"trait Get:Sized{type Out;fn relay(self,value:Self::Out)->Self::Out{value}}struct S;impl Get for S{type Out=i32;}fn main(){println!("{}",S.relay(9i32));}"#,
    ] {
        oracle::stock_native(source);
    }
}

#[test]
fn projected_static_reference_and_lifetime_forms_remain_explicit_boundaries() {
    for source in [
        r#"struct S;trait Get:Sized{type Out;fn get(self)->Self::Out;}impl Get for S{type Out=&'static i32;fn get(self)->Self::Out{&7i32}}fn main(){}"#,
        r#"trait Get:Sized{type Out;fn relay<'a>(self,value:&'a Self::Out)->&'a Self::Out;}fn main(){}"#,
    ] {
        let error = frontend::parse(source)
            .and_then(|program| frontend::check(&program))
            .expect_err(&format!(
                "separate projected-method boundary was accidentally admitted: {source}"
            ));
        assert!(
            error.to_ascii_lowercase().contains("unsupported"),
            "{error}"
        );
    }
}
