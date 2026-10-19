//! Public-boundary parameter equality tests. Every identity, obligation and proof
//! starts at real parsed source and a finalized PlanCtx. Private certificate
//! deletion/duplication requires sibling-module tests, not public test factories.
use frontend::{GenericPath, PlanCtx, Program, TraitPath};
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
struct Boxed<T> {value:T}
trait Source<A,B>:Sized {type Out; fn get(self)->Self::Out; fn relay(self,value:Self::Out)->Self::Out;}
trait Other<A,B>:Sized {type Out; fn get(self)->Self::Out;}
impl Source<i32,bool> for S {
    type Out=i32;
    fn get(self)->Self::Out {37i32}
    fn relay(self,value:Self::Out)->Self::Out {41i32}
}
impl Source<bool,i32> for S {
    type Out=bool;
    fn get(self)->Self::Out {true}
    fn relay(self,value:Self::Out)->Self::Out {false}
}
impl Source<u8,bool> for S {
    type Out=Token;
    fn get(self)->Self::Out {Token{value:43i32}}
    fn relay(self,value:Self::Out)->Self::Out {Token{value:47i32}}
}
impl Other<i32,bool> for S {type Out=i32;fn get(self)->Self::Out {53i32}}
fn get<T:Source<i32,bool,Out=U>,U>(x:T)->U {<T as Source<i32,bool>>::get(x)}
fn projected<T:Source<i32,bool,Out=U>,U>(x:T)-><T as Source<i32,bool>>::Out {<T as Source<i32,bool>>::get(x)}
fn peer<T:Source<i32,bool,Out=U>,U>(x:T)->U {<T as Source<i32,bool>>::get(x)}
fn forward<T:Source<i32,bool,Out=U>,U>(x:T)->U {get::<T,U>(x)}
fn nested<T:Source<i32,bool,Out=U>,U>(x:T)->U {forward::<T,U>(x)}
fn relay<T:Source<i32,bool,Out=U>,U>(x:T,value:U)->U {<T as Source<i32,bool>>::relay(x,value)}
fn token<T:Source<u8,bool,Out=U>,U>(x:T)->U {let value:U=<T as Source<u8,bool>>::get(x);value}
fn copy_pair<T:Source<i32,bool,Out=U>,U:Copy>(x:T)->(U,U) {let value:U=<T as Source<i32,bool>>::get(x);(value,value)}
fn wrapped<T:Source<i32,bool,Out=U>,U>(x:T)->Boxed<U> {Boxed::<U>{value:<T as Source<i32,bool>>::get(x)}}
fn array<T:Source<i32,bool,Out=U>,U>(x:T)->[U;1] {[<T as Source<i32,bool>>::get(x)]}
fn tuple<T:Source<i32,bool,Out=U>,U>(x:T)->(U,bool) {(<T as Source<i32,bool>>::get(x),true)}
fn both<T:Source<i32,bool,Out=U>+Source<bool,i32,Out=V>,U,V>(x:T,y:T)->(U,V) {
    (<T as Source<i32,bool>>::get(x),<T as Source<bool,i32>>::get(y))
}
fn alternate<T:Other<i32,bool,Out=U>,U>(x:T)->U {<T as Other<i32,bool>>::get(x)}
fn unused<T:Source<i32,bool,Out=U>,U>() {}
fn erased<T:Source<i32,bool,Out=U>,U>()->i32 {59i32}
fn body_only<T:Source<i32,bool,Out=U>,U>(x:T)->i32 {let value:U=<T as Source<i32,bool>>::get(x);61i32}
fn dead<T:Source<i32,bool,Out=U>,U>(x:T,y:T)->U {return <T as Source<i32,bool>>::get(x);<T as Source<i32,bool>>::get(y)}
fn closed_token()->Token {token::<S,Token>(S)}
fn closed_copy()->(i32,i32) {copy_pair::<S,i32>(S)}
fn main() {let value=get::<S,i32>(S);println!("parameter equality");}
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
fn bound_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut GenericPath {
    let index = index(program, name);
    let TraitPath::Source(path) = &mut program.functions[index].bounds[0].traits[0] else {
        panic!("real source bound required")
    };
    path
}
fn typed(program: &Program, plan: &PlanCtx, name: &str) -> (CheckedFunction, Arc<ValidatedFacts>) {
    let decl = declaration(program, plan, name);
    let facts = plan.callable_facts(decl).unwrap();
    let checked = frontend::type_callable_with_plan(program, decl, plan, facts.budget()).unwrap();
    checked
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
    let template =
        cfg::lower_template(Arc::new(checked.clone()), facts.clone(), facts.budget()).unwrap();
    template
        .function()
        .validate_with_facts(FactsCtx::Validated(&facts))
        .unwrap();
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
    let program = frontend::parse(SOURCE).unwrap();
    frontend::check(&program).expect("all real parameter-equality bodies check universally");
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
        .filter_map(|row| match row {
            Predicate::ProjectionEq { projection, ty } => Some((projection.clone(), ty.clone())),
            Predicate::Implements(_) => None,
        })
        .collect()
}
fn calls(checked: &CheckedFunction) -> Vec<Callee> {
    checked
        .exprs
        .iter()
        .filter_map(|expression| match &expression.kind {
            ExprKind::Call { target, .. } => Some(target.clone()),
            _ => None,
        })
        .collect()
}
fn reject_ir(checked: &CheckedFunction, facts: &Arc<ValidatedFacts>) {
    assert!(
        checked
            .validate_with_facts(FactsCtx::Validated(facts))
            .is_err()
    );
    assert!(cfg::lower_template(Arc::new(checked.clone()), facts.clone(), facts.budget()).is_err());
}
fn rejected(source: &str, unsupported: bool) {
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("invalid source was admitted")
        .to_ascii_lowercase();
    assert_eq!(error.contains("unsupported"), unsupported, "{error}");
}
fn reject_retained_source(fixture: &Fixture, name: &str) {
    let error = frontend::type_callable_with_plan(
        &fixture.program,
        declaration(&fixture.program, &fixture.plan, name),
        &fixture.plan,
        fixture.facts.budget(),
    )
    .expect_err("mutated source reused original finalized plan")
    .to_ascii_lowercase();
    assert!(!error.contains("unsupported"), "{error}");
    assert!(
        error.contains("source")
            || error.contains("original")
            || error.contains("contract")
            || error.contains("immutable")
            || error.contains("retained")
            || error.contains("foreign"),
        "{error}"
    );
}

