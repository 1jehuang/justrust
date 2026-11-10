use super::*;

#[test]
fn static_string_result_retains_empty_origins_and_replays_literal_source() {
    let (plan, checked, _) =
        fixture_graph_source("fn inspect(){let f=||\"static text\";}fn main(){}");
    let world = plan.trait_world().unwrap();
    let receipt = checked.ty.result_receipt().unwrap();
    assert!(receipt.is_static_str());
    for mode in [
        ir::ClosureKind::Fn,
        ir::ClosureKind::FnMut,
        ir::ClosureKind::FnOnce,
    ] {
        let facts = world
            .closure_callable_facts_for(checked.ty.clone(), mode)
            .unwrap();
        let body = checked.body_for(mode).unwrap();
        validate_source(receipt, &body, &facts).unwrap();
        let view = facts.carrier_contract(ir::SignatureSubject::Body).unwrap();
        view.validate_for(&facts).unwrap();
        assert_eq!(view.contract().results.len(), 1);
        assert!(view.contract().results[0].sources.is_empty());
        let mut forged = body.as_ref().clone();
        forged.exprs[receipt.tail().0].kind = ir::ExprKind::Local(forged.params[0]);
        assert!(validate_source(receipt, &forged, &facts).is_err());
        let mut forged = body.as_ref().clone();
        forged.result = ir::Ty::Unit;
        assert!(validate_source(receipt, &forged, &facts).is_err());
    }
}

const ARGUMENT_STR_SOURCE: &str = "struct Item{name:String}fn inspect(items:Vec<&Item>){let _names:Vec<&str>=items.iter().map(|i|i.name.as_str()).collect();}fn main(){}";

#[test]
fn captured_result_read_preserves_inner_reference_channel_and_source_place() {
    const SOURCE: &str = "fn inspect(bytes:&[u8]){let f=||bytes.iter().cloned();}fn main(){}";
    let (plan, checked, _) = fixture_graph_source(SOURCE);
    let world = plan.trait_world().unwrap();
    let receipt = checked.ty.result_receipt().unwrap();
    let witness = receipt.capture_witness().unwrap();
    let read = witness
        .nodes
        .iter()
        .find(|node| matches!(node.kind, ir::CaptureResultKind::Read { .. }))
        .unwrap()
        .expression;
    for mode in [
        ir::ClosureKind::Fn,
        ir::ClosureKind::FnMut,
        ir::ClosureKind::FnOnce,
    ] {
        let facts = world
            .closure_callable_facts_for(checked.ty.clone(), mode)
            .unwrap();
        let body = checked.body_for(mode).unwrap();
        validate_source(receipt, &body, &facts).unwrap();
        let view = facts.carrier_contract(ir::SignatureSubject::Body).unwrap();
        view.validate_for(&facts).unwrap();
        assert!(!view.contract().results.is_empty());
        for returned in view.contract().results.iter() {
            assert_eq!(returned.sources.len(), 1);
            let source = &returned.sources[0];
            assert_eq!(source.param, 0);
            assert_eq!(
                source.location.channel,
                ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 })
            );
            let expected_path: &[ir::CarrierStep] = if mode == ir::ClosureKind::FnOnce {
                &[ir::CarrierStep::Field(0)]
            } else {
                &[ir::CarrierStep::Referent, ir::CarrierStep::Field(0)]
            };
            assert_eq!(source.location.path.0.as_ref(), expected_path);
        }
        for mutation in 0..4 {
            let mut forged = body.as_ref().clone();
            match mutation {
                0 => {
                    // Keep the checked result type but replace reading the
                    // inner reference with the outer capture-field reference.
                    let ir::ExprKind::Deref { value } = forged.exprs[read.0].kind else {
                        panic!("capture read")
                    };
                    forged.exprs[read.0].kind = forged.exprs[value.0].kind.clone();
                }
                1 => forged.exprs[read.0].kind = ir::ExprKind::Local(forged.params[1]),
                2 => {
                    let ir::Ty::Ref { referent, .. } = &forged.exprs[read.0].ty else {
                        panic!("shared read")
                    };
                    forged.exprs[read.0].ty = ir::Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        referent: referent.clone(),
                    };
                }
                3 => forged.exprs[read.0].kind = ir::ExprKind::Local(forged.params[0]),
                _ => unreachable!(),
            }
            assert!(
                validate_source(receipt, &forged, &facts).is_err(),
                "mode={mode:?} mutation={mutation}"
            );
        }
    }
    for mutation in 0..4 {
        let mut witness = witness.clone();
        let mut nodes = witness.nodes.to_vec();
        let node = nodes
            .iter_mut()
            .find(|node| node.expression == read)
            .unwrap();
        let ir::CaptureResultKind::Read {
            source,
            original,
            prefix,
            ..
        } = &mut node.kind
        else {
            panic!("read")
        };
        match mutation {
            0 => source.kind = ir::CaptureKind::Mut,
            1 => source.place.local = ir::LocalId(source.place.local.0 + 1),
            2 => original.local = ir::LocalId(original.local.0 + 1),
            3 => *prefix = Arc::from([ir::Projection::Deref]),
            _ => unreachable!(),
        }
        witness.nodes = nodes.into();
        let forged_receipt = Arc::new(
            ir::ClosureResultReceipt::captured(
                receipt.owner(),
                receipt.provider(),
                receipt.site(),
                receipt.tail(),
                witness,
                &plan.budget,
            )
            .unwrap(),
        );
        let forged = Arc::new(
            checked
                .ty
                .as_ref()
                .clone()
                .with_result_receipt(Some(forged_receipt)),
        );
        assert!(
            world
                .closure_callable_facts_for(forged, ir::ClosureKind::Fn)
                .is_err()
        );
    }
}

