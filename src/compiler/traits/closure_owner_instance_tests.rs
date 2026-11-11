//! Genuine authored closure templates, never fabricated shape seals.
use super::*;
use crate::compiler::{frontend, source};

struct Fixture {
    plan: frontend::PlanCtx,
    template: Arc<ir::ClosureTy>,
    budget: PlanBudget,
    facts: Arc<ValidatedFacts>,
}

fn fixture() -> Fixture {
    let options = source::SourceOptions::default();
    let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler.provide(Arc::new(source::prepare_text_unit(
        "closed-owner-reconstruction.rs",
        "trait Marker{}impl Marker for u64{}fn apply<T:Marker>(x:T)->u64{let f=move||{let _y=x;7u64};f()}fn take<U>(u:U)->U{u}fn main(){}",
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
    let program = frontend::resolve(&frontend::parse_prepared(&prepared).unwrap()).unwrap();
    let budget = PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let (provider, owner) = (0..plan.callable_count())
        .find_map(|index| {
            let provider = plan.callable_decl(index).ok()?;
            plan.stored_closure_owner(provider)
                .ok()
                .flatten()
                .map(|(owner, _)| (provider, owner))
        })
        .expect("fixture has a genuine authored generic closure");
    let constants = plan.prepare_body_constants(owner, &budget).unwrap();
    assert!(constants.direct().is_empty());
    let constants = plan
        .seal_body_constants(constants, Vec::new(), &budget)
        .unwrap();
    let (_, closures) = plan
        .publish_callable_body_with_closures(owner, constants, &budget)
        .unwrap();
    let template = closures
        .iter()
        .find(|closure| closure.ty.provider() == provider)
        .unwrap()
        .ty
        .clone();
    let main = plan.callable_decl(plan.closed_roots().unwrap()[0]).unwrap();
    let facts = plan
        .instance_facts(&CallTarget {
            declaration: main,
            type_args: Arc::from([]),
        })
        .unwrap();
    Fixture {
        plan,
        template,
        budget,
        facts,
    }
}

#[test]
fn checked_body_type_scratch_reuses_only_exact_authorized_closure() {
    let f = fixture();
    let closure = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
        .unwrap();
    let ty = Ty::Closure(closure.clone());
    let mut scratch = TraitScratch::default();
    f.facts.validate_ty_scratch(&ty, &mut scratch).unwrap();
    let nodes = f.budget.usage_snapshot().nodes;
    f.facts.validate_ty_scratch(&ty, &mut scratch).unwrap();
    assert_eq!(f.budget.usage_snapshot().nodes, nodes + 1);

    let forged = Ty::Closure(Arc::new(
        ir::ClosureTy::new(
            closure.owner(),
            closure.owner_args().into(),
            closure.site(),
            closure.provider(),
            closure.params().into(),
            Ty::Bool,
            closure.captures().into(),
            closure.kind(),
        )
        .with_sources(closure.sources().into())
        .with_shape_seal(closure.shape_seal().cloned()),
    ));
    assert!(f.facts.validate_ty_scratch(&forged, &mut scratch).is_err());
    f.facts.validate_ty_scratch(&ty, &mut scratch).unwrap();

    let mut forged_facts = f.facts.as_ref().clone();
    forged_facts.predicates = Arc::new(PredicateEnv {
        world: f.facts.predicates.world,
        index: f.facts.predicates.index,
        declaration: f.facts.predicates.declaration,
        env: f.facts.predicates.env.clone(),
        predicates: f.facts.predicates.predicates.clone(),
    });
    assert!(forged_facts.validate_ty_scratch(&ty, &mut scratch).is_err());
    f.facts.validate_ty_scratch(&ty, &mut scratch).unwrap();
    let foreign = fixture();
    assert!(
        foreign
            .facts
            .validate_ty_scratch(&ty, &mut scratch)
            .is_err()
    );
    f.facts.validate_ty_scratch(&ty, &mut scratch).unwrap();

    let universal = f.facts.world.callable_facts(f.facts.declaration()).unwrap();
    let nodes = f.budget.usage_snapshot().nodes;
    universal.validate_ty_scratch(&ty, &mut scratch).unwrap();
    assert!(
        f.budget.usage_snapshot().nodes > nodes + 1,
        "another phase must revalidate"
    );
    f.facts.validate_ty_scratch(&ty, &mut scratch).unwrap();
    let other_limits = PlanBudget::new(ir::PlanLimits {
        depth: 0,
        ..ir::PlanLimits::default()
    });
    assert!(
        ir::FactsCtx::from_validated(&f.facts)
            .validate_ty_with_budget_scratch(&ty, &other_limits, &mut scratch)
            .is_err()
    );
    f.facts.validate_ty_scratch(&ty, &mut scratch).unwrap();

    let mut too_deep = Ty::Bool;
    for _ in 0..=f.budget.limits().depth {
        too_deep = Ty::Tuple(Arc::from([too_deep]));
    }
    assert_eq!(
        f.facts
            .validate_ty_scratch(&too_deep, &mut scratch)
            .unwrap_err()
            .kind(),
        FactsErrorKind::Resource
    );
    assert_eq!(
        f.facts
            .validate_ty_scratch(&ty, &mut scratch)
            .unwrap_err()
            .kind(),
        FactsErrorKind::Resource
    );
}

#[test]
fn closure_type_walk_reuses_only_the_same_checked_allocation() {
    let f = fixture();
    let closure = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
        .unwrap();
    let ty = Ty::Closure(closure.clone());
    let world = f.plan.trait_world().unwrap();
    let mut walk = TypeWalk::default();
    let height = world
        .validate_ty_inner(&ty, f.facts.env(), 0, TypeUse::Root, &mut walk)
        .unwrap();
    let nodes = f.budget.usage_snapshot().nodes;
    let completed = walk.complete.len();
    assert_eq!(
        world
            .validate_ty_inner(&ty, f.facts.env(), 0, TypeUse::Root, &mut walk)
            .unwrap(),
        height
    );
    assert_eq!(walk.complete.len(), completed);
    assert_eq!(f.budget.usage_snapshot().nodes, nodes + 1);

    // Repeated normalization of an unchanged closure must retain its exact
    // issuer allocation. Otherwise every predicate batch is artificially new,
    // despite its unchanged owner, ordered actuals, seals and current facts.
    let rows = [Predicate::Implements(TraitRef {
        declaration: world.catalog.id(CatalogItem::Sized),
        self_ty: ty.clone(),
        args: Arc::from([]),
    })];
    let mut formations = ProofScratch::default();
    let first = world
        .normalize_instantiated_projection_predicates(
            &rows,
            ProofContext::from_facts(&f.facts),
            &mut formations,
        )
        .unwrap();
    let Ty::Closure(first_closure) = &first[0].trait_ref().self_ty else {
        panic!("closure normalization changed its type kind")
    };
    assert!(Arc::ptr_eq(&closure, first_closure));
    let before = f.budget.usage_snapshot().work;
    world
        .validate_predicates_with_active(
            &first,
            f.facts.env(),
            None,
            &mut formations,
            Some(&f.facts),
        )
        .unwrap();
    let formation_cold = f.budget.usage_snapshot().work - before;
    let second = world
        .normalize_instantiated_projection_predicates(
            &rows,
            ProofContext::from_facts(&f.facts),
            &mut formations,
        )
        .unwrap();
    assert!(world.same_formation_predicates(&first, &second).unwrap());
    let before = f.budget.usage_snapshot().work;
    world
        .validate_predicates_with_active(
            &second,
            f.facts.env(),
            None,
            &mut formations,
            Some(&f.facts),
        )
        .unwrap();
    let formation_warm = f.budget.usage_snapshot().work - before;
    assert!(
        formation_warm < formation_cold,
        "normalization rebuilt immutable closure inputs: cold={formation_cold}, warm={formation_warm}"
    );
    assert_eq!(formations.predicate_formations.len(), 1);

    // Copying the authentic seal onto a different descriptor must not hit
    // the completed allocation's memo entry.
    let forged = Ty::Closure(Arc::new(
        ir::ClosureTy::new(
            closure.owner(),
            closure.owner_args().into(),
            closure.site(),
            closure.provider(),
            closure.params().into(),
            Ty::Bool,
            closure.captures().into(),
            closure.kind(),
        )
        .with_sources(closure.sources().into())
        .with_shape_seal(closure.shape_seal().cloned()),
    ));
    assert!(
        world
            .validate_ty_inner(&forged, f.facts.env(), 0, TypeUse::Root, &mut walk)
            .is_err()
    );
    let forged_rows = [Predicate::Implements(TraitRef {
        self_ty: forged.clone(),
        ..rows[0].trait_ref().clone()
    })];
    let forged_normal = world
        .normalize_instantiated_projection_predicates(
            &forged_rows,
            ProofContext::from_facts(&f.facts),
            &mut formations,
        )
        .unwrap();
    assert!(
        world
            .validate_predicates_with_active(
                &forged_normal,
                f.facts.env(),
                None,
                &mut formations,
                Some(&f.facts),
            )
            .is_err()
    );
    assert_eq!(formations.predicate_formations.len(), 1);
    assert_eq!(walk.complete.len(), completed);

    let mut proofs = ProofScratch::default();
    let bytes = f.budget.usage_snapshot().bytes;
    let closed = world
        .closedness_of_root(&ty, f.facts.env(), TypeUse::Root, 0, &mut proofs)
        .unwrap();
    assert!(!closed.has_parameters && !closed.has_opaque);
    assert_eq!(closed.height, height);
    assert!(
        f.budget.usage_snapshot().bytes > bytes,
        "retained inputs are paid"
    );
    let work = f.budget.usage_snapshot().work;
    let nodes = f.budget.usage_snapshot().nodes;
    let again = world
        .closedness_of_root(&ty, f.facts.env(), TypeUse::Root, 0, &mut proofs)
        .unwrap();
    assert_eq!(again.height, closed.height);
    assert_eq!(f.budget.usage_snapshot().work - work, 1);
    assert_eq!(f.budget.usage_snapshot().nodes - nodes, 1);
    let entries = proofs.closedness.len();
    assert!(
        world
            .closedness_of_root(&forged, f.facts.env(), TypeUse::Root, 0, &mut proofs)
            .is_err()
    );
    assert_eq!(proofs.closedness.len(), entries);
    let foreign = fixture();
    assert!(
        foreign
            .plan
            .trait_world()
            .unwrap()
            .closedness_of_root(&ty, f.facts.env(), TypeUse::Root, 0, &mut proofs)
            .is_err()
    );
    assert_eq!(proofs.closedness.len(), entries);

    let env = world.info(f.template.owner()).unwrap().env;
    let parameter = Ty::Param(env.ordered_params().next().unwrap());
    let open = world
        .closedness_of_root(&parameter, &env, TypeUse::Actual, 0, &mut proofs)
        .unwrap();
    assert!(open.has_parameters && !open.has_opaque);
    let plain = world
        .closedness_of_root(&Ty::Bool, &env, TypeUse::Actual, 0, &mut proofs)
        .unwrap();
    assert!(
        !plain.has_parameters && !plain.has_opaque,
        "flags belong to each root"
    );
    assert!(
        world
            .closedness_of_root(&parameter, f.facts.env(), TypeUse::Actual, 0, &mut proofs)
            .is_err()
    );
    let open_again = world
        .closedness_of_root(&parameter, &env, TypeUse::Actual, 0, &mut proofs)
        .unwrap();
    assert!(open_again.has_parameters);
    world
        .closedness_of_root(&Ty::Str, f.facts.env(), TypeUse::Root, 0, &mut proofs)
        .unwrap();
    // `str` is a well-formed actual for a `?Sized` parameter; an implicitly
    // Sized parameter rejects it through its own Sized row instead.
    world
        .closedness_of_root(&Ty::Str, f.facts.env(), TypeUse::Actual, 0, &mut proofs)
        .unwrap();
    // One operation may alternate between source and closed caller facts.
    // Each context retains its own immutable closure formation, while the raw
    // parent's completed rows and authority remain untouched.
    let universal = world.callable_facts(f.facts.declaration()).unwrap();
    let mut cold = Vec::new();
    for facts in [f.facts.as_ref(), universal.as_ref()] {
        let before = f.budget.usage_snapshot().work;
        assert_eq!(
            world
                .validate_ty_inner_with_facts(
                    &ty,
                    facts.env(),
                    0,
                    TypeUse::Root,
                    &mut walk,
                    Some(facts),
                )
                .unwrap(),
            height,
        );
        cold.push(f.budget.usage_snapshot().work - before);
    }
    assert_eq!(walk.contexts.len(), 2);
    for (facts, cold) in [f.facts.as_ref(), universal.as_ref()].into_iter().zip(cold) {
        let before = f.budget.usage_snapshot().work;
        world
            .validate_ty_inner_with_facts(
                &ty,
                facts.env(),
                0,
                TypeUse::Root,
                &mut walk,
                Some(facts),
            )
            .unwrap();
        let warm = f.budget.usage_snapshot().work - before;
        assert!(
            warm < cold,
            "closure formation context was discarded: cold={cold}, warm={warm}"
        );
        assert!(
            world
                .validate_ty_inner_with_facts(
                    &forged,
                    facts.env(),
                    0,
                    TypeUse::Root,
                    &mut walk,
                    Some(facts),
                )
                .is_err()
        );
    }
    assert_eq!(walk.contexts.len(), 2);
    assert_eq!(walk.complete.len(), completed);
    assert!(
        world
            .validate_ty_inner_with_facts(
                &ty,
                foreign.facts.env(),
                0,
                TypeUse::Root,
                &mut walk,
                Some(&foreign.facts),
            )
            .is_err()
    );
    // A hit still checks the complete subtree's depth at its new position.
    assert!(
        world
            .validate_ty_inner(
                &ty,
                f.facts.env(),
                f.budget.limits().depth,
                TypeUse::Root,
                &mut walk,
            )
            .is_err()
    );
    assert!(
        world
            .validate_ty_inner_with_facts(
                &ty,
                f.facts.env(),
                0,
                TypeUse::Root,
                &mut walk,
                Some(&f.facts),
            )
            .unwrap_err()
            .is_resource()
    );
}

#[test]
fn closedness_memo_retains_subtree_depth_on_hits() {
    let f = fixture();
    let world = f.plan.trait_world().unwrap();
    let ty = Ty::Array {
        element: Arc::new(Ty::Bool),
        len: 1,
    };
    let mut scratch = ProofScratch::default();
    let result = world
        .closedness_of_root(&ty, f.facts.env(), TypeUse::Root, 0, &mut scratch)
        .unwrap();
    assert_eq!(result.height, 1);
    assert!(
        world
            .closedness_of_root(
                &ty,
                f.facts.env(),
                TypeUse::Root,
                f.budget.limits().depth,
                &mut scratch
            )
            .is_err()
    );
}

#[test]
fn checked_owner_reconstruction_preserves_seal_and_discharges_real_bounds() {
    let f = fixture();
    let concrete = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
        .unwrap();
    assert_eq!(concrete.owner_args(), &[Ty::Int(ir::IntTy::U64)]);
    assert!(Arc::ptr_eq(
        f.template.shape_seal().unwrap(),
        concrete.shape_seal().unwrap()
    ));
    assert_eq!(concrete.owner(), f.template.owner());
    assert_eq!(concrete.provider(), f.template.provider());
    assert_eq!(concrete.site(), f.template.site());
    let error = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Bool])
        .unwrap_err();
    assert_eq!(
        error.kind,
        FactsErrorKind::Unproved,
        "wrong owner bound: {error:?}"
    );
}

#[test]
fn checked_owner_reconstruction_rejects_bad_arity_phase_and_nonoriginal_template() {
    let f = fixture();
    for arguments in [
        vec![],
        vec![Ty::Int(ir::IntTy::U64), Ty::Bool],
        vec![f.template.owner_args()[0].clone()],
    ] {
        assert!(
            f.facts
                .instantiate_closure_owner(&f.template, &arguments)
                .is_err(),
            "invalid actuals admitted: {arguments:?}"
        );
    }
    let concrete = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
        .unwrap();
    assert!(
        f.facts
            .instantiate_closure_owner(&concrete, &[Ty::Int(ir::IntTy::U64)])
            .is_err()
    );
    let universal = f.plan.callable_facts(f.template.owner()).unwrap();
    assert!(
        universal
            .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
            .is_err()
    );
}

#[test]
fn checked_owner_reconstruction_rejects_foreign_recipe_and_changed_sealed_shape() {
    let f = fixture();
    let foreign = fixture();
    // Independently prepared source worlds allocate distinct snapshots. Keep
    // their authored providers' ordinal correspondence, then explicitly copy
    // the foreign seal onto the local descriptor so numeric identity cannot
    // be the reason the second negative rejects.
    assert_eq!(
        f.template.provider().index,
        foreign.template.provider().index
    );
    assert_ne!(
        f.template.provider().snapshot,
        foreign.template.provider().snapshot
    );
    assert!(
        f.facts
            .instantiate_closure_owner(&foreign.template, &[Ty::Int(ir::IntTy::U64)])
            .is_err()
    );
    let transplanted = Arc::new(
        f.template
            .as_ref()
            .clone()
            .with_shape_seal(foreign.template.shape_seal().cloned()),
    );
    assert_eq!(transplanted.provider(), f.template.provider());
    assert!(
        f.facts
            .instantiate_closure_owner(&transplanted, &[Ty::Int(ir::IntTy::U64)])
            .is_err(),
        "foreign recipe seal admitted with exact local declaration IDs"
    );
    let changed = Arc::new(
        f.template
            .try_map::<String>(|ty| {
                Ok(if ty == &Ty::Int(ir::IntTy::U64) {
                    Ty::Bool
                } else {
                    ty.clone()
                })
            })
            .unwrap(),
    );
    assert!(
        f.facts
            .instantiate_closure_owner(&changed, &[Ty::Int(ir::IntTy::U64)])
            .is_err()
    );
}

#[test]
fn closure_actual_graph_key_roundtrips_through_template_replay() {
    use crate::compiler::instances::graph_key;
    let f = fixture();
    let budget = &f.budget;
    let concrete = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
        .unwrap();
    let take = (0..f.plan.callable_count())
        .map(|i| f.plan.callable_decl(i).unwrap())
        .find(|id| {
            matches!(
                f.plan.persistent_decl_path(*id).unwrap().item(),
                frontend::PersistentDeclItem::Name("take")
            )
        })
        .unwrap();
    let target = CallTarget {
        declaration: take,
        type_args: Arc::from([Ty::Closure(concrete.clone())]),
    };
    let words = graph_key::encode(&target, budget).unwrap();
    let missing = std::cell::RefCell::new(Vec::new());
    let replay = |templates: &graph_key::ClosureTemplates| {
        graph_key::decode_with_closures(&words, &f.plan, budget, &|p, a| {
            graph_key::replay_closure(&f.plan, templates, &missing, budget, p, a)
        })
    };
    // Without the provider template the decoder names exactly what to request.
    assert!(replay(&graph_key::ClosureTemplates::new()).is_err());
    assert_eq!(missing.borrow().as_slice(), &[f.template.provider()]);
    let mut templates = graph_key::ClosureTemplates::new();
    templates.insert(f.template.provider(), f.template.clone());
    assert_eq!(replay(&templates).unwrap(), target);
    // A forged (already substituted) template under the provider is rejected.
    let mut forged = graph_key::ClosureTemplates::new();
    forged.insert(f.template.provider(), concrete);
    assert!(replay(&forged).is_err());
    // The plain decoder still refuses closure actuals without graph replay.
    assert!(graph_key::decode(&words, &f.plan, budget).is_err());
}

#[test]
fn closed_owner_typed_reads_reject_foreign_engine_world_order_and_poison() {
    use crate::compiler::nominal_realization::{RealizationKey, read_manifest};
    use crate::compiler::{CompilerProvider, Input, SIGNATURES, Snapshot, TemplateBodyFact};
    use crate::engine::{Context, Engine, EngineAuthority, Key, Provider, TaskFuture};

    struct TypedOwnerProvider {
        snapshot: Arc<Snapshot>,
        signatures: Option<frontend::PlanCtx>,
    }
    struct IssuedOwner(frontend::PlanCtx, Arc<ir::ClosureTy>, Ty, EngineAuthority);
    impl crate::engine::Fact for IssuedOwner {}
    impl Provider for TypedOwnerProvider {
        fn outputs(&self, key: &Key) -> usize {
            if key.kind == 997 {
                1
            } else {
                CompilerProvider(self.snapshot.clone()).outputs(key)
            }
        }
        fn start(&self, cx: Context, key: Key) -> TaskFuture {
            if key.kind == SIGNATURES
                && let Some(plan) = self.signatures.clone()
            {
                return Box::pin(async move { cx.publish_fact(0, plan) });
            }
            if key.kind != 997 {
                return CompilerProvider(self.snapshot.clone()).start(cx, key);
            }
            Box::pin(async move {
                let handle = cx
                    .request(crate::compiler::keys::SignaturesDebtRoot.key())
                    .get(0);
                let plan: Arc<frontend::PlanCtx> = cx.read_fact(handle).await?;
                let provider = (0..plan.callable_count())
                    .map(|index| plan.callable_decl(index).unwrap())
                    .find(|provider| plan.stored_closure_owner(*provider).unwrap().is_some())
                    .unwrap();
                let anchor = (0..plan.callable_count())
                    .map(|index| plan.callable_decl(index).unwrap())
                    .find(|declaration| {
                        matches!(
                            plan.persistent_decl_path(*declaration).unwrap().item(),
                            frontend::PersistentDeclItem::Name("anchor")
                        )
                    })
                    .unwrap();
                let world = plan.trait_world().unwrap();
                let source = world
                    .schema
                    .declaration_schema(anchor)
                    .unwrap()
                    .signature
                    .result;
                let Ty::Struct(source) = source else {
                    panic!("authored anchor result must be nominal")
                };
                let nominal = RealizationKey::encode(
                    world.nominal_realization_world(),
                    source.def().source_decl,
                    &[],
                    32,
                )?;
                let template_handle = cx
                    .request(crate::compiler::keys::item(
                        crate::compiler::TEMPLATE_BODY,
                        plan.callable_index(provider).unwrap(),
                    ))
                    .get(0);
                let mut reads = read_manifest(&cx, &[nominal], world.budget()).await?;
                let template: Arc<TemplateBodyFact> = cx.read_fact(template_handle).await?;
                let closure = template.closure.as_ref().unwrap().ty.clone();
                cx.publish_fact(
                    0,
                    IssuedOwner(
                        plan.as_ref().clone(),
                        closure,
                        reads.remove(0).definition()?,
                        cx.authority(),
                    ),
                )
            })
        }
    }
    for workers in [1, 8] {
        let budget = PlanBudget::default();
        let snapshot = Arc::new(Snapshot {
            input: Input::Text("struct Payload{value:u64}fn anchor()->Payload{Payload{value:1u64}}fn apply<T,U>(x:T,y:U)->u64{let f=move||{let _x=x;let _y=y;7u64};f()}fn main(){}".into()),
            source_options: source::SourceOptions::default(),
            budget: budget.clone(),
        });
        let engine = Engine::new(TypedOwnerProvider {
            snapshot: snapshot.clone(),
            signatures: None,
        });
        let request = engine.request(crate::compiler::keys::test_key(997, vec![]));
        engine.run(workers).unwrap();
        request.completion().unwrap();
        let issued = request.fact::<IssuedOwner>(0).unwrap();
        let IssuedOwner(plan, template, graph, authority) = issued.as_ref();
        let arguments = [graph.clone(), Ty::Int(ir::IntTy::U64)];
        let facts = plan
            .instance_facts(&CallTarget {
                declaration: template.owner(),
                type_args: Arc::from(arguments.clone()),
            })
            .unwrap();
        let mut context = plan.clone();
        let actual = facts
            .instantiate_closure_owner_with_reads(template, &arguments, &mut context, *authority)
            .unwrap();
        assert_eq!(actual.owner_args(), &arguments);
        assert!(Arc::ptr_eq(
            actual.shape_seal().unwrap(),
            template.shape_seal().unwrap()
        ));
        assert!(
            facts
                .instantiate_closure_owner_with_reads(
                    template,
                    &arguments[..1],
                    &mut context,
                    *authority
                )
                .is_err()
        );
        let original_parameters = template.owner_args();
        let changed = Arc::new(
            template
                .try_map::<String>(|ty| {
                    Ok(if ty == &original_parameters[0] {
                        original_parameters[1].clone()
                    } else if ty == &original_parameters[1] {
                        original_parameters[0].clone()
                    } else {
                        ty.clone()
                    })
                })
                .unwrap(),
        );
        assert!(
            facts
                .instantiate_closure_owner_with_reads(
                    &changed,
                    &arguments,
                    &mut context,
                    *authority
                )
                .is_err()
        );
        let foreign = Engine::new(TypedOwnerProvider {
            snapshot,
            signatures: Some(plan.clone()),
        });
        let foreign_request = foreign.request(crate::compiler::keys::test_key(997, vec![]));
        foreign.run(workers).unwrap();
        foreign_request.completion().unwrap();
        let foreign_issued = foreign_request.fact::<IssuedOwner>(0).unwrap();
        assert_ne!(*authority, foreign_issued.3);
        assert!(
            facts
                .instantiate_closure_owner_with_reads(
                    template,
                    &arguments,
                    &mut context,
                    foreign_issued.3
                )
                .is_err()
        );
        let foreign_arguments = [foreign_issued.2.clone(), arguments[1].clone()];
        assert!(
            facts
                .instantiate_closure_owner_with_reads(
                    template,
                    &foreign_arguments,
                    &mut context,
                    *authority
                )
                .is_err()
        );
        let foreign_actual = facts
            .instantiate_closure_owner_with_reads(
                template,
                &foreign_arguments,
                &mut context,
                foreign_issued.3,
            )
            .unwrap();
        for (local, foreign) in [
            (
                Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(graph.clone()),
                },
                Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(foreign_issued.2.clone()),
                },
            ),
            (Ty::Closure(actual.clone()), Ty::Closure(foreign_actual)),
        ] {
            let local_arguments = [local, arguments[1].clone()];
            let foreign_arguments = [foreign, arguments[1].clone()];
            facts
                .instantiate_closure_owner_with_reads(
                    template,
                    &local_arguments,
                    &mut context,
                    *authority,
                )
                .unwrap();
            assert!(
                facts
                    .instantiate_closure_owner_with_reads(
                        template,
                        &local_arguments,
                        &mut context,
                        foreign_issued.3
                    )
                    .is_err()
            );
            assert!(
                facts
                    .instantiate_closure_owner_with_reads(
                        template,
                        &foreign_arguments,
                        &mut context,
                        *authority
                    )
                    .is_err()
            );
        }
        let foreign_options = source::SourceOptions {
            prelude: false,
            ..source::SourceOptions::default()
        };
        let mut foreign_assembler = source::SourceAssembler::new(&foreign_options);
        let foreign_prepared = match foreign_assembler
            .provide(Arc::new(source::prepare_text_unit(
                "foreign-closed-owner-world.rs",
                "fn apply<T,U>(x:T,y:U)->u64{let _f=move||{let _x=x;let _y=y;7u64};7u64}fn main(){}",
                &foreign_options,
            )))
            .unwrap()
        {
            source::AssemblyStep::Done(prepared) => prepared,
            source::AssemblyStep::Need(_) => panic!("minimal foreign world needs no modules"),
        };
        let foreign_program =
            frontend::resolve(&frontend::parse_prepared(&foreign_prepared).unwrap()).unwrap();
        let foreign_budget = PlanBudget::default();
        let foreign_plan =
            frontend::PlanCtx::from_program(&foreign_program, &foreign_budget).unwrap();
        let foreign_provider = (0..foreign_plan.callable_count())
            .map(|index| foreign_plan.callable_decl(index).unwrap())
            .find(|provider| {
                foreign_plan
                    .stored_closure_owner(*provider)
                    .unwrap()
                    .is_some()
            })
            .unwrap();
        let foreign_owner = foreign_plan
            .stored_closure_owner(foreign_provider)
            .unwrap()
            .unwrap()
            .0;
        let foreign_constants = foreign_plan
            .prepare_body_constants(foreign_owner, &foreign_budget)
            .unwrap();
        let foreign_constants = foreign_plan
            .seal_body_constants(foreign_constants, Vec::new(), &foreign_budget)
            .unwrap();
        let (_, foreign_closures) = foreign_plan
            .publish_callable_body_with_closures(foreign_owner, foreign_constants, &foreign_budget)
            .unwrap();
        assert!(
            foreign_closures
                .iter()
                .any(|closure| closure.ty.provider() == foreign_provider)
        );
        let foreign_main = foreign_plan
            .callable_decl(foreign_plan.closed_roots().unwrap()[0])
            .unwrap();
        foreign_plan
            .instance_facts(&CallTarget {
                declaration: foreign_main,
                type_args: Arc::from([]),
            })
            .unwrap();
        assert!(
            facts
                .instantiate_closure_owner_with_reads(
                    template,
                    &arguments,
                    &mut foreign_plan.clone(),
                    *authority
                )
                .is_err()
        );
        facts
            .instantiate_closure_owner_with_reads(template, &arguments, &mut context, *authority)
            .unwrap();
        assert!(budget.charge_work(usize::MAX).is_err());
        assert!(
            facts
                .instantiate_closure_owner_with_reads(
                    template,
                    &arguments,
                    &mut context,
                    *authority
                )
                .is_err()
        );
    }
}