#[test]
fn producer_preserves_exact_rhs_param_and_both_body_and_required_result_tails() {
    let fixture = fixture("get");
    assert_eq!(fixture.facts.phase(), FactsPhase::Universal);
    let decl = declaration(&fixture.program, &fixture.plan, "get");
    let params: Vec<_> = fixture
        .plan
        .callable_env(decl)
        .unwrap()
        .ordered_params()
        .collect();
    assert_eq!(params.len(), 2);
    let (projection, rhs) = equalities(&fixture.facts).remove(0);
    assert_eq!(projection.trait_ref.self_ty, Ty::Param(params[0]));
    assert_eq!(rhs, Ty::Param(params[1]));
    assert_ne!(params[0], params[1]);
    assert_eq!(fixture.checked.result, rhs);
    assert!(!fixture.facts.must_copy(&rhs).unwrap());
    let proof = fixture
        .facts
        .world()
        .normalize_projection_equality(&projection, &fixture.facts)
        .unwrap();
    assert_eq!(proof.ty(), &rhs);
    proof.validate_for(&projection, &fixture.facts).unwrap();
    let call = calls(&fixture.checked).remove(0);
    assert!(matches!(call, Callee::TraitItem(_)));
    for subject in [SignatureSubject::Body, SignatureSubject::Call(&call)] {
        let view = fixture.facts.carrier_contract(subject).unwrap();
        view.validate_for(&fixture.facts).unwrap();
        assert!(
            view.contract()
                .results
                .iter()
                .any(|result| matches!(result.location.channel,
            ir::CarrierChannel::ParamTail {parameter,..} if parameter == params[1]))
        );
        assert!(
            view.contract()
                .results
                .iter()
                .all(|result| result.sources.is_empty()),
            "T must not become an invented source of the unrelated U family"
        );
        assert!(
            view.contract()
                .inputs
                .iter()
                .any(|input| matches!(input.location.channel,
            ir::CarrierChannel::ParamTail {parameter,..} if parameter == params[0]))
        );
        assert!(
            !view
                .contract()
                .inputs
                .iter()
                .any(|input| matches!(input.location.channel,
            ir::CarrierChannel::ParamTail {parameter,..} if parameter == params[1]))
        );
    }
}