#[test]
fn captured_result_replays_original_capture_call_and_current_context() {
    const SOURCE: &str =
        "fn inspect(){let bytes=[2u8,3u8];let f=||bytes.iter().cloned();}fn main(){}";
    let (plan, checked, _) = fixture_graph_source(SOURCE);
    let world = plan.trait_world().unwrap();
    let receipt = checked.ty.result_receipt().unwrap();
    let witness = receipt.capture_witness().unwrap();
    let borrow = witness
        .nodes
        .iter()
        .find(|node| matches!(node.kind, ir::CaptureResultKind::Borrow { .. }))
        .unwrap()
        .expression;
    for mode in [
        ir::ClosureKind::Fn,
        ir::ClosureKind::FnMut,
        ir::ClosureKind::FnOnce,
    ] {
        let facts = world
            .closure_callable_facts_for(checked.ty.clone(), mode)
            .unwrap();
        let body = checked.body_for(mode).unwrap();
        validate_source(receipt, &body, &facts).unwrap();
        let view = facts.carrier_contract(ir::SignatureSubject::Body).unwrap();
        view.validate_for(&facts).unwrap();
        assert!(!view.contract().results.is_empty());
        for returned in view.contract().results.iter() {
            assert!(!returned.sources.is_empty());
            assert!(returned.sources.iter().all(|source| source.param == 0
                && source.location.path.0.contains(&ir::CarrierStep::Field(0))));
        }
        for mutation in 0..7 {
            let mut forged = body.as_ref().clone();
            match mutation {
                0 => {
                    let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[borrow.0].kind
                    else {
                        panic!("borrow")
                    };
                    let field = place
                        .projection
                        .iter_mut()
                        .find(|step| matches!(step, ir::Projection::Field(_)))
                        .unwrap();
                    *field = ir::Projection::Field(1);
                }
                1 => {
                    let ir::ExprKind::Borrow { kind, .. } = &mut forged.exprs[borrow.0].kind else {
                        panic!("borrow")
                    };
                    *kind = ir::BorrowKind::Mut;
                }
                2 => {
                    let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[borrow.0].kind
                    else {
                        panic!("borrow")
                    };
                    place.local = forged.params[1];
                }
                3 => forged.body.tail = Some(borrow),
                4 => {
                    let ir::ExprKind::Call { args, .. } = &mut forged.exprs[receipt.tail().0].kind
                    else {
                        panic!("call")
                    };
                    args[0] = borrow;
                }
                5 => {
                    let ir::ExprKind::Call { target, .. } =
                        &mut forged.exprs[receipt.tail().0].kind
                    else {
                        panic!("call")
                    };
                    match target {
                        ir::Callee::Direct(call) => call.declaration = checked.ty.provider(),
                        ir::Callee::TraitItem(call) => call.item = checked.ty.provider(),
                        _ => panic!("source call"),
                    }
                }
                6 => {
                    if let Some(node) = witness.nodes.iter().find(|node| {
                        matches!(node.kind, ir::CaptureResultKind::ArrayToSlice { .. })
                    }) {
                        let ir::ExprKind::ArrayToSlice { reference } =
                            &mut forged.exprs[node.expression.0].kind
                        else {
                            panic!("unsize")
                        };
                        *reference = receipt.tail();
                    } else {
                        forged.result = ir::Ty::Unit;
                    }
                }
                _ => unreachable!(),
            }
            assert!(
                validate_source(receipt, &forged, &facts).is_err(),
                "mode={mode:?} mutation={mutation}"
            );
        }
    }
    let facts = world
        .closure_callable_facts_for(checked.ty.clone(), ir::ClosureKind::FnOnce)
        .unwrap();
    assert!(
        validate_source(
            receipt,
            &checked.body_for(ir::ClosureKind::Fn).unwrap(),
            &facts
        )
        .is_err()
    );
    assert!(
        validate_source(
            receipt,
            &checked.body,
            &world.callable_facts(checked.ty.owner()).unwrap()
        )
        .is_err()
    );
    let schema_item = *plan.registry.trait_items.iter().flatten().next().unwrap();
    assert!(
        validate_source(
            receipt,
            &checked.body,
            &world.schema_facts(schema_item).unwrap()
        )
        .is_err()
    );
    let (foreign_plan, foreign_checked, _) = fixture_graph_source(SOURCE);
    let foreign = foreign_plan
        .trait_world()
        .unwrap()
        .closure_callable_facts_for(foreign_checked.ty.clone(), ir::ClosureKind::Fn)
        .unwrap();
    assert!(validate_source(receipt, &checked.body, &foreign).is_err());
    for mutation in 0..5 {
        let mut witness = witness.clone();
        let mut nodes = witness.nodes.to_vec();
        let node = nodes
            .iter_mut()
            .find(|node| matches!(node.kind, ir::CaptureResultKind::Borrow { .. }))
            .unwrap();
        let ir::CaptureResultKind::Borrow { field, source, .. } = &mut node.kind else {
            unreachable!()
        };
        match mutation {
            0 => *field += 1,
            1 => source.kind = ir::CaptureKind::Mut,
            2 => source.place.local = ir::LocalId(source.place.local.0 + 1),
            3 => {
                witness.recipe = foreign_checked
                    .ty
                    .result_receipt()
                    .unwrap()
                    .capture_witness()
                    .unwrap()
                    .recipe
                    .clone()
            }
            4 => {}
            _ => unreachable!(),
        }
        witness.nodes = nodes.into();
        let forged = Arc::new(
            ir::ClosureResultReceipt::captured(
                checked.ty.owner(),
                checked.ty.provider(),
                checked.ty.site() + u32::from(mutation == 4),
                receipt.tail(),
                witness,
                &plan.budget,
            )
            .unwrap(),
        );
        let forged = Arc::new(
            checked
                .ty
                .as_ref()
                .clone()
                .with_result_receipt(Some(forged)),
        );
        assert!(
            world
                .closure_callable_facts_for(forged, ir::ClosureKind::Fn)
                .is_err()
        );
    }
}

