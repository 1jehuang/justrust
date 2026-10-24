//! Raw membership must not authorize a structurally forged closure shape.
use super::*;

#[test]
fn generic_owner_membership_replays_ordered_substitution() {
    use crate::compiler::source;
    use ir::TraitCx;
    let options = source::SourceOptions::default();
    let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "generic-closure-membership.rs",
            "fn outer<A,B>(a:A,b:B){let f=move||{let first=a;let second=b;7i32};}fn main(){}",
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
    let (_, bodies) =
        type_check::type_callable_source_and_closures(&program, owner, &plan, &budget, None)
            .unwrap();
    let template = &bodies
        .iter()
        .find(|body| body.ty.provider() == provider)
        .unwrap()
        .ty;
    assert_eq!(template.owner_args().len(), 2);
    let concrete = Arc::new(
        template
            .try_map::<String>(|ty| {
                Ok(if ty == &template.owner_args()[0] {
                    ir::Ty::Bool
                } else if ty == &template.owner_args()[1] {
                    ir::Ty::Unit
                } else {
                    ty.clone()
                })
            })
            .unwrap(),
    );
    let world = plan.trait_world().unwrap();
    let declaration = plan
        .registry
        .trait_ids
        .iter()
        .copied()
        .find(|id| world.closure_trait_kind(*id).unwrap() == Some(ir::ClosureKind::FnOnce))
        .unwrap();
    let facts = plan.callable_facts(owner).unwrap();
    let goal = |closure: Arc<ir::ClosureTy>| {
        ir::Predicate::Implements(ir::TraitRef {
            declaration,
            args: Arc::from([if closure.params().is_empty() {
                ir::Ty::Unit
            } else {
                ir::Ty::Tuple(closure.params().into())
            }]),
            self_ty: ir::Ty::Closure(closure),
        })
    };
    assert!(matches!(
        world.prove_goal(&goal(concrete.clone()), &facts).unwrap(),
        crate::compiler::traits::ProofOutcome::Proved(_)
    ));
    let changed = |owner_args: Arc<[ir::Ty]>, result: ir::Ty| {
        Arc::new(
            ir::ClosureTy::new(
                owner,
                owner_args,
                concrete.site(),
                provider,
                concrete.params().into(),
                result,
                concrete.captures().into(),
                concrete.kind(),
            )
            .with_sources(concrete.sources().into())
            .with_shape_seal(concrete.shape_seal().cloned()),
        )
    };
    for (label, forged) in [
        (
            "reordered owner prefix",
            changed(
                Arc::from([ir::Ty::Unit, ir::Ty::Bool]),
                concrete.result().clone(),
            ),
        ),
        (
            "truncated owner prefix",
            changed(Arc::from([ir::Ty::Bool]), concrete.result().clone()),
        ),
        (
            "copied seal changed result",
            changed(concrete.owner_args().into(), ir::Ty::Bool),
        ),
        (
            "callback remapped shape",
            Arc::new(concrete.try_map::<String>(|_| Ok(ir::Ty::Bool)).unwrap()),
        ),
    ] {
        assert!(
            !matches!(
                world.prove_goal(&goal(forged), &facts),
                Ok(crate::compiler::traits::ProofOutcome::Proved(_))
            ),
            "{label} admitted"
        );
    }
}

