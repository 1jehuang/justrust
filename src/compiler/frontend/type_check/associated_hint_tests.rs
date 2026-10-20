//! Genuine caller/Call-subject controls for associated-result hint mapping.
//! Callback substitutions here are adversarial metadata, not graph capabilities.
use super::*;

#[test]
fn associated_hint_preserves_missing_and_rejects_wrong_mapped_calls() {
    let program = parse("struct Wrap<T>{value:T}struct Other<T>{value:T}impl<T> Wrap<T>{fn make(value:T)->Self{Self{value}}}fn probe()->Wrap<u32>{Wrap::<u32>::make(4u32)}fn wrong()->Other<u32>{Other{value:1u32}}fn changed()->Wrap<bool>{Wrap{value:true}}fn main(){}").unwrap();
    let signatures = signature_table(&program).unwrap();
    let plan = signatures.plan().unwrap();
    let declaration = |spelling: &str| {
        let index = program
            .functions
            .iter()
            .position(|function| {
                function
                    .name
                    .segments()
                    .last()
                    .is_some_and(|name| program.names.spelling(*name).unwrap() == spelling)
            })
            .unwrap();
        plan.function_decl(index).unwrap()
    };
    let probe = declaration("probe");
    let function = plan.callable_body(&program, probe).unwrap();
    let Some(expression) = function.body.tail.as_deref() else {
        panic!("missing canonical source tail")
    };
    let Expr::AssociatedCall {
        owner,
        name,
        type_args,
        args,
    } = expression
    else {
        panic!("expected canonical factory occurrence")
    };
    let mut caller = plan.environment(Some(probe)).unwrap();
    caller.facts = Some(plan.callable_facts(probe).unwrap());
    let schema = |_| Ok(None);
    let source = |_: &Type| Ok(None);
    let missing = |_: &NominalDemand| Ok(None);
    let inputs = NominalPreparationInputs::new(&schema, &source, &missing);
    let mut checker = Checker::new(&caller, ir::Ty::Unit, function.module);
    checker
        .install_lexical_scope(function.lexical_scope)
        .unwrap();
    checker.declaration = Some(probe);
    checker.nominal_inputs = Some(&inputs);
    let error = checker
        .associated_result_hint(owner, name, type_args, args)
        .unwrap_err();
    assert!(
        error.contains("missing graph input: associated result hint"),
        "{error}"
    );
    let Some(NominalPreparation::Blocked {
        nominals,
        needs_source_schemas,
        unresolved_constraints,
        ..
    }) = checker.nominal_blocked.borrow_mut().take()
    else {
        panic!("missing typed input must preserve exact blocked inventory")
    };
    assert_eq!(nominals.len(), 1);
    assert!(!needs_source_schemas);
    assert!(!unresolved_constraints);
    assert!(matches!(
        nominals[0].arguments.as_ref(),
        [NominalShape::Leaf(ir::Ty::Int(ir::IntTy::U32))]
    ));
    checker.nominal_inputs = None;
    for altered in ["wrong", "changed"] {
        let ty = plan
            .function_schema(declaration(altered))
            .unwrap()
            .signature
            .result;
        let replace = |_: &NominalDemand| Ok(Some(ty.clone()));
        let inputs = NominalPreparationInputs::new(&schema, &source, &replace);
        let mut checker = Checker::new(&caller, ir::Ty::Unit, function.module);
        checker
            .install_lexical_scope(function.lexical_scope)
            .unwrap();
        checker.declaration = Some(probe);
        checker.nominal_inputs = Some(&inputs);
        assert!(
            checker
                .associated_result_hint(owner, name, type_args, args)
                .is_err(),
            "{altered} metadata must not authorize a different canonical Call result"
        );
        assert!(
            checker.nominal_blocked.borrow().is_none(),
            "invalid mapped metadata must not become a missing-input retry"
        );
    }
}