#[test]
fn projected_noncopy_collection_builds_with_default_budget() {
    let budget = ir::PlanBudget::default();
    let source = "struct Item{first:String,second:String}fn main(){let items=vec![Item{first:String::from(\"apple\"),second:String::from(\"pear\")}];let first:Vec<&String>=items.iter().map(|i|&i.first).collect();let second:Vec<&String>=items.iter().map(|i|&i.second).collect();println!(\"{} {}\",first[0],second[0]);}";
    crate::compiler::compile_input_with_budget(
        crate::compiler::Input::Text(source.into()),
        1,
        false,
        budget.clone(),
    )
    .unwrap_or_else(|error| {
        panic!(
            "{error}; actual={:?}; first={:?}; usage={:?}",
            error.resource_usage,
            budget.first_resource_failure(),
            budget.usage_snapshot()
        )
    });
}

#[test]
fn projected_argument_result_replays_exact_field_and_borrow_for_all_modes() {
    const SOURCE: &str = "struct Item{first:String,second:String}fn inspect(items:Vec<&Item>){let _names:Vec<&String>=items.iter().map(|i|&i.first).collect();}fn main(){}";
    let (plan, checked, _) = fixture_graph_source(SOURCE);
    let world = plan.trait_world().unwrap();
    let receipt = checked.ty.result_receipt().unwrap();
    assert!(receipt.is_projected_input());
    let input = receipt.argument_origin().unwrap();
    assert_eq!(input.depth, 1);
    for mode in [
        ir::ClosureKind::Fn,
        ir::ClosureKind::FnMut,
        ir::ClosureKind::FnOnce,
    ] {
        let facts = world
            .closure_callable_facts_for(checked.ty.clone(), mode)
            .unwrap();
        let body = checked.body_for(mode).unwrap();
        validate_source(receipt, &body, &facts).unwrap();
        let view = facts.carrier_contract(ir::SignatureSubject::Body).unwrap();
        view.validate_for(&facts).unwrap();
        assert_eq!(
            view.contract().results[0].sources[0].location.channel,
            ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 })
        );
        for mutation in 0..6 {
            let mut forged = body.as_ref().clone();
            match mutation {
                0 => {
                    let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[input.borrow.0].kind
                    else {
                        panic!("borrow")
                    };
                    *place.projection.last_mut().unwrap() = ir::Projection::Field(1);
                }
                1 => {
                    let ir::ExprKind::Borrow { kind, .. } = &mut forged.exprs[input.borrow.0].kind
                    else {
                        panic!("borrow")
                    };
                    *kind = ir::BorrowKind::Mut;
                }
                2 => {
                    let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[input.borrow.0].kind
                    else {
                        panic!("borrow")
                    };
                    place.local = forged.params[0];
                }
                3 => {
                    let ir::TypedStmt::Let { value, .. } = &forged.body.statements[0] else {
                        panic!("entry")
                    };
                    let ir::ExprKind::Field { index, .. } = &mut forged.exprs[value.0].kind else {
                        panic!("field")
                    };
                    *index = 1;
                }
                4 => forged.body.tail = Some(ir::ExprId(receipt.tail().0 + 1)),
                5 => forged.exprs[input.borrow.0].ty = ir::Ty::Unit,
                _ => unreachable!(),
            }
            assert!(
                validate_source(receipt, &forged, &facts).is_err(),
                "mode={mode:?}, mutation={mutation}"
            );
            assert!(
                facts
                    .validate_closure_body_source(&checked.ty, mode, &forged)
                    .is_err()
            );
        }
        let template =
            crate::compiler::cfg::lower_closure_template(body, facts, &checked, mode, &plan.budget)
                .unwrap();
        crate::compiler::ownership::verify_template(template).unwrap();
    }
    for (owner, provider, site) in [
        (
            checked.ty.provider(),
            checked.ty.provider(),
            checked.ty.site(),
        ),
        (checked.ty.owner(), checked.ty.owner(), checked.ty.site()),
        (
            checked.ty.owner(),
            checked.ty.provider(),
            checked.ty.site() + 1,
        ),
    ] {
        let receipt = Arc::new(
            ir::ClosureResultReceipt::projected_input(
                owner,
                provider,
                site,
                receipt.tail(),
                input.argument,
                input.tuple,
                input.borrow,
                input.place.clone(),
                input.depth,
                &plan.budget,
            )
            .unwrap(),
        );
        let forged = Arc::new(
            checked
                .ty
                .as_ref()
                .clone()
                .with_result_receipt(Some(receipt)),
        );
        assert!(
            world
                .closure_callable_facts_for(forged, ir::ClosureKind::Fn)
                .is_err()
        );
    }
    let (foreign_plan, foreign_checked, _) = fixture_graph_source(SOURCE);
    let foreign = foreign_plan
        .trait_world()
        .unwrap()
        .closure_callable_facts_for(foreign_checked.ty.clone(), ir::ClosureKind::Fn)
        .unwrap();
    assert!(
        validate_source(
            receipt,
            &checked.body_for(ir::ClosureKind::Fn).unwrap(),
            &foreign
        )
        .is_err()
    );
    let (other_plan, other_checked, _) = fixture_graph_source(
        "struct Item{first:String,second:String}fn inspect(items:Vec<&Item>){let _names:Vec<&String>=items.iter().map(|i|&i.second).collect();}fn main(){}",
    );
    let other = other_plan
        .trait_world()
        .unwrap()
        .closure_callable_facts_for(other_checked.ty.clone(), ir::ClosureKind::Fn)
        .unwrap();
    assert!(
        validate_source(
            receipt,
            &other_checked.body_for(ir::ClosureKind::Fn).unwrap(),
            &other
        )
        .is_err()
    );
}