#[test]
fn same_u_input_preserves_conservative_sources_without_proving_receiver_passthrough() {
    let fixture = fixture("relay");
    let call = calls(&fixture.checked).remove(0);
    let (_, rhs) = equalities(&fixture.facts).remove(0);
    let Ty::Param(parameter) = rhs else {
        panic!("real RHS parameter")
    };
    let view = fixture
        .facts
        .carrier_contract(SignatureSubject::Call(&call))
        .unwrap();
    view.validate_for(&fixture.facts).unwrap();
    let result = view
        .contract()
        .results
        .iter()
        .find(|result| {
            matches!(result.location.channel,ir::CarrierChannel::ParamTail {parameter:actual,..}
            if actual == parameter)
        })
        .unwrap();
    assert!(!result.sources.is_empty());
    assert!(result.sources.iter().all(|source| source.param == 1
        && matches!(source.location.channel,ir::CarrierChannel::ParamTail {parameter:actual,..}
            if actual == parameter)));
    // Source's concrete relay returns 41, not its value argument. The source
    // union is conservative carrier topology, not a value-identity proof.
}

#[test]
fn direct_projection_free_forwarding_retains_raw_obligations_and_rhs_tail() {
    let fixture = fixture("forward");
    for name in [
        "projected",
        "forward",
        "nested",
        "relay",
        "wrapped",
        "array",
        "tuple",
        "body_only",
        "unused",
        "erased",
    ] {
        let (checked, facts) = typed(&fixture.program, &fixture.plan, name);
        let rows = equalities(&facts);
        assert_eq!(rows.len(), 1, "{name}");
        let schema = facts
            .world()
            .normalized_callable_schema(declaration(&fixture.program, &fixture.plan, name))
            .unwrap();
        if name == "projected" {
            assert_eq!(checked.result, rows[0].1);
            assert_eq!(schema.signature.result, rows[0].1);
        }
        if name == "forward" || name == "nested" {
            assert_eq!(schema.signature.result, rows[0].1);
            let call = calls(&checked).remove(0);
            assert!(matches!(call, Callee::Direct(_)));
            let view = facts
                .carrier_contract(SignatureSubject::Call(&call))
                .unwrap();
            view.validate_for(&facts).unwrap();
            let Ty::Param(rhs) = rows[0].1 else {
                panic!("real RHS parameter required")
            };
            assert!(
                view.contract()
                    .results
                    .iter()
                    .any(|result| matches!(result.location.channel,
                ir::CarrierChannel::ParamTail {parameter,..} if parameter == rhs))
            );
        }
    }
}

#[test]
fn noncopy_moves_once_and_only_genuine_copy_permits_duplicate() {
    let fixture = fixture("token");
    assert!(!fixture.facts.must_copy(&fixture.checked.result).unwrap());
    let (closed, facts) = typed(&fixture.program, &fixture.plan, "closed_token");
    assert!(matches!(closed.result, Ty::Struct(_)));
    assert!(!facts.must_copy(&closed.result).unwrap());
    typed(&fixture.program, &fixture.plan, "closed_copy");
    rejected(
        r#"
trait Source:Sized {type Out;fn get(self)->Self::Out;}
fn duplicate<T:Source<Out=U>,U>(x:T)->(U,U) {
    let value:U=<T as Source>::get(x);(value,value)
}
fn main() {}
"#,
        false,
    );
}

