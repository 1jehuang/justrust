//! Nominal construction must not reenter the plan mutex through metadata proof.
use super::*;

fn plan(budget: &ir::PlanBudget) -> PlanCtx {
    let source =
        parse("struct Inner<T>{value:T} struct Outer<T>{data:Inner<T>} fn main(){}").unwrap();
    PlanCtx::from_program(&source, budget).unwrap()
}

fn declaration(plan: &PlanCtx, index: usize) -> ir::DeclId {
    let source = &plan.program.structs[index];
    let key =
        name_scope::path_key(plan.program.names.as_ref(), &source.name, Namespace::Type).unwrap();
    *plan.registry.nominals.get(&key).unwrap()
}

#[test]
fn validated_nested_nominal_interning_replays_authority_after_unlock() {
    let budget = ir::PlanBudget::default();
    let plan = plan(&budget);
    let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
    let caller = ir::FactsCtx::Validated(&facts);
    let inner = plan
        .intern_nominal(
            declaration(&plan, 0),
            &[ir::Ty::Int(ir::IntTy::I32)],
            caller,
            &budget,
        )
        .unwrap();
    let outer = plan
        .intern_nominal(declaration(&plan, 1), &[inner.clone()], caller, &budget)
        .unwrap();
    caller
        .validate_ty_metadata_with_budget(&outer, &budget)
        .unwrap();
    let ir::Ty::Struct(outer) = outer else {
        panic!("lost outer nominal")
    };
    assert_eq!(outer.def().type_args.as_ref(), &[inner]);
}

#[test]
fn validated_nominal_interning_rejects_foreign_actual_and_world() {
    let budget = ir::PlanBudget::default();
    let local = plan(&budget);
    let foreign = plan(&budget);
    let local_facts = local
        .callable_facts(local.function_decl(0).unwrap())
        .unwrap();
    let foreign_facts = foreign
        .callable_facts(foreign.function_decl(0).unwrap())
        .unwrap();
    let actual = foreign
        .intern_nominal(
            declaration(&foreign, 0),
            &[ir::Ty::Int(ir::IntTy::I32)],
            ir::FactsCtx::Validated(&foreign_facts),
            &budget,
        )
        .unwrap();
    assert!(
        local
            .intern_nominal(
                declaration(&local, 1),
                &[actual],
                ir::FactsCtx::Validated(&local_facts),
                &budget
            )
            .is_err(),
        "foreign nominal actual gained canonical authority"
    );
    assert!(
        local
            .intern_nominal(
                declaration(&local, 0),
                &[ir::Ty::Int(ir::IntTy::I32)],
                ir::FactsCtx::Validated(&foreign_facts),
                &budget
            )
            .unwrap_err()
            .contains("authority")
    );
}