#[test]
fn shared_argument_result_replays_exact_tuple_channel_for_all_modes() {
    let (plan, checked, _) = fixture_graph_source(ARGUMENT_STR_SOURCE);
    let world = plan.trait_world().unwrap();
    let receipt = checked.ty.result_receipt().unwrap();
    let input = receipt.argument_origin().unwrap();
    assert_eq!(input.argument, 0);
    assert_eq!(
        input.depth, 1,
        "&&Item must return through inner &Item, not temporary outer reference"
    );
    for mode in [
        ir::ClosureKind::Fn,
        ir::ClosureKind::FnMut,
        ir::ClosureKind::FnOnce,
    ] {
        let facts = world
            .closure_callable_facts_for(checked.ty.clone(), mode)
            .unwrap();
        let body = checked.body_for(mode).unwrap();
        validate_source(receipt, &body, &facts).unwrap();
        let view = facts.carrier_contract(ir::SignatureSubject::Body).unwrap();
        view.validate_for(&facts).unwrap();
        assert_eq!(view.contract().results.len(), 1);
        let sources = &view.contract().results[0].sources;
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].param, 1);
        assert_eq!(
            sources[0].location.path.0.as_ref(),
            &[ir::CarrierStep::Field(0)]
        );
        assert_eq!(
            sources[0].location.channel,
            ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 })
        );
        for mutation in 0..5 {
            let mut forged = body.as_ref().clone();
            match mutation {
                0 => {
                    let ir::ExprKind::Borrow { kind, .. } = &mut forged.exprs[input.borrow.0].kind
                    else {
                        panic!("borrow")
                    };
                    *kind = ir::BorrowKind::Mut;
                }
                1 => {
                    let ir::ExprKind::Borrow { place, .. } = &mut forged.exprs[input.borrow.0].kind
                    else {
                        panic!("borrow")
                    };
                    place.projection.remove(0);
                }
                2 => {
                    let ir::TypedStmt::Let { local, .. } = &mut forged.body.statements[0] else {
                        panic!("entry")
                    };
                    *local = forged.params[0];
                }
                3 => {
                    let ir::ExprKind::Call { args, .. } = &mut forged.exprs[receipt.tail().0].kind
                    else {
                        panic!("call")
                    };
                    args[0] = receipt.tail();
                }
                4 => forged.call_evidence = Arc::from([]),
                _ => unreachable!(),
            }
            assert!(
                validate_source(receipt, &forged, &facts).is_err(),
                "mode={mode:?} mutation={mutation}"
            );
            assert!(
                facts
                    .validate_closure_body_source(&checked.ty, mode, &forged)
                    .is_err()
            );
        }
    }
    let (foreign_plan, foreign_closure, _) = fixture_graph_source(ARGUMENT_STR_SOURCE);
    let foreign = foreign_plan
        .trait_world()
        .unwrap()
        .closure_callable_facts_for(foreign_closure.ty.clone(), ir::ClosureKind::Fn)
        .unwrap();
    assert!(
        validate_source(
            receipt,
            &checked.body_for(ir::ClosureKind::Fn).unwrap(),
            &foreign
        )
        .is_err()
    );
}

