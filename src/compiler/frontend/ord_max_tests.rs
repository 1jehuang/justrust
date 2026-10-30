//! Genuine source-backed Ord default selection and replay controls.
use super::*;
use crate::compiler::source;

fn genuine() -> Program {
    let options = source::SourceOptions::default();
    let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "ord-max-authority.rs",
            "trait Other{fn max(self,other:Self)->Self where Self:Sized;}fn main(){}",
            &options,
        )))
        .unwrap();
    let prepared = loop {
        match step {
            source::AssemblyStep::Done(prepared) => break prepared,
            source::AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None => source::prepare_prelude_unit(&options, &budget),
                    Some(parent) => {
                        source::prepare_child_unit(parent, need.index, &options, &budget)
                    }
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    };
    resolve(&parse_prepared(&prepared).unwrap()).unwrap()
}

#[test]
fn ord_max_default_signature_sized_and_member_world_replay() {
    ord_default_signature_replay("max", 2);
}

#[test]
fn ord_min_clamp_default_signature_sized_and_member_world_replay() {
    ord_default_signature_replay("min", 2);
    ord_default_signature_replay("clamp", 3);
}

fn ord_default_signature_replay(method: &str, arity: usize) {
    let program = genuine();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let trait_index = |name: &str| {
        program
            .traits
            .iter()
            .position(|declaration| {
                program
                    .names
                    .spelling(*declaration.name.segments().last().unwrap())
                    .unwrap()
                    == name
            })
            .unwrap()
    };
    let ord = trait_index("Ord");
    let other = trait_index("Other");
    let item = program.traits[ord]
        .items
        .iter()
        .position(|item| program.names.spelling(item.signature.name).unwrap() == method)
        .unwrap();
    let name = program.traits[ord].items[item].signature.name;
    let default = plan.registry.trait_default_callables[&(ord, item)];
    let default_facts = plan.callable_facts(default).unwrap();
    let sized = plan.catalog.id(tr::CatalogItem::Sized);
    assert!(
        default_facts
            .predicates()
            .predicates()
            .iter()
            .any(|predicate| {
                matches!(predicate, ir::Predicate::Implements(bound)
            if bound.declaration == sized && matches!(bound.self_ty, ir::Ty::Param(_)))
            }),
        "selected source default lost its Self:Sized obligation"
    );
    let main = program
        .functions
        .iter()
        .position(|function| {
            program
                .names
                .spelling(*function.name.segments().last().unwrap())
                .unwrap()
                == "main"
        })
        .unwrap();
    let facts = plan
        .callable_facts(plan.registry.function_ids[main])
        .unwrap();
    let i32_ty = ir::Ty::Int(ir::IntTy::I32);
    let trait_ref = ir::TraitRef {
        declaration: plan.registry.trait_ids[ord],
        self_ty: i32_ty,
        args: Arc::from([]),
    };
    let (callee, signature, receiver) = plan
        .select_trait(
            &trait_ref,
            &name,
            &[],
            ir::FactsCtx::Validated(&facts),
            false,
            &budget,
        )
        .unwrap()
        .unwrap();
    assert!(matches!(receiver, ReceiverKind::Value { .. }));
    assert_eq!(signature.params.len(), arity);
    assert!(
        signature
            .params
            .iter()
            .all(|ty| matches!(ty, ir::Ty::Int(ir::IntTy::I32)))
    );
    assert!(matches!(signature.result, ir::Ty::Int(ir::IntTy::I32)));
    facts
        .validate_signature(ir::SignatureSubject::Call(&callee), &signature)
        .unwrap();
    let ir::Callee::TraitItem(call) = &callee else {
        panic!("lost canonical trait call")
    };
    let resolved = facts.service().resolve_trait_call(call, &facts).unwrap();
    let ir::Callee::Direct(target) = resolved.callee() else {
        panic!("closed Ord default did not resolve")
    };
    assert_eq!(target.declaration, default);
    assert!(matches!(
        target.type_args.as_ref(),
        [ir::Ty::Int(ir::IntTy::I32)]
    ));
    let mut changed_signature = signature.clone();
    changed_signature.result = ir::Ty::Bool;
    assert!(
        facts
            .validate_signature(ir::SignatureSubject::Call(&callee), &changed_signature)
            .is_err()
    );
    let mut changed_argument = signature.clone();
    changed_argument.params[1] = ir::Ty::Bool;
    assert!(
        facts
            .validate_signature(ir::SignatureSubject::Call(&callee), &changed_argument)
            .is_err()
    );
    let mut foreign_member = call.clone();
    foreign_member.item = plan.registry.trait_items[other][0];
    assert!(
        facts
            .service()
            .instantiate_trait_signature(&foreign_member, &facts)
            .is_err()
    );
    assert!(
        facts
            .service()
            .resolve_trait_call(&foreign_member, &facts)
            .is_err()
    );
    let foreign_program = genuine();
    let foreign_plan = PlanCtx::from_program(&foreign_program, &ir::PlanBudget::default()).unwrap();
    let foreign_main = foreign_program
        .functions
        .iter()
        .position(|function| {
            foreign_program
                .names
                .spelling(*function.name.segments().last().unwrap())
                .unwrap()
                == "main"
        })
        .unwrap();
    let foreign_facts = foreign_plan
        .callable_facts(foreign_plan.registry.function_ids[foreign_main])
        .unwrap();
    assert!(
        foreign_facts
            .validate_signature(ir::SignatureSubject::Call(&callee), &signature)
            .is_err()
    );
    assert!(
        foreign_facts
            .service()
            .resolve_trait_call(call, &foreign_facts)
            .is_err()
    );
}
