//! Authority controls for reference-typed Self generic trait methods
//! (`impl<T: Tr> Tr for &T`, `fn m<S: Sink>(&self, s: &mut S)`), driven
//! through the real validated-facts path: a genuine source program is typed
//! with its own PlanCtx/ValidatedFacts, then the retained trait call's
//! signature reference contract is forged. Each forgery must be rejected by
//! checked-IR validation and by CFG ingress; nothing is constructed by hand
//! except the single mutated field under test.
use frontend::{PlanCtx, Program};
use ir::{Callee, CheckedFunction, ExprKind, FactsCtx, Signature, TraitCall};
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir};
use std::sync::Arc;

const SOURCE: &str = r#"
trait Sink { fn put(&mut self, n: usize); }
trait Feed { fn feed<S: Sink>(&self, s: &mut S); }
impl Feed for u32 { fn feed<S: Sink>(&self, s: &mut S) { s.put(*self as usize); } }
impl<T: Feed> Feed for &T { fn feed<S: Sink>(&self, s: &mut S) { (**self).feed(s); } }
struct Count { n: usize }
impl Sink for Count { fn put(&mut self, n: usize) { self.n += n; } }
fn forward<T: Feed, S: Sink>(t: &T, s: &mut S) { <&T as Feed>::feed::<S>(&t, s); }
fn main() { let mut c = Count { n: 0 }; let x = 7u32; forward(&x, &mut c); }
"#;

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

fn typed(name: &str) -> (CheckedFunction, Arc<ValidatedFacts>, Program) {
    let program = frontend::parse(SOURCE).expect("genuine reference-Self source");
    frontend::check(&program).expect("genuine reference-Self source checks");
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = declaration(&program, &plan, name);
    let checked = frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    checked
        .validate_with_facts(FactsCtx::Validated(&facts))
        .expect("unforged call validates");
    (checked, facts, program)
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
        .expect("retained trait-item call through &T")
}

fn reject(function: &CheckedFunction, facts: &Arc<ValidatedFacts>, what: &str) {
    assert!(
        function
            .validate_with_facts(FactsCtx::Validated(facts))
            .is_err(),
        "{what}: forged contract gained checked-call authority"
    );
    assert!(
        cfg::lower_template(Arc::new(function.clone()), facts.clone(), facts.budget()).is_err(),
        "{what}: forged contract bypassed exact-facts CFG ingress"
    );
}

#[test]
fn genuine_reference_self_call_validates() {
    let (checked, facts, _program) = typed("forward");
    let template = cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget());
    assert!(
        template.is_ok(),
        "genuine call must lower: {:?}",
        template.err()
    );
}

#[test]
fn dropped_inner_self_input_row_is_rejected() {
    let (mut checked, facts, _program) = typed("forward");
    let (_, signature) = call_mut(&mut checked);
    let mut contract = (*signature.ref_contract).clone();
    let before = contract.input_regions.len();
    let rows: Vec<_> = contract
        .input_regions
        .iter()
        .filter(|row| !(row.input.param == 0 && row.input.position.depth == 1))
        .cloned()
        .collect();
    assert!(
        rows.len() < before,
        "realized contract must carry the &&T inner row"
    );
    contract.input_regions = rows.into();
    signature.ref_contract = Arc::new(contract);
    reject(&checked, &facts, "dropped (param 0, depth 1) row");
}

#[test]
fn raw_trait_contract_substitution_is_rejected() {
    // Replace the realized contract with a single-layer (raw trait schema
    // shaped) one: param 0 depth 0 and param 1 depth 0 only.
    let (mut checked, facts, _program) = typed("forward");
    let (_, signature) = call_mut(&mut checked);
    let mut contract = (*signature.ref_contract).clone();
    contract.input_regions = contract
        .input_regions
        .iter()
        .filter(|row| row.input.position.depth == 0)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    signature.ref_contract = Arc::new(contract);
    reject(&checked, &facts, "raw single-layer contract");
}

#[test]
fn aliased_inner_and_outer_self_regions_are_rejected() {
    // Point the inner Self layer at the outer layer's region (alias), which
    // erases the inner-outlives-outer distinction.
    let (mut checked, facts, _program) = typed("forward");
    let (_, signature) = call_mut(&mut checked);
    let mut contract = (*signature.ref_contract).clone();
    let outer = contract
        .input_regions
        .iter()
        .find(|row| row.input.param == 0 && row.input.position.depth == 0)
        .map(|row| row.region)
        .expect("outer receiver row");
    let rows: Vec<_> = contract
        .input_regions
        .iter()
        .map(|row| {
            let mut row = row.clone();
            if row.input.param == 0 && row.input.position.depth == 1 {
                row.region = outer;
            }
            row
        })
        .collect();
    contract.input_regions = rows.into();
    signature.ref_contract = Arc::new(contract);
    reject(&checked, &facts, "aliased inner/outer Self region");
}