fn fixture_graph_source(
    text: &str,
) -> (PlanCtx, super::super::check::CheckedClosure, ir::PlanBudget) {
    use crate::compiler::{
        ClosureBodiesFact, CompilerProvider, Input, Snapshot, TemplateBodyFact, keys,
    };
    use crate::engine::Engine;
    let budget = ir::PlanBudget::default();
    let snapshot = Arc::new(Snapshot {
        input: Input::Text(text.into()),
        source_options: crate::compiler::source::SourceOptions::default(),
        budget: budget.clone(),
    });
    let engine = Engine::new(CompilerProvider(snapshot));
    let signatures = engine.request(keys::SignaturesDebtRoot.key());
    let parent = engine.request(keys::item(crate::compiler::TEMPLATE_BODY, 0));
    engine.run(1).unwrap();
    let plan = signatures.fact::<PlanCtx>(0).unwrap();
    parent.completion().unwrap();
    let template = parent.fact::<TemplateBodyFact>(0).unwrap();
    let closures = parent.fact::<ClosureBodiesFact>(1).unwrap();
    let checked = closures
        .publication
        .as_ref()
        .unwrap()
        .restore_closure(
            &plan,
            template.publication.as_ref().unwrap(),
            closures.bodies.first().unwrap(),
            &budget,
        )
        .unwrap();
    (plan.as_ref().clone(), checked, budget)
}

