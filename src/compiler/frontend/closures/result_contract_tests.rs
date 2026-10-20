use super::*;

fn fixture() -> (PlanCtx, super::super::check::CheckedClosure, ir::PlanBudget) {
    use crate::compiler::source;
    let options = source::SourceOptions::default();
    let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler.provide(Arc::new(source::prepare_text_unit(
        "closure-symbolic-result.rs",
        "fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C){let composed=move |x:A|g(f(x));}fn main(){}",
        &options,
    ))).unwrap();
    let prepared = loop {
        match step {
            source::AssemblyStep::Done(prepared) => break prepared,
            source::AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None => source::prepare_prelude_unit(&options, &expansion),
                    Some(parent) => {
                        source::prepare_child_unit(parent, need.index, &options, &expansion)
                    }
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    };
    let program = resolve(&parse_prepared(&prepared).unwrap()).unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let (provider, owner) = (0..plan.callable_count())
        .find_map(|index| {
            let provider = plan.callable_decl(index).ok()?;
            plan.stored_closure_owner(provider)
                .ok()
                .flatten()
                .map(|(owner, _)| (provider, owner))
        })
        .unwrap();
    let (_, closures) = super::super::super::type_check::type_callable_source_and_closures(
        &program, owner, &plan, &budget, None,
    )
    .unwrap();
    let checked = closures
        .iter()
        .find(|closure| closure.ty.provider() == provider)
        .unwrap()
        .clone();
    (plan, checked, budget)
}

#[test]
fn higher_order_symbolic_result_replays_original_call_contract() {
    let (plan, checked, budget) = fixture();
    let owner = checked.ty.owner();
    let provider = checked.ty.provider();
    let receipt = checked
        .ty
        .result_receipt()
        .expect("symbolic C requires a real result receipt");
    assert_eq!(receipt.owner(), owner);
    assert_eq!(receipt.provider(), provider);
    assert!(matches!(checked.ty.result(), ir::Ty::Param(_)));
    let world = plan.trait_world().unwrap();
    for mode in [
        ir::ClosureKind::Fn,
        ir::ClosureKind::FnMut,
        ir::ClosureKind::FnOnce,
    ] {
        let facts = world
            .closure_callable_facts_for(checked.ty.clone(), mode)
            .unwrap();
        let source = checked.body_for(mode).unwrap();
        validate_source(receipt, &source, &facts).unwrap();
        let view = facts.carrier_contract(ir::SignatureSubject::Body).unwrap();
        view.validate_for(&facts).unwrap();
        assert_eq!(view.contract().results.len(), 1);
        let template =
            crate::compiler::cfg::lower_closure_template(source, facts, &checked, mode, &budget)
                .unwrap();
        crate::compiler::ownership::verify_template(template).unwrap();
    }
}

#[test]
fn restricted_result_receipts_reject_foreign_source_context_and_reference_actuals() {
    let (plan, checked, budget) = fixture();
    let world = plan.trait_world().unwrap();
    let facts = world
        .closure_callable_facts_for(checked.ty.clone(), ir::ClosureKind::Fn)
        .unwrap();
    let source = checked.body_for(ir::ClosureKind::Fn).unwrap();
    let original = checked.ty.result_receipt().unwrap();
    let ir::ExprKind::Call {
        target: ir::Callee::TraitItem(call),
        args,
        signature,
        ..
    } = &source.exprs[original.tail().0].kind
    else {
        panic!("expected original required call")
    };
    let make = |owner, provider, site, tail, call, args, signature| {
        Arc::new(
            ir::ClosureResultReceipt::new(
                owner, provider, site, tail, call, args, signature, &budget,
            )
            .unwrap(),
        )
    };
    let mut changed_member = call.clone();
    changed_member.item = checked.ty.provider();
    let mut changed_actuals = call.clone();
    changed_actuals.method_args = Arc::from([ir::Ty::Unit]);
    let mut changed_result = signature.clone();
    changed_result.result = ir::Ty::Unit;
    let mut changed_args = args.clone();
    changed_args[0] = original.tail();
    let forged = [
        (
            "owner",
            make(
                checked.ty.provider(),
                checked.ty.provider(),
                checked.ty.site(),
                original.tail(),
                call.clone(),
                args.clone().into(),
                signature.clone(),
            ),
        ),
        (
            "provider",
            make(
                checked.ty.owner(),
                checked.ty.owner(),
                checked.ty.site(),
                original.tail(),
                call.clone(),
                args.clone().into(),
                signature.clone(),
            ),
        ),
        (
            "site",
            make(
                checked.ty.owner(),
                checked.ty.provider(),
                checked.ty.site() + 1,
                original.tail(),
                call.clone(),
                args.clone().into(),
                signature.clone(),
            ),
        ),
        (
            "tail",
            make(
                checked.ty.owner(),
                checked.ty.provider(),
                checked.ty.site(),
                ir::ExprId(original.tail().0 + 1),
                call.clone(),
                args.clone().into(),
                signature.clone(),
            ),
        ),
        (
            "member",
            make(
                checked.ty.owner(),
                checked.ty.provider(),
                checked.ty.site(),
                original.tail(),
                changed_member,
                args.clone().into(),
                signature.clone(),
            ),
        ),
        (
            "ordered actuals",
            make(
                checked.ty.owner(),
                checked.ty.provider(),
                checked.ty.site(),
                original.tail(),
                changed_actuals,
                args.clone().into(),
                signature.clone(),
            ),
        ),
        (
            "argument occurrence",
            make(
                checked.ty.owner(),
                checked.ty.provider(),
                checked.ty.site(),
                original.tail(),
                call.clone(),
                changed_args.into(),
                signature.clone(),
            ),
        ),
        (
            "result witness",
            make(
                checked.ty.owner(),
                checked.ty.provider(),
                checked.ty.site(),
                original.tail(),
                call.clone(),
                args.clone().into(),
                changed_result,
            ),
        ),
    ];
    for (label, receipt) in forged {
        assert!(
            validate_source(&receipt, &source, &facts).is_err(),
            "{label} source replay admitted"
        );
        let ty = Arc::new((*checked.ty).clone().with_result_receipt(Some(receipt)));
        assert!(
            world
                .closure_callable_facts_for(ty, ir::ClosureKind::Fn)
                .is_err(),
            "{label} copied seal admitted"
        );
    }
    let missing = Arc::new((*checked.ty).clone().with_result_receipt(None));
    assert!(
        world
            .closure_callable_facts_for(missing, ir::ClosureKind::Fn)
            .is_err()
    );
    let view = facts.carrier_contract(ir::SignatureSubject::Body).unwrap();
    let wrong_mode = world
        .closure_callable_facts_for(checked.ty.clone(), ir::ClosureKind::FnMut)
        .unwrap();
    assert!(view.validate_for(&wrong_mode).is_err());
    assert!(
        view.validate_for(&plan.callable_facts(checked.ty.owner()).unwrap())
            .is_err()
    );
    assert!(
        view.validate_for(&world.schema_facts(call.item).unwrap())
            .is_err()
    );
    let (foreign_plan, foreign_checked, _) = fixture();
    let foreign_facts = foreign_plan
        .trait_world()
        .unwrap()
        .closure_callable_facts_for(foreign_checked.ty, ir::ClosureKind::Fn)
        .unwrap();
    assert!(view.validate_for(&foreign_facts).is_err());
    let result_parameter = checked.ty.result().clone();
    for reference in [
        ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Int(ir::IntTy::I32)),
        },
        ir::Ty::Tuple(Arc::from([ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(ir::Ty::Int(ir::IntTy::I32)),
        }])),
    ] {
        let actual = Arc::new(
            checked
                .ty
                .try_map::<String>(|ty| {
                    Ok(if ty == &result_parameter {
                        reference.clone()
                    } else {
                        ty.clone()
                    })
                })
                .unwrap(),
        );
        assert!(
            world
                .closure_callable_facts_for(actual, ir::ClosureKind::Fn)
                .is_err(),
            "reference-bearing substitution admitted"
        );
    }
    let mut parameters = checked.ty.owner_args().to_vec();
    parameters.swap(0, 1);
    let reordered = Arc::new(
        ir::ClosureTy::new(
            checked.ty.owner(),
            parameters.into(),
            checked.ty.site(),
            checked.ty.provider(),
            checked.ty.params().into(),
            checked.ty.result().clone(),
            checked.ty.captures().into(),
            checked.ty.kind(),
        )
        .with_sources(checked.ty.sources().into())
        .with_result_receipt(checked.ty.result_receipt().cloned())
        .with_shape_seal(checked.ty.shape_seal().cloned()),
    );
    assert!(
        world
            .closure_callable_facts_for(reordered, ir::ClosureKind::Fn)
            .is_err(),
        "reordered owner binding admitted"
    );
}