#[test]
fn raw_fn_membership_rejects_forged_final_shape() {
    use crate::compiler::source;
    use ir::TraitCx;
    let options = source::SourceOptions::default();
    let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "closure-membership.rs",
            "fn main(){let f=|x:i32|x+1;}",
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
    let (_, bodies) =
        type_check::type_callable_source_and_closures(&program, owner, &plan, &budget, None)
            .unwrap();
    let original = bodies
        .iter()
        .find(|body| body.ty.provider() == provider)
        .unwrap();
    let world = plan.trait_world().unwrap();
    let declaration = plan
        .registry
        .trait_ids
        .iter()
        .copied()
        .find(|id| world.closure_trait_kind(*id).unwrap() == Some(ir::ClosureKind::Fn))
        .unwrap();
    let facts = plan.callable_facts(owner).unwrap();
    let goal = |closure: Arc<ir::ClosureTy>| {
        ir::Predicate::Implements(ir::TraitRef {
            declaration,
            args: Arc::from([if closure.params().is_empty() {
                ir::Ty::Unit
            } else {
                ir::Ty::Tuple(closure.params().into())
            }]),
            self_ty: ir::Ty::Closure(closure),
        })
    };
    assert!(matches!(
        world
            .prove_goal(&goal(original.ty.clone()), &facts)
            .unwrap(),
        crate::compiler::traits::ProofOutcome::Proved(_)
    ));
    let forged = Arc::new(ir::ClosureTy::new(
        owner,
        Arc::from([]),
        original.ty.site(),
        provider,
        Arc::from([ir::Ty::Bool]),
        ir::Ty::Bool,
        Arc::from([]),
        ir::ClosureKind::Fn,
    ));
    let copied = Arc::new(
        (*forged)
            .clone()
            .with_shape_seal(original.ty.shape_seal().cloned()),
    );
    let mapped = Arc::new(original.ty.try_map::<String>(|_| Ok(ir::Ty::Bool)).unwrap());
    let changed_site = Arc::new(
        ir::ClosureTy::new(
            owner,
            Arc::from([]),
            original.ty.site() + 1,
            provider,
            original.ty.params().into(),
            original.ty.result().clone(),
            Arc::from([]),
            ir::ClosureKind::Fn,
        )
        .with_shape_seal(original.ty.shape_seal().cloned()),
    );
    let changed_sources =
        Arc::new(
            (*original.ty)
                .clone()
                .with_sources(Arc::from([ir::CaptureSource {
                    place: ir::Place {
                        local: ir::LocalId(999),
                        projection: Vec::new(),
                    },
                    kind: ir::CaptureKind::Value,
                }])),
        );
    for (label, closure) in [
        ("raw", forged),
        ("copied seal", copied),
        ("callback mapped", mapped),
        ("changed site", changed_site),
        ("changed capture source", changed_sources),
    ] {
        let outcome = world.prove_goal(&goal(closure), &facts);
        assert!(
            !matches!(
                outcome,
                Ok(crate::compiler::traits::ProofOutcome::Proved(_))
            ),
            "{label} forged closure membership was admitted"
        );
    }
    // Keep every numeric metadata ID identical while changing only the
    // unforgeable expected source recipe capability.
    assert!(
        !original
            .ty
            .shape_seal()
            .unwrap()
            .validates(
                &ir::ClosureRecipeId::new(),
                &original.ty,
                |left, right| Ok::<_, String>(left == right),
            )
            .unwrap()
    );
    // A real independent world allocates a new snapshot. Remap declaration
    // handles to its genuine registered IDs, but transplant the old seal.
    // Each independent world owns its own unchanged per-task budget.
    let foreign_budget = ir::PlanBudget::default();
    let foreign = PlanCtx::from_program(&program, &foreign_budget).unwrap();
    let foreign_world = foreign.trait_world().unwrap();
    let foreign_owner = foreign
        .callable_decl(plan.callable_index(owner).unwrap())
        .unwrap();
    let foreign_provider = foreign
        .callable_decl(plan.callable_index(provider).unwrap())
        .unwrap();
    let foreign_declaration = foreign
        .registry
        .trait_ids
        .iter()
        .copied()
        .find(|id| foreign_world.closure_trait_kind(*id).unwrap() == Some(ir::ClosureKind::Fn))
        .unwrap();
    let foreign_facts = foreign.callable_facts(foreign_owner).unwrap();
    let transplanted = Arc::new(
        ir::ClosureTy::new(
            foreign_owner,
            Arc::from([]),
            original.ty.site(),
            foreign_provider,
            original.ty.params().into(),
            original.ty.result().clone(),
            original.ty.captures().into(),
            original.ty.kind(),
        )
        .with_sources(original.ty.sources().into())
        .with_shape_seal(original.ty.shape_seal().cloned()),
    );
    let foreign_goal = ir::Predicate::Implements(ir::TraitRef {
        declaration: foreign_declaration,
        args: Arc::from([ir::Ty::Tuple(transplanted.params().into())]),
        self_ty: ir::Ty::Closure(transplanted),
    });
    let Err(error) = foreign_world.prove_goal(&foreign_goal, &foreign_facts) else {
        panic!("foreign source recipe admitted a transplanted closure shape");
    };
    assert!(error.to_string().contains("source recipe or final shape"));
}

#[test]
fn finalized_parent_correspondence_rejects_republication_and_changed_payloads() {
    assert_parent_correspondence("fn main(){let n=7i32;let f=move |x:i32|x+n+1;}", false);
}