#[test]
fn proof_and_carrier_replay_bind_world_environment_phase_member_and_ordered_arguments() {
    let fixture = fixture("get");
    let (projection, _) = equalities(&fixture.facts).remove(0);
    let proof = fixture
        .facts
        .world()
        .normalize_projection_equality(&projection, &fixture.facts)
        .unwrap();
    let call = calls(&fixture.checked).remove(0);
    let view = fixture
        .facts
        .carrier_contract(SignatureSubject::Call(&call))
        .unwrap();
    let (_, peer) = typed(&fixture.program, &fixture.plan, "peer");
    assert!(proof.validate_for(&projection, &peer).is_err());
    assert!(view.validate_for(&peer).is_err());
    let foreign = self::fixture("get");
    assert!(proof.validate_for(&projection, &foreign.facts).is_err());
    assert!(view.validate_for(&foreign.facts).is_err());
    reject_ir(&fixture.checked, &foreign.facts);
    let (_, alternate) = typed(&fixture.program, &fixture.plan, "alternate");
    let (other, _) = equalities(&alternate).remove(0);
    let mut changed = projection.clone();
    changed.item = other.item;
    assert!(proof.validate_for(&changed, &fixture.facts).is_err());
    assert!(
        fixture
            .facts
            .world()
            .normalize_projection_equality(&changed, &fixture.facts)
            .is_err()
    );
    let mut changed = projection.clone();
    let mut args = changed.trait_ref.args.to_vec();
    args.swap(0, 1);
    changed.trait_ref.args = args.into();
    assert!(proof.validate_for(&changed, &fixture.facts).is_err());
    let Callee::TraitItem(required) = call else {
        panic!("required call")
    };
    let schema = fixture.facts.world().schema_facts(required.item).unwrap();
    assert_eq!(schema.phase(), FactsPhase::Schema);
    assert!(proof.validate_for(&projection, &schema).is_err());
    assert!(view.validate_for(&schema).is_err());
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
            declaration: declaration(&fixture.program, &fixture.plan, "get"),
            type_args: vec![actual.trait_ref.self_ty.clone(), Ty::Int(ir::IntTy::I32)].into(),
        })
        .unwrap();
    assert_eq!(closed.phase(), FactsPhase::Closed);
    assert!(proof.validate_for(&projection, &closed).is_err());
    assert!(view.validate_for(&closed).is_err());
}

#[test]
fn ordered_rows_normalize_to_distinct_exact_rhs_parameters() {
    let fixture = fixture("both");
    let rows = equalities(&fixture.facts);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0.item, rows[1].0.item);
    assert_eq!(rows[0].0.trait_ref.self_ty, rows[1].0.trait_ref.self_ty);
    assert_ne!(rows[0].0.trait_ref.args, rows[1].0.trait_ref.args);
    assert_ne!(rows[0].1, rows[1].1);
    for _ in 0..2 {
        for (projection, rhs) in rows.iter().rev() {
            let proof = fixture
                .facts
                .world()
                .normalize_projection_equality(projection, &fixture.facts)
                .unwrap();
            assert_eq!(proof.ty(), rhs);
            let other = rows
                .iter()
                .find(|(candidate, _)| candidate != projection)
                .unwrap();
            assert!(proof.validate_for(&other.0, &fixture.facts).is_err());
        }
    }
}

#[test]
fn public_ir_changes_cannot_retain_original_call_or_body_authority() {
    for name in ["get", "forward", "relay"] {
        let fixture = fixture(name);
        for mutation in 0..4 {
            let mut changed = fixture.checked.clone();
            match mutation {
                0 => changed.result = Ty::Bool,
                1 => {
                    let expression = changed
                        .exprs
                        .iter_mut()
                        .find(|expression| matches!(expression.kind, ExprKind::Call { .. }))
                        .unwrap();
                    let ExprKind::Call { signature, .. } = &mut expression.kind else {
                        unreachable!()
                    };
                    signature.result = Ty::Bool;
                }
                2 => {
                    let expression = changed
                        .exprs
                        .iter_mut()
                        .find(|expression| matches!(expression.kind, ExprKind::Call { .. }))
                        .unwrap();
                    expression.ty = Ty::Bool;
                }
                _ => {
                    let expression = changed
                        .exprs
                        .iter_mut()
                        .find(|expression| matches!(expression.kind, ExprKind::Call { .. }))
                        .unwrap();
                    let ExprKind::Call { signature, .. } = &mut expression.kind else {
                        unreachable!()
                    };
                    signature.params[0] = Ty::Bool;
                }
            }
            // call_evidence and body_substitution are deliberately unchanged.
            if mutation == 2 {
                // A node/result disagreement is a CFG consistency error; the
                // retained call signature and its evidence are unchanged.
                assert!(
                    cfg::lower_template(
                        Arc::new(changed),
                        fixture.facts.clone(),
                        fixture.facts.budget()
                    )
                    .is_err()
                );
            } else {
                reject_ir(&changed, &fixture.facts);
            }
        }
    }
}

