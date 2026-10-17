//! Parameterized traits retain real source applications and exact canonical worlds.
//! No proof/evidence factory, synthetic nominal declaration or rewritten facts are used.
use frontend::{Expr, GenericPath, PlanCtx, Program, TraitPath, Type};
use ir::{Callee, CheckedFunction, ExprKind, FactsCtx, Signature, TraitCall, Ty};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir, ownership};
use std::sync::Arc;

const SOURCE: &str = r#"
struct S;
trait Convert<A>:Sized {
    fn convert(self,value:A)->A;
    fn echo(self,value:A)->A {value}
}
trait Other<A> {fn convert(self,value:A)->A;}
trait Duo<A,B> {fn first(self,left:A,right:B)->A;}
impl Convert<i32> for S {fn convert(self,value:i32)->i32{value}}
impl Convert<bool> for S {fn convert(self,value:bool)->bool{value}}
impl Duo<i32,bool> for S {fn first(self,left:i32,right:bool)->i32{left}}
impl Duo<bool,i32> for S {fn first(self,left:bool,right:i32)->bool{left}}
fn invoke<T:Convert<i32>>(value:T,input:i32)->i32 {<T as Convert<i32>>::convert(value,input)}
fn invoke_bool<T:Convert<bool>>(value:T,input:bool)->bool {<T as Convert<bool>>::convert(value,input)}
fn pair<T:Duo<i32,bool>>(value:T,left:i32,right:bool)->i32 {<T as Duo<i32,bool>>::first(value,left,right)}
fn alternate<U:Other<i32>>(value:U,input:i32)->i32 {<U as Other<i32>>::convert(value,input)}
fn actual_i32()->i32 {<S as Convert<i32>>::convert(S,4i32)}
fn actual_bool()->bool {<S as Convert<bool>>::convert(S,true)}
fn default_i32()->i32 {<S as Convert<i32>>::echo(S,5i32)}
fn default_bool()->bool {<S as Convert<bool>>::echo(S,false)}
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
fn trait_index(program: &Program, name: &str) -> usize {
    program
        .traits
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
fn bound<'a>(program: &'a Program, name: &str) -> &'a GenericPath {
    let TraitPath::Source(path) = &program.functions[index(program, name)].bounds[0].traits[0]
    else {
        panic!("actual source trait bound required")
    };
    path
}
fn bound_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut GenericPath {
    let index = index(program, name);
    let TraitPath::Source(path) = &mut program.functions[index].bounds[0].traits[0] else {
        panic!("actual source trait bound required")
    };
    path
}
fn ufcs_mut<'a>(program: &'a mut Program, name: &str) -> &'a mut GenericPath {
    let index = index(program, name);
    let Expr::TraitCall { trait_path, .. } =
        program.functions[index].body.tail.as_deref_mut().unwrap()
    else {
        panic!("honest source UFCS trait application required")
    };
    let TraitPath::Source(path) = trait_path.as_mut() else {
        panic!("actual source trait application required")
    };
    path
}
fn parsed() -> Program {
    let program = frontend::parse(SOURCE).expect("actual resolved parameterized trait source");
    frontend::check(&program)
        .expect("all original trait applications and unused universal bodies check");
    program
}
fn declaration(program: &Program, plan: &PlanCtx, name: &str) -> ir::DeclId {
    (0..plan.callable_count())
        .map(|index| plan.callable_decl(index).unwrap())
        .find(|declaration| {
            let function = plan.callable_body(program, *declaration).unwrap();
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap()
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
    ownership::verify_template(template).expect("same-facts universal ownership baseline");
    (checked, facts)
}
struct Fixture {
    checked: CheckedFunction,
    facts: Arc<ValidatedFacts>,
    boolean: CheckedFunction,
    pair: CheckedFunction,
    pair_facts: Arc<ValidatedFacts>,
    alternate: CheckedFunction,
    _program: Program,
}
fn fixture() -> Fixture {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (checked, facts) = typed(&program, &plan, &budget, "invoke");
    let (boolean, _) = typed(&program, &plan, &budget, "invoke_bool");
    let (pair, pair_facts) = typed(&program, &plan, &budget, "pair");
    let (alternate, _) = typed(&program, &plan, &budget, "alternate");
    Fixture {
        checked,
        facts,
        boolean,
        pair,
        pair_facts,
        alternate,
        _program: program,
    }
}
fn call(function: &CheckedFunction) -> (&TraitCall, &Signature) {
    function
        .exprs
        .iter()
        .find_map(|expression| match &expression.kind {
            ExprKind::Call {
                target: Callee::TraitItem(call),
                signature,
                ..
            } => Some((call, signature)),
            _ => None,
        })
        .expect("real universal trait-item call retained before closed dispatch")
}
fn call_mut(function: &mut CheckedFunction) -> (&mut TraitCall, &mut Signature) {
    function
        .exprs
        .iter_mut()
        .find_map(|expression| match &mut expression.kind {
            ExprKind::Call {
                target: Callee::TraitItem(call),
                signature,
                ..
            } => Some((call, signature)),
            _ => None,
        })
        .expect("real universal trait-item call retained before closed dispatch")
}
fn reject_ir(function: &CheckedFunction, facts: &Arc<ValidatedFacts>) {
    let error = function
        .validate_with_facts(FactsCtx::Validated(facts))
        .expect_err("forged trait application gained checked-call authority");
    assert!(!error.is_empty());
    assert!(
        cfg::lower_template(Arc::new(function.clone()), facts.clone(), facts.budget()).is_err(),
        "trait application bypassed exact-facts CFG ingress"
    );
}
fn reject_source(program: Program) {
    let budget = ir::PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("foreign trait source gained authority"),
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
fn semantic_rejection(source: &str) {
    let error = frontend::parse(source)
        .and_then(|program| frontend::check(&program))
        .expect_err("invalid generic trait source was admitted");
    assert!(
        !error.to_ascii_lowercase().contains("unsupported"),
        "wrong boundary: {error}"
    );
}
#[test]
fn exact_source_clone_retains_parameterized_trait_owners() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    PlanCtx::from_shared_program(Arc::new(program.clone()), &budget).unwrap();
    let convert = &program.traits[trait_index(&program, "Convert")];
    let duo = &program.traits[trait_index(&program, "Duo")];
    assert_eq!(convert.type_params.len(), 1);
    assert_eq!(duo.type_params.len(), 2);
}
#[test]
fn same_self_distinct_trait_arguments_select_real_distinct_implementations() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (integers, integer_facts) = typed(&program, &plan, &budget, "actual_i32");
    let (booleans, boolean_facts) = typed(&program, &plan, &budget, "actual_bool");
    let direct = |function: &CheckedFunction| {
        function
            .exprs
            .iter()
            .find_map(|expression| match &expression.kind {
                ExprKind::Call {
                    target: Callee::Direct(target),
                    signature,
                    ..
                } => Some((target.declaration, signature.clone())),
                _ => None,
            })
            .unwrap()
    };
    let (integer_target, integer_signature) = direct(&integers);
    let (boolean_target, boolean_signature) = direct(&booleans);
    assert_ne!(integer_target, boolean_target);
    assert_eq!(integer_signature.params[0], boolean_signature.params[0]);
    assert_eq!(integer_signature.params[1], Ty::Int(ir::IntTy::I32));
    assert_eq!(boolean_signature.params[1], Ty::Bool);
    for (checked, facts) in [(&integers, &integer_facts), (&booleans, &boolean_facts)] {
        let template =
            cfg::lower_template(Arc::new(checked.clone()), facts.clone(), facts.budget()).unwrap();
        let verified = ownership::verify_template(template)
            .expect("selected concrete callable under original universal facts");
        assert!(verified.analysis().passed());
    }
}
#[test]
fn default_provider_substitutes_actual_trait_parameter_not_only_self() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    for (name, result) in [
        ("default_i32", Ty::Int(ir::IntTy::I32)),
        ("default_bool", Ty::Bool),
    ] {
        let (checked, _) = typed(&program, &plan, &budget, name);
        let signature = checked
            .exprs
            .iter()
            .find_map(|expression| match &expression.kind {
                ExprKind::Call { signature, .. } => Some(signature),
                _ => None,
            })
            .unwrap();
        assert_eq!(signature.params[1], result);
        assert_eq!(signature.result, result);
    }
}
#[test]
fn universal_call_preserves_actual_caller_param_and_full_trait_application() {
    let program = parsed();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let decl = declaration(&program, &plan, "invoke");
    let env = plan.callable_env(decl).unwrap();
    let params: Vec<_> = env.ordered_params().collect();
    assert_eq!(params.len(), 1);
    let (checked, _) = typed(&program, &plan, &budget, "invoke");
    let (call, signature) = call(&checked);
    assert_eq!(call.trait_ref.self_ty, Ty::Param(params[0]));
    assert_eq!(call.trait_ref.args.as_ref(), &[Ty::Int(ir::IntTy::I32)]);
    assert_eq!(signature.params[0], Ty::Param(params[0]));
    assert_eq!(signature.result, Ty::Int(ir::IntTy::I32));
}
#[test]
fn ordered_two_parameter_trait_application_remains_distinct() {
    let fixture = fixture();
    let (call, signature) = call(&fixture.pair);
    assert_eq!(
        call.trait_ref.args.as_ref(),
        &[Ty::Int(ir::IntTy::I32), Ty::Bool]
    );
    assert_eq!(&signature.params[1..], &[Ty::Int(ir::IntTy::I32), Ty::Bool]);
}
#[test]
fn original_bound_cannot_authorize_another_trait_argument() {
    let mut fixture = fixture();
    let donor = call(&fixture.boolean).0.trait_ref.args.clone();
    call_mut(&mut fixture.checked).0.trait_ref.args = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn original_bound_cannot_authorize_trait_argument_permutation() {
    let mut fixture = fixture();
    let target = &mut call_mut(&mut fixture.pair).0.trait_ref;
    let mut args = target.args.to_vec();
    args.swap(0, 1);
    target.args = args.into();
    reject_ir(&fixture.pair, &fixture.pair_facts);
}
#[test]
fn trait_argument_arity_cannot_be_erased_under_original_evidence() {
    let mut fixture = fixture();
    call_mut(&mut fixture.checked).0.trait_ref.args = [].into();
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn trait_argument_arity_cannot_be_extended_under_original_evidence() {
    let mut fixture = fixture();
    let target = &mut call_mut(&mut fixture.checked).0.trait_ref;
    let mut args = target.args.to_vec();
    args.push(args[0].clone());
    target.args = args.into();
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn actual_other_trait_item_cannot_replace_original_item_owner() {
    let mut fixture = fixture();
    let donor = call(&fixture.alternate).0.item;
    call_mut(&mut fixture.checked).0.item = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn actual_other_trait_declaration_cannot_keep_original_item() {
    let mut fixture = fixture();
    let donor = call(&fixture.alternate).0.trait_ref.declaration;
    call_mut(&mut fixture.checked).0.trait_ref.declaration = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn same_spelling_other_caller_param_is_not_original_self_binder() {
    let mut fixture = fixture();
    let donor = call(&fixture.boolean).0.trait_ref.self_ty.clone();
    assert_ne!(call(&fixture.checked).0.trait_ref.self_ty, donor);
    call_mut(&mut fixture.checked).0.trait_ref.self_ty = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn unchanged_call_evidence_does_not_repair_wrong_substituted_signature() {
    let mut fixture = fixture();
    let donor = call(&fixture.boolean).1.result.clone();
    call_mut(&mut fixture.checked).1.result = donor;
    reject_ir(&fixture.checked, &fixture.facts);
}
#[test]
fn resolved_application_is_checked_against_original_bound_not_callee_spelling() {
    let fixture = fixture();
    let (original, _) = call(&fixture.checked);
    fixture
        .facts
        .service()
        .resolve_trait_call(original, &fixture.facts)
        .unwrap();
    let mut changed = original.clone();
    changed.trait_ref.args = call(&fixture.boolean).0.trait_ref.args.clone();
    assert!(
        fixture
            .facts
            .service()
            .resolve_trait_call(&changed, &fixture.facts)
            .is_err()
    );
}
#[test]
fn actual_foreign_world_cannot_replay_original_universal_call_or_facts() {
    let fixture = fixture();
    let foreign = self::fixture();
    let (original, _) = call(&fixture.checked);
    assert!(
        foreign
            .facts
            .service()
            .resolve_trait_call(original, &foreign.facts)
            .is_err()
    );
    assert!(
        fixture
            .checked
            .validate_with_facts(FactsCtx::Validated(&foreign.facts))
            .is_err()
    );
}
#[test]
fn foreign_original_bound_application_is_rejected_before_planning_authority() {
    let mut program = parsed();
    let foreign = parsed();
    let donor = bound(&foreign, "invoke").clone();
    *bound_mut(&mut program, "invoke") = donor;
    reject_source(program);
}
#[test]
fn foreign_original_ufcs_trait_application_is_rejected_before_authority() {
    let mut program = parsed();
    let mut foreign = parsed();
    let donor = ufcs_mut(&mut foreign, "invoke").clone();
    *ufcs_mut(&mut program, "invoke") = donor;
    reject_source(program);
}
#[test]
fn foreign_type_argument_occurrence_cannot_hide_inside_original_trait_path() {
    let mut program = parsed();
    let foreign = parsed();
    let donor = foreign.functions[index(&foreign, "invoke")].params[0]
        .1
        .clone();
    bound_mut(&mut program, "invoke")
        .segments
        .last_mut()
        .unwrap()
        .args[0] = donor;
    reject_source(program);
}
#[test]
fn foreign_trait_parameter_declaration_cannot_keep_original_method_schemas() {
    let mut program = parsed();
    let foreign = parsed();
    let donor = foreign.traits[trait_index(&foreign, "Convert")].type_params[0];
    let index = trait_index(&program, "Convert");
    program.traits[index].type_params[0] = donor;
    reject_source(program);
}
#[test]
fn unused_generic_trait_method_body_cannot_import_foreign_source() {
    let mut program = parsed();
    let foreign = parsed();
    let index = trait_index(&program, "Convert");
    let donor = foreign.traits[trait_index(&foreign, "Convert")].items[1]
        .default
        .as_ref()
        .unwrap()
        .body
        .clone();
    program.traits[index].items[1]
        .default
        .as_mut()
        .unwrap()
        .body = donor;
    reject_source(program);
}
#[test]
fn missing_explicit_source_trait_argument_is_wrong_arity_not_inferred_permission() {
    let mut program = parsed();
    bound_mut(&mut program, "invoke")
        .segments
        .last_mut()
        .unwrap()
        .args
        .clear();
    let error = frontend::check(&program)
        .expect_err("missing trait application argument was silently inferred");
    assert!(
        error.to_ascii_lowercase().contains("arity")
            || error.to_ascii_lowercase().contains("argument"),
        "{error}"
    );
}
#[test]
fn extra_explicit_source_trait_argument_is_wrong_arity() {
    let mut program = parsed();
    bound_mut(&mut program, "invoke")
        .segments
        .last_mut()
        .unwrap()
        .args
        .push(Type::Bool);
    let error =
        frontend::check(&program).expect_err("extra trait application argument was ignored");
    assert!(
        error.to_ascii_lowercase().contains("arity")
            || error.to_ascii_lowercase().contains("argument"),
        "{error}"
    );
}
#[test]
fn unused_universal_body_cannot_replace_required_i32_bound_with_bool() {
    semantic_rejection(
        "trait Convert<A>{fn convert(self,value:A)->A;}fn bad<T:Convert<i32>>(value:T,input:bool)->bool{<T as Convert<bool>>::convert(value,input)}fn main(){}",
    );
}
#[test]
fn favorable_concrete_impl_cannot_repair_missing_universal_bound() {
    semantic_rejection(
        "struct S;trait Convert<A>{fn convert(self,value:A)->A;}impl Convert<i32> for S{fn convert(self,value:i32)->i32{value}}fn bad<T>(value:T,input:i32)->i32{<T as Convert<i32>>::convert(value,input)}fn main(){}",
    );
}
#[test]
fn overlapping_same_application_impls_remain_invalid() {
    semantic_rejection(
        "struct S;trait Convert<A>{fn convert(self,value:A)->A;}impl Convert<i32> for S{fn convert(self,value:i32)->i32{value}}impl Convert<i32> for S{fn convert(self,value:i32)->i32{value}}fn main(){}",
    );
}
#[test]
fn source_name_owner_swap_cannot_be_laundered_by_trait_application() {
    let mut program = parsed();
    program.names = parsed().names;
    reject_source(program);
}