#[test]
fn nested_parent_correspondence_rejects_republication_and_changed_payloads() {
    assert_parent_correspondence("fn main(){let f=|x:i32|{let g=|y:i32|y+1;x+1};}", true);
}

fn assert_parent_correspondence(source: &str, nested: bool) {
    use crate::compiler::frontend::PublishedCallableBody;
    let program = parse(source).unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    let owner = (0..plan.callable_count())
        .find_map(|index| {
            let provider = plan.callable_decl(index).ok()?;
            plan.stored_closure_owner(provider)
                .ok()
                .flatten()
                .and_then(|_| plan.closure_publication_owner(provider, &budget).ok())
        })
        .unwrap();
    let publish = || {
        let prepared = plan.prepare_body_constants(owner, &budget).unwrap();
        assert!(prepared.direct().is_empty());
        let constants = plan
            .seal_body_constants(prepared, Vec::new(), &budget)
            .unwrap();
        let (publication, closures) = plan
            .publish_callable_body_with_closures(owner, constants, &budget)
            .unwrap();
        let PublishedCallableBody::Source(publication) = publication else {
            panic!("expected authored source")
        };
        (publication, closures)
    };
    let (publication, closures) = publish();
    let proof = crate::compiler::frontend::closures::publication::ClosurePublicationProof::capture(
        &plan,
        &publication,
        &closures,
        &budget,
    )
    .unwrap();
    let checked = closures
        .iter()
        .find(|checked| {
            plan.stored_closure_owner(checked.ty.owner())
                .unwrap()
                .is_some()
                == nested
        })
        .expect("closure with requested lexical nesting");
    assert_eq!(
        plan.closure_publication_owner(checked.ty.provider(), &budget)
            .unwrap(),
        owner
    );
    proof
        .validate(&plan, &publication, checked, &budget)
        .unwrap();
    let published =
        crate::compiler::frontend::closures::publication::PublishedClosure::capture(&plan, checked)
            .unwrap();
    let restored = proof
        .restore_closure(&plan, &publication, &published, &budget)
        .unwrap();
    assert!(!Arc::ptr_eq(&restored.body, &checked.body));
    proof
        .validate(&plan, &publication, &restored, &budget)
        .unwrap();
    let (republished, _) = publish();
    assert!(
        proof
            .restore_closure(&plan, &republished, &published, &budget)
            .is_err()
    );
    assert!(
        crate::compiler::frontend::closures::publication::ClosurePublicationProof::capture(
            &plan,
            &republished,
            &closures,
            &budget,
        )
        .is_err()
    );
    assert!(
        proof
            .validate(&plan, &republished, checked, &budget)
            .is_err()
    );

    let mut changed = checked.clone();
    Arc::make_mut(&mut changed.body).result = ir::Ty::Bool;
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );
    assert!(
        crate::compiler::frontend::closures::publication::PublishedClosure::capture(
            &plan, &changed,
        )
        .is_err()
    );
    let mut changed = checked.clone();
    Arc::make_mut(&mut changed.body).name.push_str(" forged");
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );
    let mut changed = checked.clone();
    let body = Arc::make_mut(&mut changed.body);
    body.locals[0].ty = ir::Ty::Bool;
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );
    let mut changed = checked.clone();
    let body = Arc::make_mut(&mut changed.body);
    let expression = body
        .exprs
        .iter_mut()
        .find(|expr| matches!(expr.kind, ir::ExprKind::Int(_)))
        .unwrap();
    expression.kind = ir::ExprKind::Int(ir::IntValue::i32(123));
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );
    let mut changed = checked.clone();
    Arc::make_mut(&mut changed.body).exprs[0].ty = ir::Ty::Bool;
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );
    let mut changed = checked.clone();
    Arc::make_mut(&mut changed.body).runtime_sites = ir::RuntimeSites::from_source(vec![(
        ir::ExprId(0),
        Arc::new(ir::RuntimeLocation {
            path: Arc::from("foreign-source.rs"),
            line: 1,
            column: 1,
        }),
    )])
    .unwrap();
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );
    let mut changed = checked.clone();
    let mut modes = changed.modes.to_vec();
    let alternate = modes
        .iter_mut()
        .find(|mode| mode.mode != checked.ty.kind())
        .unwrap();
    Arc::make_mut(&mut alternate.body)
        .name
        .push_str(" alternate forged");
    changed.modes = modes.into();
    assert!(
        proof
            .validate(&plan, &publication, &changed, &budget)
            .is_err()
    );
}
