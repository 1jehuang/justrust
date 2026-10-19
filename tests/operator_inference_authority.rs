//! Real operator calls retain exact source, ordered arguments and selected Output.
use justrust_engine::compiler::traits::ValidatedFacts;
use justrust_engine::compiler::{cfg, frontend, ir};
#[path = "support/native_prelude.rs"]
mod native_prelude;
use std::sync::Arc;

const SOURCE: &str = "use std::ops::Mul;struct A{v:i32}impl Mul<u8> for A{type Output=i32;fn mul(self,n:u8)->i32{self.v*n as i32}}fn compute(a:A,n:u8)->i32{a*n}fn main(){}";

fn fixture(source: &str) -> (ir::CheckedFunction, Arc<ValidatedFacts>) {
    let program = native_prelude::parse(source).unwrap();
    // The native prelude has bodies requiring canonical source capabilities.
    // Build its validated plan, then check the exact operator consumer below.
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let index = program
        .functions
        .iter()
        .position(|function| {
            function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == "compute"
        })
        .unwrap();
    let declaration = plan.function_decl(index).unwrap();
    let checked = frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan.callable_facts(declaration).unwrap();
    checked
        .validate_with_facts(ir::FactsCtx::Validated(&facts))
        .unwrap();
    cfg::lower_template(Arc::new(checked.clone()), facts.clone(), facts.budget()).unwrap();
    (checked, facts)
}

fn reject(checked: ir::CheckedFunction, facts: &Arc<ValidatedFacts>) {
    assert!(
        checked
            .validate_with_facts(ir::FactsCtx::Validated(facts))
            .is_err()
    );
    assert!(cfg::lower_template(Arc::new(checked), facts.clone(), facts.budget()).is_err());
}

#[test]
fn canonical_operator_calls_reject_foreign_world_rhs_order_and_output_substitution() {
    reject_forged_call(SOURCE);
}

#[test]
fn inferred_narrow_rhs_does_not_grant_forged_call_authority() {
    reject_forged_call(
        "use std::ops::Mul;struct A{v:i32}impl Mul<u8> for A{type Output=i32;fn mul(self,n:u8)->i32{self.v*n as i32}}fn compute(a:A)->i32{a*2}fn main(){}",
    );
}

fn reject_forged_call(source: &str) {
    let (checked, facts) = fixture(source);
    let (foreign, foreign_facts) = fixture(source);
    reject(checked.clone(), &foreign_facts);
    let call = checked
        .exprs
        .iter()
        .position(|e| matches!(e.kind, ir::ExprKind::Call { .. }))
        .unwrap();
    let foreign_call = foreign
        .exprs
        .iter()
        .find_map(|e| match &e.kind {
            ir::ExprKind::Call { target, .. } => Some(target.clone()),
            _ => None,
        })
        .unwrap();
    for mutation in 0..4 {
        let mut forged = checked.clone();
        let ir::ExprKind::Call {
            target,
            args,
            signature,
            ..
        } = &mut forged.exprs[call].kind
        else {
            unreachable!()
        };
        match mutation {
            0 => args.reverse(),
            1 => signature.params[1] = ir::Ty::Int(ir::IntTy::U16),
            2 => signature.result = ir::Ty::Bool,
            3 => *target = foreign_call.clone(),
            _ => unreachable!(),
        }
        reject(forged, &facts);
    }
}