fn fixture() -> (PlanCtx, super::super::check::CheckedClosure, ir::PlanBudget) {
    fixture_source(
        "fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C){let composed=move |x:A|g(f(x));}fn main(){}",
    )
}

fn fixture_source(text: &str) -> (PlanCtx, super::super::check::CheckedClosure, ir::PlanBudget) {
    use crate::compiler::source;
    let options = source::SourceOptions::default();
    let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "closure-symbolic-result.rs",
            text,
            &options,
        )))
        .unwrap();
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
fn symbolic_closure_output_proves_defining_opaque_return_bound() {
    let (plan, checked, _) = fixture_source(
        "fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C{move |x:A|g(f(x))}fn main(){}",
    );
    assert!(checked.ty.result_receipt().is_some());
    assert!(matches!(checked.ty.result(), ir::Ty::Param(_)));
    assert_eq!(checked.ty.owner_args().len(), 5);
    let world = plan.trait_world().unwrap();
    for mode in [
        ir::ClosureKind::Fn,
        ir::ClosureKind::FnMut,
        ir::ClosureKind::FnOnce,
    ] {
        let facts = world
            .closure_callable_facts_for(checked.ty.clone(), mode)
            .unwrap();
        validate_source(
            checked.ty.result_receipt().unwrap(),
            &checked.body_for(mode).unwrap(),
            &facts,
        )
        .unwrap();
    }
}