#[test]
fn actual_calls_and_dead_result_types_cannot_reuse_original_evidence() {
    let fixture = fixture("get");
    let (peer, peer_facts) = typed(&fixture.program, &fixture.plan, "peer");
    let (_, alternate_facts) = typed(&fixture.program, &fixture.plan, "alternate");
    let foreign_rhs = equalities(&peer_facts)[0].1.clone();
    assert!(match (&foreign_rhs, &fixture.checked.result) {
        (Ty::Param(a), Ty::Param(b)) => {
            assert_ne!(a.owner, b.owner);
            a.index == b.index
        }
        _ => false,
    });
    let foreign_call = calls(&peer).remove(0);
    let alternate_item = equalities(&alternate_facts)[0].0.item;
    for mutation in 0..4 {
        let mut changed = fixture.checked.clone();
        let expression = changed
            .exprs
            .iter_mut()
            .find(|expression| matches!(expression.kind, ExprKind::Call { .. }))
            .unwrap();
        let ExprKind::Call {
            target, signature, ..
        } = &mut expression.kind
        else {
            unreachable!()
        };
        match mutation {
            0 => signature.result = foreign_rhs.clone(),
            1 => *target = foreign_call.clone(),
            2 => {
                let Callee::TraitItem(call) = target else {
                    panic!("required call")
                };
                call.item = alternate_item;
            }
            _ => {
                let Callee::TraitItem(call) = target else {
                    panic!("required call")
                };
                let mut args = call.trait_ref.args.to_vec();
                args.swap(0, 1);
                call.trait_ref.args = args.into();
            }
        }
        reject_ir(&changed, &fixture.facts);
    }
    let (mut dead, facts) = typed(&fixture.program, &fixture.plan, "dead");
    assert_eq!(calls(&dead).len(), 2, "retain source's unreachable call");
    let expression = dead
        .exprs
        .iter_mut()
        .rev()
        .find(|expression| matches!(expression.kind, ExprKind::Call { .. }))
        .unwrap();
    expression.ty = foreign_rhs;
    reject_ir(&dead, &facts);
    let (mut local, facts) = typed(&fixture.program, &fixture.plan, "body_only");
    let slot = local
        .locals
        .iter_mut()
        .find(|slot| slot.name == "value")
        .unwrap();
    slot.ty = Ty::Bool;
    // Local initializer agreement is checked during CFG lowering, separately
    // from the unchanged callable signature and its proof.
    assert!(cfg::lower_template(Arc::new(local), facts.clone(), facts.budget()).is_err());
}

#[test]
fn direct_actual_arguments_cannot_exchange_same_named_caller_parameters() {
    let fixture = fixture("forward");
    let mut changed = fixture.checked.clone();
    let expression = changed
        .exprs
        .iter_mut()
        .find(|expression| matches!(expression.kind, ExprKind::Call { .. }))
        .unwrap();
    let ExprKind::Call {
        target: Callee::Direct(target),
        ..
    } = &mut expression.kind
    else {
        panic!("real direct forwarding call")
    };
    let mut actuals = target.type_args.to_vec();
    assert_eq!(actuals.len(), 2);
    actuals.swap(0, 1);
    target.type_args = actuals.into();
    reject_ir(&changed, &fixture.facts);
}