#[test]
fn symbolic_closure_output_equality_rejects_foreign_context_and_rhs() {
    let (plan, checked, _) = fixture_source(
        "fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C{move |x:A|g(f(x))}fn main(){}",
    );
    let world = plan.trait_world().unwrap();
    let facts = world.callable_facts(checked.ty.owner()).unwrap();
    let schema = world
        .normalized_callable_schema(checked.ty.owner())
        .unwrap();
    let ir::Ty::Opaque(opaque) = schema.signature.result else {
        panic!("expected opaque result")
    };
    let bound = &opaque.bounds()[0];
    let (item, rhs) = &bound.equalities()[0];
    let projection = ir::AssociatedTypeProjection {
        trait_ref: bound.application(ir::Ty::Closure(checked.ty.clone())),
        item: *item,
    };
    let goal = ir::Predicate::ProjectionEq {
        projection: projection.clone(),
        ty: rhs.clone(),
    };
    assert!(matches!(
        facts.service().prove_goal(&goal, &facts).unwrap(),
        tr::ProofOutcome::Proved(_)
    ));
    let normal = world
        .normalize_associated_type(&projection, &facts)
        .unwrap();
    assert_eq!(normal.ty(), rhs);
    normal.validate_for(&projection, &facts).unwrap();
    assert!(matches!(
        facts
            .service()
            .prove_goal(
                &ir::Predicate::ProjectionEq {
                    projection: projection.clone(),
                    ty: ir::Ty::Bool,
                },
                &facts
            )
            .unwrap(),
        tr::ProofOutcome::NotProven
    ));
    let foreign_owner = world
        .closure_callable_facts_for(checked.ty.clone(), ir::ClosureKind::Fn)
        .unwrap();
    assert!(normal.validate_for(&projection, &foreign_owner).is_err());
    let body = checked.body_for(ir::ClosureKind::Fn).unwrap();
    let ir::ExprKind::Call {
        target: ir::Callee::TraitItem(call),
        ..
    } = &body.exprs[checked.ty.result_receipt().unwrap().tail().0].kind
    else {
        panic!("expected required source call")
    };
    let schema_phase = world.schema_facts(call.item).unwrap();
    assert!(normal.validate_for(&projection, &schema_phase).is_err());
    let (other_plan, other_checked, _) = fixture();
    let other_world = other_plan.trait_world().unwrap();
    assert!(
        normal
            .validate_for(
                &projection,
                &other_world
                    .callable_facts(other_checked.ty.owner())
                    .unwrap()
            )
            .is_err()
    );
    let mut changed = projection.clone();
    changed.item = checked.ty.provider();
    assert!(normal.validate_for(&changed, &facts).is_err());
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
        view.validate_for(&facts).unwrap_or_else(|error| {
            panic!(
                "mode={mode:?}; error={error:?}; firstfailure={:?}; usage={:?}",
                budget.first_resource_failure(),
                budget.usage_snapshot(),
            )
        });
        assert_eq!(view.contract().results.len(), 1);
        let template =
            crate::compiler::cfg::lower_closure_template(source, facts, &checked, mode, &budget)
                .unwrap_or_else(|error| {
                    panic!(
                        "lowering mode={mode:?}; error={error:?}; firstfailure={:?}; usage={:?}",
                        budget.first_resource_failure(),
                        budget.usage_snapshot(),
                    )
                });
        crate::compiler::ownership::verify_template(template).unwrap_or_else(|error| {
            panic!(
                "ownership mode={mode:?}; error={error:?}; firstfailure={:?}; usage={:?}",
                budget.first_resource_failure(),
                budget.usage_snapshot(),
            )
        });
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