#[test]
fn original_ast_binding_rows_are_not_reconstructed_from_names() {
    for mutation in 0..5 {
        let mut fixture = fixture("get");
        // Capture actual original bound occurrences before any mutation. These
        // carry source identities, unlike equal-looking newly fabricated rows.
        let original = bound_mut(&mut fixture.program, "get").clone();
        let peer = bound_mut(&mut fixture.program, "peer").clone();
        let mut foreign = frontend::parse(SOURCE).unwrap();
        let foreign = bound_mut(&mut foreign, "get").clone();
        let target = bound_mut(&mut fixture.program, "get")
            .segments
            .last_mut()
            .unwrap();
        match mutation {
            0 => {
                target.associated_types[0] =
                    peer.segments.last().unwrap().associated_types[0].clone()
            }
            1 => {
                target.associated_types[0].value = foreign.segments.last().unwrap().associated_types
                    [0]
                .value
                .clone()
            }
            2 => {
                target.associated_types[0].name =
                    foreign.segments.last().unwrap().associated_types[0].name
            }
            3 => target.args.swap(0, 1),
            _ => target.associated_types.clear(),
        }
        assert_eq!(original.segments.last().unwrap().associated_types.len(), 1);
        reject_retained_source(&fixture, "get");
    }
    let fixture = fixture("get");
    let exact_clone = fixture.program.clone();
    frontend::type_callable_with_plan(
        &exact_clone,
        declaration(&exact_clone, &fixture.plan, "get"),
        &fixture.plan,
        fixture.facts.budget(),
    )
    .unwrap();
}

#[test]
fn unused_erased_body_local_and_forwarding_obligations_reject_wrong_actual_equality() {
    let fixture = fixture("get");
    let (projection, _) = equalities(&fixture.facts).remove(0);
    let receiver = fixture
        .facts
        .world()
        .impl_headers()
        .iter()
        .find(|header| {
            header.trait_ref.declaration == projection.trait_ref.declaration
                && header.trait_ref.args == projection.trait_ref.args
        })
        .unwrap()
        .trait_ref
        .self_ty
        .clone();
    for name in [
        "get",
        "forward",
        "nested",
        "unused",
        "erased",
        "body_only",
        "relay",
    ] {
        for (actual, valid) in [(Ty::Int(ir::IntTy::I32), true), (Ty::Bool, false)] {
            let result = fixture.plan.instance_facts(&ir::CallTarget {
                declaration: declaration(&fixture.program, &fixture.plan, name),
                type_args: vec![receiver.clone(), actual].into(),
            });
            if valid {
                assert_eq!(result.unwrap().phase(), FactsPhase::Closed)
            } else {
                let error = result.expect_err("wrong equality survived erased signature");
                assert!(
                    !error
                        .to_string()
                        .to_ascii_lowercase()
                        .contains("unsupported"),
                    "{error}"
                );
            }
        }
    }
    rejected(
        r#"
trait Source:Sized {type Out;fn get(self)->Self::Out;}
fn never_called<T:Source<Out=U>,U>(x:T)->bool {<T as Source>::get(x)}
fn main() {}
"#,
        false,
    );
}

#[test]
fn closed_lhs_nominal_where_syntax_remains_an_explicit_boundary() {
    // The private world control separately exercises closed-LHS/rigid-RHS
    // rejection. This source stops earlier at the existing where-clause grammar.
    let error = frontend::parse(
        r#"
struct S;
trait Source:Sized {type Out;}
impl Source for S {type Out=i32;}
fn closed<U>() where S:Source<Out=U> {}
fn main() {}
"#,
    )
    .expect_err("nominal where-clause grammar was unexpectedly admitted");
    assert!(
        error.contains("where predicates are supported only on visible type parameters"),
        "{error}"
    );
}

#[test]
fn reference_actual_domains_reject_direct_hidden_and_zero_length_metadata() {
    for rhs in ["&'static i32", "Hidden", "[&'static i32;0]"] {
        // The equality is unused and erased from the result, so no result-row
        // disappearance can waive its recursive domain obligation.
        let source = format!(
            r#"
struct S;
struct Hidden {{value:&'static i32}}
trait Source:Sized {{type Out;}}
impl Source for S {{type Out={rhs};}}
fn unused<T:Source<Out=U>,U>() {{}}
fn main() {{unused::<S,{rhs}>();}}
"#
        );
        rejected(&source, true);
    }
}

#[test]
fn exact_reference_actuals_fail_instance_publication_without_banning_unrelated_refs() {
    let source = r#"
struct S;
struct Hidden<T> {value:T}
trait Source:Sized {type Out;}
impl Source for S {type Out=i32;}
fn unused<T:Source<Out=U>,U>() {}
fn erased<T:Source<Out=U>,U>()->i32 {1i32}
fn scalar(x:S,value:i32,h:Hidden<i32>,unrelated:&i32) {}
fn main() {}
"#;
    let program = frontend::parse(source).unwrap();
    frontend::check(&program).expect("unrelated reference signatures remain valid");
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (scalar, facts) = typed(&program, &plan, "scalar");
    let receiver = scalar.locals[scalar.params[0].0].ty.clone();
    let Ty::Struct(hidden) = &scalar.locals[scalar.params[2].0].ty else {
        panic!("canonical Hidden")
    };
    let reference = scalar.locals[scalar.params[3].0].ty.clone();
    // Reference-bearing nominal actuals are admitted (G3 aggregates).
    plan.intern_nominal(
        hidden.def().source_decl,
        std::slice::from_ref(&reference),
        FactsCtx::Validated(&facts),
        &budget,
    )
    .expect("reference-bearing nominal is constructible");
    for (name, rhs, admitted) in [
        ("scalar", Ty::Int(ir::IntTy::I32), true),
        ("reference", reference.clone(), false),
        ("nested", Ty::Tuple(Arc::from([reference.clone()])), false),
        (
            "zero",
            Ty::Array {
                element: Arc::new(reference),
                len: 0,
            },
            false,
        ),
    ] {
        for callee in ["unused", "erased"] {
            let result = plan.trait_world().unwrap().instance_facts(&ir::CallTarget {
                declaration: declaration(&program, &plan, callee),
                type_args: Arc::from([receiver.clone(), rhs.clone()]),
            });
            if admitted {
                assert_eq!(result.unwrap().phase(), FactsPhase::Closed);
            } else {
                let error = result.expect_err("reference-bearing RHS acquired executable facts");
                assert_eq!(
                    error.kind(),
                    justrust_engine::compiler::traits::FactsErrorKind::Unsupported,
                    "{name}/{callee}: {error}"
                );
            }
        }
    }
}

#[test]
fn source_body_transplants_and_conflicting_rhs_rows_do_not_gain_authority() {
    let mut fixture = fixture("get");
    let foreign = frontend::parse(SOURCE).unwrap();
    let target = index(&fixture.program, "body_only");
    let donor = index(&foreign, "body_only");
    fixture.program.functions[target].body = foreign.functions[donor].body.clone();
    reject_retained_source(&fixture, "body_only");
    for bounds in ["Source<Out=U>+Source<Out=V>", "Source<Out=V>+Source<Out=U>"] {
        rejected(
            &format!(
                r#"
trait Source:Sized {{type Out;}}
fn unused<T:{bounds},U,V>() {{}}
fn main() {{}}
"#
            ),
            true,
        );
    }
}

#[test]
fn poisoned_original_budget_remains_authoritative_for_retained_evidence() {
    for bytes in [false, true] {
        let fixture = fixture("get");
        let (projection, _) = equalities(&fixture.facts).remove(0);
        let proof = fixture
            .facts
            .world()
            .normalize_projection_equality(&projection, &fixture.facts)
            .unwrap();
        let call = calls(&fixture.checked).remove(0);
        let view = fixture
            .facts
            .carrier_contract(SignatureSubject::Call(&call))
            .unwrap();
        if bytes {
            assert!(fixture.facts.budget().retain_bytes(usize::MAX).is_err())
        } else {
            assert!(fixture.facts.budget().check_depth(usize::MAX).is_err())
        }
        let failure = fixture.facts.budget().first_resource_failure().unwrap();
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
            fixture.facts.budget().first_resource_failure().unwrap(),
            failure
        );
    }
}
