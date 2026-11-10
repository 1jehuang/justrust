use super::*;

#[derive(Clone)]
struct Provider {
    plan: Arc<super::super::super::frontend::PlanCtx>,
    roots: Vec<RealizationKey>,
    forge: fn(&mut ComponentFact),
}
impl crate::engine::Provider for Provider {
    fn outputs(&self, _: &Key) -> usize {
        1
    }
    fn start(&self, cx: Context, key: Key) -> crate::engine::TaskFuture {
        let this = self.clone();
        Box::pin(async move {
            if key == crate::compiler::keys::SignaturesDebtRoot.key() {
                return cx.publish_fact(0, this.plan.as_ref().clone());
            }
            if key.kind == super::super::super::NOMINAL_DECL_WELL_FORMED {
                let declaration = ir::DeclId {
                    snapshot: ir::SnapshotId::from_graph_word(key.params[0]),
                    index: key.params[1],
                };
                return cx.publish_fact(
                    0,
                    this.plan
                        .publish_nominal_declaration(declaration)
                        .map_err(Error::new)?,
                );
            }
            if key.kind == super::super::KIND {
                if key.params.get(1) == Some(&super::super::component_key::DOMAIN) {
                    let mut fact = produce(&cx, key).await?;
                    (this.forge)(&mut fact);
                    return cx.publish_fact(0, fact);
                }
                return super::super::run(&cx, key).await;
            }
            if key.kind == 996 {
                let world = this.plan.trait_world().map_err(Error::new)?;
                let reads = super::super::read_manifest(&cx, &this.roots, world.budget()).await?;
                return cx.publish_fact(0, reads);
            }
            Err(Error::new("unexpected component test key"))
        })
    }
}

fn provider(source: &str, root_indices: &[usize], actuals: &[Term]) -> Provider {
    provider_with_modules(source, root_indices, actuals).0
}

fn provider_with_modules(
    source: &str,
    root_indices: &[usize],
    actuals: &[Term],
) -> (Provider, Arc<crate::compiler::module_identity::ModuleArena>) {
    let program = super::super::super::frontend::parse(source).unwrap();
    let modules = program.modules.clone();
    let plan = Arc::new(
        super::super::super::frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default())
            .unwrap(),
    );
    let world = plan.trait_world().unwrap();
    let roots = root_indices
        .iter()
        .map(|&index| {
            producer::key(
                world.nominal_realization_world(),
                plan.nominal_declaration(index).unwrap(),
                actuals,
                world.budget(),
            )
            .unwrap()
        })
        .collect();
    (
        Provider {
            plan,
            roots,
            forge: |_| {},
        },
        modules,
    )
}
fn test_root() -> Key {
    crate::compiler::keys::params(996, 0, vec![0])
}

#[test]
fn acyclic_function_item_container_keeps_recursive_child_in_its_own_component() {
    for workers in [1, 8] {
        let mut own = provider(
            "struct Node{next:*mut Node}struct Wrapper<T>{value:T}fn identity(){}fn main(){}",
            &[],
            &[],
        );
        let world = own.plan.trait_world().unwrap();
        let node = own.plan.nominal_declaration(0).unwrap();
        let wrapper = own.plan.nominal_declaration(1).unwrap();
        let function = own.plan.function_decl(0).unwrap();
        let actual = Term::Tuple(vec![
            Term::Nominal(node, vec![]),
            Term::FnItem(function, vec![]),
        ]);
        own.roots = vec![
            producer::key(
                world.nominal_realization_world(),
                wrapper,
                &[actual],
                world.budget(),
            )
            .unwrap(),
            producer::key(world.nominal_realization_world(), node, &[], world.budget()).unwrap(),
        ];
        let engine = crate::engine::Engine::new(own);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        assert!(
            !reads[0].is_component(),
            "acyclic root was forced into its child's SCC"
        );
        assert!(
            reads[1].is_component(),
            "recursive child lost its typed component"
        );
        for read in reads.iter() {
            let ty = read.definition().unwrap();
            read.replay(&ty, &world).unwrap();
        }
    }
}

#[test]
fn nominal_replay_sessions_preserve_completed_external_authority() {
    for workers in [1, 8] {
        let source = "struct Node{next:*mut Node,extra:Extra}struct Extra{n:i32}fn main(){}";
        let own = provider(source, &[0], &[]);
        let plan = own.plan.clone();
        let world = own.plan.trait_world().unwrap();
        let component_key = ComponentKey::encode(
            world.nominal_realization_world(),
            &own.roots,
            world.budget(),
        )
        .unwrap();
        let foreign = provider(source, &[0], &[]);
        let foreign_world = foreign.plan.trait_world().unwrap();
        let engine = crate::engine::Engine::new(own);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        let caller = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let roots = [
            ir::Ty::Tuple(Arc::from([root.clone(), ir::Ty::Bool])),
            ir::Ty::Tuple(Arc::from([root.clone(), ir::Ty::Char])),
        ];
        assert!(
            world
                .component_type_walk_replays_for_tests(&roots, &caller)
                .unwrap()
                >= 2
        );
        let mut replays = traits::NominalReplays::default();
        let before = world.budget().usage_snapshot().work;
        reads[0].replay_with(&root, &world, &mut replays).unwrap();
        let cold = world.budget().usage_snapshot().work - before;
        let completed = replays.replay_counts();
        assert!(
            completed.0 >= 2,
            "external completed facts were lost: {completed:?}"
        );
        let before = world.budget().usage_snapshot().work;
        for _ in 0..32 {
            reads[0].replay_with(&root, &world, &mut replays).unwrap();
        }
        assert!(world.budget().usage_snapshot().work - before < cold);
        assert_eq!(replays.replay_counts(), completed);
        let ir::Ty::Struct(original) = &root else {
            panic!("struct root")
        };
        let mut changed = original.def().clone();
        changed.fields = Arc::<[ir::FieldDef]>::from([]).into();
        let forged = ir::Ty::Struct(
            ir::StructTy::new_parent_local(changed).with_realization(reads[0].clone()),
        );
        assert!(reads[0].replay_with(&forged, &world, &mut replays).is_err());
        assert!(
            reads[0]
                .replay_with(&root, &foreign_world, &mut replays)
                .is_err()
        );
        assert_eq!(replays.replay_counts(), completed);
        reads[0].replay_with(&root, &world, &mut replays).unwrap();

        crate::compiler::frontend::with_source_argument_session_for_tests(|validate| {
            let before = world.budget().usage_snapshot().work;
            validate(&roots[0], &caller, world.budget(), 0).unwrap();
            let cold = world.budget().usage_snapshot().work - before;
            let before = world.budget().usage_snapshot().work;
            for _ in 0..8 {
                validate(&roots[1], &caller, world.budget(), 0).unwrap();
            }
            let warm = world.budget().usage_snapshot().work - before;
            assert!(
                warm < cold,
                "source strict session lost its completed roots: {warm}/{cold}"
            );
            assert!(validate(&forged, &caller, world.budget(), 0).is_err());
            assert!(validate(&root, &caller, &ir::PlanBudget::default(), 0).is_err());
            let foreign_caller = foreign
                .plan
                .callable_facts(foreign.plan.function_decl(0).unwrap())
                .unwrap();
            assert!(validate(&root, &foreign_caller, foreign_world.budget(), 0).is_err());
            // Neither the forged root nor a different caller can publish a
            // completed success into the original source session.
            validate(&root, &caller, world.budget(), 0).unwrap();
        });

        // A changed immutable fact allocation with the same claimed key is
        // never a hit. Its forged size witness must be replayed and rejected.
        let fact = engine
            .fact_of::<ComponentFact>(&component_key.0, 0)
            .unwrap();
        let mut forged_fact = fact.as_ref().clone();
        forged_fact.metadata_heights = Arc::from([usize::MAX]);
        let forged_read = ReadComponent {
            fact: Arc::new(forged_fact),
            cells: Arc::from([]),
        };
        let forged_capability =
            ReadNominal::Component(forged_read.materialize(0, world.budget()).unwrap());
        assert!(!replays.contains_fact(&forged_capability, &world).unwrap());
        let forged_root = forged_capability.definition().unwrap();
        assert!(
            forged_capability
                .replay_with(&forged_root, &world, &mut replays)
                .is_err()
        );
        assert_eq!(replays.replay_counts(), completed);
        assert!(world.budget().first_resource_failure().is_none());

        let assert_cycle = |message: &str| {
            assert!(
                message.contains("has infinite size")
                    || message.contains("infinite by-value layout"),
                "cycle control failed for an unrelated reason: {message}",
            );
        };
        let mut cycle_rejections = 0;
        for source in [
            "struct Node{next:Node}fn main(){}",
            "struct Node{next:Other}struct Other{next:Node}fn main(){}",
        ] {
            let program = crate::compiler::frontend::parse(source).unwrap();
            let plan = match crate::compiler::frontend::PlanCtx::from_program(
                &program,
                &ir::PlanBudget::default(),
            ) {
                Ok(plan) => plan,
                Err(error) => {
                    assert_cycle(&error.to_string());
                    cycle_rejections += 1;
                    continue;
                }
            };
            let world = match plan.trait_world() {
                Ok(world) => world,
                Err(error) => {
                    assert_cycle(&error.to_string());
                    cycle_rejections += 1;
                    continue;
                }
            };
            let key = producer::key(
                world.nominal_realization_world(),
                plan.nominal_declaration(0).unwrap(),
                &[],
                world.budget(),
            )
            .unwrap();
            let cyclic = crate::engine::Engine::new(Provider {
                plan: Arc::new(plan),
                roots: vec![key],
                forge: |_| {},
            });
            let rejected = cyclic.request(test_root());
            cyclic.run(workers).unwrap();
            assert_cycle(&rejected.completion().unwrap_err().to_string());
            cycle_rejections += 1;
            assert!(rejected.fact::<Vec<ReadNominal>>(0).is_err());
        }
        assert_eq!(cycle_rejections, 2);
        crate::compiler::frontend::with_source_argument_session_for_tests(|validate| {
            validate(&root, &caller, world.budget(), 0).unwrap();
            assert!(
                reads[0]
                    .replay_component_dependency(
                        &root,
                        &world,
                        world.budget().limits().depth,
                        &mut replays,
                    )
                    .is_err()
            );
            let failure = world.budget().first_resource_failure();
            assert!(failure.is_some());
            assert!(validate(&root, &caller, world.budget(), 0).is_err());
            assert_eq!(world.budget().first_resource_failure(), failure);
        });
        assert_eq!(replays.replay_counts(), completed);
    }
}

#[test]
fn completed_component_replay_shares_fact_not_rooted_type_identity() {
    for workers in [1, 8] {
        let source = "struct First{next:*mut Second,extra:Extra}struct Second{next:*mut First}struct Extra{value:i32}fn main(){}";
        let own = provider(source, &[0, 1], &[]);
        let world = own.plan.trait_world().unwrap();
        let key = ComponentKey::encode(
            world.nominal_realization_world(), &own.roots, world.budget(),
        ).unwrap();
        let foreign = provider(source, &[0, 1], &[]);
        let foreign_world = foreign.plan.trait_world().unwrap();
        let engine = crate::engine::Engine::new(own);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0].component_identity(), reads[1].component_identity());
        assert!(!reads[0].same_fact(&reads[1]));
        assert!(reads[0].same_replay_fact(&reads[1]));
        let first = reads[0].definition().unwrap();
        let second = reads[1].definition().unwrap();
        // Recursive component children still replay their exact capability,
        // but a completed type/environment row is retained only once.
        let env = ir::GenericEnv::new(&[], world.budget()).unwrap();
        let mut metadata = traits::NominalReplays::default();
        world.validate_ty_metadata_with(&first, &env, 0, &mut metadata).unwrap();
        let before_rows = metadata.replay_counts();
        let pair = ir::Ty::Tuple(Arc::from([first.clone(), first.clone()]));
        world.validate_ty_metadata_with(&pair, &env, 0, &mut metadata).unwrap();
        let after_rows = metadata.replay_counts();
        assert_eq!(after_rows.2, before_rows.2 + 1);
        assert_eq!(after_rows.3, before_rows.3 + 1);
        let mut replays = traits::NominalReplays::default();
        let before = world.budget().usage_snapshot().work;
        reads[0].replay_component_dependency(&first, &world, 2, &mut replays).unwrap();
        let cold = world.budget().usage_snapshot().work - before;
        let completed = replays.replay_counts();
        assert!(completed.0 >= 2, "external receipt was not replayed");
        assert!(replays.contains_fact_at(&reads[1], &world, 2).unwrap());
        assert!(!replays.contains_fact_at(&reads[1], &world, 3).unwrap());
        let before = world.budget().usage_snapshot().work;
        for (read, ty) in [(&reads[1], &second), (&reads[0], &first)] {
            read.replay_component_dependency(ty, &world, 1, &mut replays).unwrap();
        }
        assert!(world.budget().usage_snapshot().work - before < cold);
        assert_eq!(replays.replay_counts(), completed);

        // A whole-component proof does not equate two root selections or
        // authorize a changed definition attached to a genuine selected read.
        assert!(reads[1].replay_with(&first, &world, &mut replays).is_err());
        let ir::Ty::Struct(original) = &second else { panic!("struct second") };
        let mut changed = original.def().clone();
        changed.fields = Arc::<[ir::FieldDef]>::from([]).into();
        let forged = ir::Ty::Struct(
            ir::StructTy::new_parent_local(changed).with_realization(reads[1].clone()),
        );
        assert!(reads[1].replay_with(&forged, &world, &mut replays).is_err());
        assert!(reads[1].replay_with(&second, &foreign_world, &mut replays).is_err());
        assert_eq!(replays.replay_counts(), completed);

        let fact = engine.fact_of::<ComponentFact>(&key.0, 0).unwrap();
        let mut changed_fact = fact.as_ref().clone();
        let mut heights = changed_fact.metadata_heights.to_vec();
        heights[0] = usize::MAX;
        changed_fact.metadata_heights = heights.into();
        let forged_read = ReadComponent { fact: Arc::new(changed_fact), cells: Arc::from([]) };
        let member = forged_read.member_for(reads[1].application_key()).unwrap();
        let forged_capability = ReadNominal::Component(
            forged_read.materialize(member, world.budget()).unwrap(),
        );
        assert!(!reads[1].same_replay_fact(&forged_capability));
        assert!(!replays.contains_fact_at(&forged_capability, &world, 1).unwrap());
        let forged_root = forged_capability.definition().unwrap();
        assert!(forged_capability.replay_with(&forged_root, &world, &mut replays).is_err());

        reads[1].replay_component_dependency(&second, &world, 3, &mut replays).unwrap();
        assert!(replays.contains_fact_at(&reads[0], &world, 3).unwrap());
        assert!(!replays.contains_fact_at(&reads[0], &world, 4).unwrap());
        assert_eq!(replays.replay_counts(), completed);
        let limit = world.budget().limits().depth;
        assert!(reads[1].replay_component_dependency(&second, &world, limit, &mut replays).is_err());
        let failure = world.budget().first_resource_failure();
        assert!(failure.is_some());
        assert!(replays.contains_fact_at(&reads[0], &world, 0).is_err());
        assert_eq!(world.budget().first_resource_failure(), failure);
    }
}

#[test]
fn completed_nominal_replay_depth_preserves_nested_authority_and_limits() {
    for workers in [1, 8] {
        let source = "struct Node{next:*mut Node,extra:Extra}struct Extra{n:i32}fn main(){}";
        let own = provider(source, &[0], &[]);
        let world = own.plan.trait_world().unwrap();
        let foreign = provider(source, &[0], &[]);
        let foreign_world = foreign.plan.trait_world().unwrap();
        let engine = crate::engine::Engine::new(own);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let read = &reads[0];
        let root = read.definition().unwrap();
        let mut replays = traits::NominalReplays::default();
        let before = world.budget().usage_snapshot().work;
        read.replay_component_dependency(&root, &world, 2, &mut replays)
            .unwrap();
        let cold = world.budget().usage_snapshot().work - before;
        let completed = replays.replay_counts();
        assert!(replays.contains_fact_at(read, &world, 2).unwrap());
        assert!(!replays.contains_fact_at(read, &world, 3).unwrap());
        let before = world.budget().usage_snapshot().work;
        for depth in [2, 1, 0, 2, 1, 0] {
            read.replay_component_dependency(&root, &world, depth, &mut replays)
                .unwrap();
        }
        assert!(world.budget().usage_snapshot().work - before < cold);
        assert_eq!(replays.replay_counts(), completed);

        // A deeper successful full traversal extends, rather than replaces,
        // the exact fact's completion witness.
        read.replay_component_dependency(&root, &world, 3, &mut replays)
            .unwrap();
        assert!(replays.contains_fact_at(read, &world, 3).unwrap());
        assert!(!replays.contains_fact_at(read, &world, 4).unwrap());
        assert_eq!(replays.replay_counts(), completed);
        assert!(
            read.replay_component_dependency(&root, &foreign_world, 1, &mut replays)
                .is_err()
        );
        let ir::Ty::Struct(original) = &root else {
            panic!("struct root")
        };
        let mut changed = original.def().clone();
        changed.fields = Arc::<[ir::FieldDef]>::from([]).into();
        let forged =
            ir::Ty::Struct(ir::StructTy::new_parent_local(changed).with_realization(read.clone()));
        assert!(
            read.replay_component_dependency(&forged, &world, 1, &mut replays)
                .is_err()
        );
        read.replay_component_dependency(&root, &world, 1, &mut replays)
            .unwrap();

        // Even a successful root just below the limit cannot authorize its
        // external edge one level past that limit. Failed deeper traversals
        // never publish a stronger witness or clear the issuer's failure.
        let limit = world.budget().limits().depth;
        read.replay_component_dependency(&root, &world, limit - 1, &mut replays)
            .unwrap();
        assert!(!replays.contains_fact_at(read, &world, limit).unwrap());
        assert!(
            read.replay_component_dependency(&root, &world, limit, &mut replays)
                .is_err()
        );
        let failure = world.budget().first_resource_failure();
        assert!(failure.is_some());
        assert_eq!(replays.replay_counts(), completed);
        assert!(
            read.replay_component_dependency(&root, &world, 0, &mut replays)
                .is_err()
        );
        assert_eq!(world.budget().first_resource_failure(), failure);
    }
}

#[test]
fn source_nominal_walk_reuses_only_completed_exact_component_allocations() {
    for workers in [1, 8] {
        let provider = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let plan = provider.plan.clone();
        let engine = crate::engine::Engine::new(provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        assert!(reads[0].is_component());
        let root = reads[0].definition().unwrap();
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let budget = facts.budget();
        let validate = crate::compiler::frontend::validate_nominal_tree_for_tests;
        let before = budget.usage_snapshot();
        validate(&plan, &root, &facts).unwrap();
        let single = budget.usage_snapshot().work - before.work;
        assert!(single > 0);
        let repeated = ir::Ty::Tuple(vec![root.clone(); 32].into());
        let before = budget.usage_snapshot();
        validate(&plan, &repeated, &facts).unwrap();
        let repeated_work = budget.usage_snapshot().work - before.work;
        assert!(
            repeated_work < single * 4,
            "duplicate immutable components repeated source proof work: {repeated_work}/{single}"
        );
        let ir::Ty::Struct(nominal) = &root else {
            panic!("struct root")
        };
        let mut changed = nominal.def().clone();
        changed.fields = Vec::<ir::FieldDef>::new().into();
        let forged = ir::Ty::Struct(
            ir::StructTy::new_parent_local(changed).with_realization(reads[0].clone()),
        );
        let mixed = ir::Ty::Tuple(Arc::from([root.clone(), forged]));
        assert!(validate(&plan, &mixed, &facts).is_err());
        let foreign = self::provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let foreign_facts = foreign
            .plan
            .callable_facts(foreign.plan.function_decl(0).unwrap())
            .unwrap();
        assert!(validate(&plan, &repeated, &foreign_facts).is_err());
        assert!(budget.first_resource_failure().is_none());
    }
}

#[test]
fn scoped_topology_storage_preserves_source_world_authority() {
    for workers in [1, 8] {
        let own = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let own_world = own.plan.trait_world().unwrap();
        let foreign = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let foreign_world = foreign.plan.trait_world().unwrap();
        let engine = crate::engine::Engine::new(own);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        reads[0].replay(&root, &own_world).unwrap();
        assert!(reads[0].replay(&root, &foreign_world).is_err());
        assert!(foreign_world.budget().first_resource_failure().is_none());
        reads[0].replay(&root, &own_world).unwrap();
    }
}

#[test]
fn component_envelope_replay_compares_borrowed_canonical_members() {
    let declaration = ir::DeclId {
        snapshot: ir::SnapshotId::from_graph_word(37),
        index: 9,
    };
    let encoding = ir::PlanBudget::default();
    let first_actuals = [Term::Ref(ir::BorrowKind::Shared, Box::new(Term::Bool))];
    let second_actuals = [Term::Tuple(vec![Term::Char, Term::Int(ir::IntTy::I128)])];
    let first = producer::key(19, declaration, &first_actuals, &encoding).unwrap();
    let second = producer::key(19, declaration, &second_actuals, &encoding).unwrap();
    let mut members = vec![second.clone(), first.clone()];
    let encoded = ComponentKey::encode(19, &members, &encoding).unwrap();
    members.sort_by(|a, b| a.0.params.cmp(&b.0.params));
    let (decoded, world, decoded_members) = ComponentKey::decode(&encoded.0, &encoding).unwrap();
    assert_eq!(world, 19);
    assert_eq!(decoded, encoded);
    assert_eq!(decoded_members, members);
    let budget = ir::PlanBudget::new(ir::PlanLimits {
        bytes: 0,
        ..ir::PlanLimits::default()
    });
    for _ in 0..64 {
        assert!(
            encoded
                .matches_members(19, members.iter(), &budget)
                .unwrap()
        );
        assert_eq!(budget.usage_snapshot().bytes, 0);
    }
    for index in 0..encoded.0.params.len() {
        let mut changed = encoded.clone();
        changed.0.params[index] ^= u64::MAX;
        assert!(
            !changed
                .matches_members(19, members.iter(), &budget)
                .unwrap()
        );
        let mut truncated = encoded.clone();
        truncated.0.params.truncate(index);
        assert!(
            !truncated
                .matches_members(19, members.iter(), &budget)
                .unwrap()
        );
    }
    let mut trailing = encoded.clone();
    trailing.0.params.push(0);
    assert!(
        !trailing
            .matches_members(19, members.iter(), &budget)
            .unwrap()
    );
    let mut version = encoded.clone();
    version.0.version += 1;
    assert!(
        !version
            .matches_members(19, members.iter(), &budget)
            .unwrap()
    );
    let mut kind = encoded.clone();
    kind.0.kind += 1;
    assert!(!kind.matches_members(19, members.iter(), &budget).unwrap());
    assert!(
        encoded
            .matches_members(20, members.iter(), &budget)
            .is_err()
    );
    assert!(
        encoded
            .matches_members(19, members.iter().rev(), &budget)
            .is_err()
    );
    assert!(
        encoded
            .matches_members(19, [first.clone(), first.clone()].iter(), &budget)
            .is_err()
    );
    assert!(encoded.matches_members(19, [].iter(), &budget).is_err());
    for member in &members {
        for index in [0, 1, 2] {
            let mut foreign = member.clone();
            foreign.0.params[index] ^= u64::MAX;
            assert!(
                encoded
                    .matches_members(19, [foreign].iter(), &budget)
                    .is_err()
            );
        }
        let mut foreign_version = member.clone();
        foreign_version.0.version += 1;
        assert!(
            encoded
                .matches_members(19, [foreign_version].iter(), &budget)
                .is_err()
        );
        let mut foreign_kind = member.clone();
        foreign_kind.0.kind += 1;
        assert!(
            encoded
                .matches_members(19, [foreign_kind].iter(), &budget)
                .is_err()
        );
    }
    // Envelopes are not term proofs. Exact source-bound authentication must
    // still reject a changed actual after a structurally genuine envelope.
    assert!(!producer::matches_key(&first.0, 19, declaration, &second_actuals, &budget).unwrap());
    assert!(
        !producer::matches_key(
            &first.0,
            19,
            ir::DeclId {
                index: 10,
                ..declaration
            },
            &first_actuals,
            &budget
        )
        .unwrap()
    );
    assert!(budget.first_resource_failure().is_none());
    let denied = ir::PlanBudget::new(ir::PlanLimits {
        work: 0,
        ..ir::PlanLimits::default()
    });
    assert!(
        encoded
            .matches_members(19, members.iter(), &denied)
            .is_err()
    );
    for workers in [1, 8] {
        let own = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let own_world = own.plan.trait_world().unwrap();
        let foreign = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let foreign_world = foreign.plan.trait_world().unwrap();
        let engine = crate::engine::Engine::new(own);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        reads[0].replay(&root, &own_world).unwrap();
        assert!(reads[0].replay(&root, &foreign_world).is_err());
        assert!(foreign_world.budget().first_resource_failure().is_none());
    }
}

#[test]
fn canonical_key_replay_compares_borrowed_terms_without_retained_copies() {
    let declaration = ir::DeclId {
        snapshot: ir::SnapshotId::from_graph_word(37),
        index: 9,
    };
    let actuals = vec![
        Term::Int(ir::IntTy::I128),
        Term::Float(ir::FloatTy::F32),
        Term::Float(ir::FloatTy::F64),
        Term::Bool,
        Term::Unit,
        Term::Never,
        Term::Char,
        Term::Str,
        Term::Ref(ir::BorrowKind::Shared, Box::new(Term::Bool)),
        Term::RawPtr(ir::BorrowKind::Mut, Box::new(Term::Char)),
        Term::Tuple(vec![Term::Bool, Term::Int(ir::IntTy::U64)]),
        Term::Array(Box::new(Term::Bool), 7),
        Term::Slice(Box::new(Term::Char)),
        Term::Nominal(declaration, vec![Term::Bool]),
        Term::FnPointer(vec![Term::Bool], Box::new(Term::Unit)),
        Term::Cell(declaration, Box::new(Term::Bool)),
        Term::ManuallyDrop(declaration, Box::new(Term::Bool)),
        Term::PhantomData(declaration, Box::new(Term::Bool)),
        Term::Closure(declaration, vec![Term::Bool]),
        Term::FnItem(declaration, vec![Term::Bool]),
        Term::Dyn(
            declaration,
            vec![Term::Bool],
            vec![(declaration, Term::Char)],
        ),
        Term::Const(ir::ConstArgument::Integer {
            ty: ir::IntTy::U128,
            bits: u128::MAX,
        }),
    ];
    let encoding_budget = ir::PlanBudget::default();
    let encoded = producer::key(19, declaration, &actuals, &encoding_budget).unwrap();
    let (decoded_world, decoded_declaration, decoded_actuals) =
        producer::decode(&encoded.0, &encoding_budget).unwrap();
    assert_eq!(decoded_world, 19);
    assert_eq!(decoded_declaration, declaration);
    assert_eq!(decoded_actuals, actuals);
    let budget = ir::PlanBudget::new(ir::PlanLimits {
        bytes: 0,
        ..ir::PlanLimits::default()
    });
    for _ in 0..64 {
        assert!(producer::matches_key(&encoded.0, 19, declaration, &actuals, &budget).unwrap());
        assert_eq!(budget.usage_snapshot().bytes, 0);
    }
    for index in 0..encoded.0.params.len() {
        let mut changed = encoded.0.clone();
        changed.params[index] ^= u64::MAX;
        assert!(!producer::matches_key(&changed, 19, declaration, &actuals, &budget).unwrap());
        let mut truncated = encoded.0.clone();
        truncated.params.truncate(index);
        assert!(!producer::matches_key(&truncated, 19, declaration, &actuals, &budget).unwrap());
    }
    let mut trailing = encoded.0.clone();
    trailing.params.push(0);
    assert!(!producer::matches_key(&trailing, 19, declaration, &actuals, &budget).unwrap());
    let mut wrong_version = encoded.0.clone();
    wrong_version.version += 1;
    assert!(!producer::matches_key(&wrong_version, 19, declaration, &actuals, &budget).unwrap());
    let mut wrong_kind = encoded.0.clone();
    wrong_kind.kind += 1;
    assert!(!producer::matches_key(&wrong_kind, 19, declaration, &actuals, &budget).unwrap());
    assert!(!producer::matches_key(&encoded.0, 20, declaration, &actuals, &budget).unwrap());
    assert!(
        Term::Tuple(Vec::new())
            .matches_encoding(&[10, 0], &budget)
            .is_err()
    );
    assert!(budget.first_resource_failure().is_none());
    let denied = ir::PlanBudget::new(ir::PlanLimits {
        work: 0,
        ..ir::PlanLimits::default()
    });
    assert!(producer::matches_key(&encoded.0, 19, declaration, &actuals, &denied).is_err());
    for workers in [1, 8] {
        let own = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let own_world = own.plan.trait_world().unwrap();
        let foreign = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let foreign_world = foreign.plan.trait_world().unwrap();
        let engine = crate::engine::Engine::new(own);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        reads[0].replay(&root, &own_world).unwrap();
        assert!(reads[0].replay(&root, &foreign_world).is_err());
        assert!(foreign_world.budget().first_resource_failure().is_none());
    }
}

#[test]
fn retained_nominal_replay_rejects_foreign_world_cold_and_warm() {
    for (source, component) in [
        ("struct Empty;fn main(){}", false),
        ("struct Node{next:*mut Node}fn main(){}", true),
    ] {
        for workers in [1, 8] {
            let own_provider = provider(source, &[0], &[]);
            let world = own_provider.plan.trait_world().unwrap();
            let foreign_provider = provider(source, &[0], &[]);
            let foreign_world = foreign_provider.plan.trait_world().unwrap();
            assert_ne!(
                world.nominal_realization_world(),
                foreign_world.nominal_realization_world()
            );
            let engine = crate::engine::Engine::new(own_provider);
            let demand = engine.request(test_root());
            engine.run(workers).unwrap();
            demand.completion().unwrap();
            let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
            assert_eq!(reads[0].is_component(), component);
            let root = reads[0].definition().unwrap();
            let mut replays = crate::compiler::traits::NominalReplays::default();
            let error = reads[0]
                .replay_with(&root, &foreign_world, &mut replays)
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("foreign closed nominal replay world")
            );
            assert_eq!(replays.replay_counts(), (0, 0, 0, 0));
            assert!(foreign_world.budget().first_resource_failure().is_none());
            reads[0].replay_with(&root, &world, &mut replays).unwrap();
            let completed = replays.replay_counts();
            assert!(completed.0 > 0);
            for _ in 0..2 {
                let error = reads[0]
                    .replay_with(&root, &foreign_world, &mut replays)
                    .unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("foreign closed nominal replay world")
                );
                assert_eq!(replays.replay_counts(), completed);
                assert!(foreign_world.budget().first_resource_failure().is_none());
                // A rejection never retags or discards the original completion.
                reads[0].replay_with(&root, &world, &mut replays).unwrap();
                assert_eq!(replays.replay_counts(), completed);
            }
        }
    }
}

#[test]
fn warm_nominal_replay_observes_original_module_issuer_failure() {
    for workers in [1, 8] {
        let (provider, modules) =
            provider_with_modules("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let world = provider.plan.trait_world().unwrap();
        let engine = crate::engine::Engine::new(provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        assert!(reads[0].is_component());
        let root = reads[0].definition().unwrap();
        let mut replays = crate::compiler::traits::NominalReplays::default();
        reads[0].replay_with(&root, &world, &mut replays).unwrap();
        assert!(replays.contains_fact(&reads[0], &world).unwrap());
        let completed = replays.replay_counts();
        assert!(completed.0 > 0);
        reads[0].replay_with(&root, &world, &mut replays).unwrap();
        assert_eq!(replays.replay_counts(), completed);
        assert!(world.budget().first_resource_failure().is_none());
        assert!(
            modules
                .budget()
                .charge(crate::compiler::expansion_budget::Counter::Work, u64::MAX,)
                .is_err()
        );
        let issuer_failure = modules.budget().snapshot().failure;
        let error = reads[0]
            .replay_with(&root, &world, &mut replays)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("module authority resource boundary")
        );
        assert_eq!(replays.replay_counts(), completed);
        assert_eq!(modules.budget().snapshot().failure, issuer_failure);
        assert!(reads[0].replay_with(&root, &world, &mut replays).is_err());
        assert_eq!(replays.replay_counts(), completed);
        assert_eq!(modules.budget().snapshot().failure, issuer_failure);
    }
}

#[test]
fn symbolic_nominal_hidden_references_cannot_prove_result_absence() {
    for workers in [1, 8] {
        let provider = provider(
            "struct Node<'a>{next:&'a Node<'a>,loan:&'a i32}fn main(){}",
            &[0],
            &[],
        );
        let plan = provider.plan.clone();
        let world = plan.trait_world().unwrap();
        let facts = plan
            .instance_facts(&ir::CallTarget {
                declaration: plan.function_decl(0).unwrap(),
                type_args: Arc::from([]),
            })
            .unwrap();
        let engine = crate::engine::Engine::new(provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        assert!(
            reads[0].is_component(),
            "regression requires real symbolic component fields"
        );
        let root = reads[0].definition().unwrap();
        reads[0].check_definition(&root).unwrap();
        facts.validate_ty_metadata(&root).unwrap();
        assert_eq!(ir::root_reference_layers(&root).unwrap(), 0);
        let (opaque, height) =
            ir::reference_type_summary(&root, false, facts.budget(), 0, &mut 0).unwrap();
        assert!(
            !opaque && height > 0,
            "must reach the absence scan, not the opaque rejection"
        );
        assert!(
            !world
                .result_carriers_absent_for_tests(&root, &facts)
                .unwrap()
        );
        let ir::Ty::Struct(owner) = &root else {
            panic!("symbolic record");
        };
        // Keep simultaneous materializations alive to prove addresses are not
        // stable evidence. The production scan consumes each read once.
        let first = owner.def().fields.read(facts.budget()).unwrap();
        let second = owner.def().fields.read(facts.budget()).unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(matches!(first[1].ty, ir::Ty::Ref { .. }));
        assert!(matches!(second[1].ty, ir::Ty::Ref { .. }));
        let mut forged = owner.def().clone();
        forged.fields = Arc::<[ir::FieldDef]>::from([]).into();
        let forged = ir::Ty::Struct(ir::StructTy::new(forged));
        assert!(
            world
                .result_carriers_absent_for_tests(&forged, &facts)
                .is_err()
        );
    }
}

#[test]
fn symbolic_component_metadata_preserves_schema_shape_limits() {
    let budget = ir::PlanBudget::default();
    assert!(metadata(&Term::Tuple(vec![]), &budget, 0).is_err());
    assert!(
        metadata(
            &Term::Array(Box::new(Term::Bool), ir::MAX_ARRAY_TYPE_LEN + 1),
            &budget,
            0
        )
        .is_err()
    );
    assert!(metadata(&Term::Unit, &budget, 0).is_ok());
    for unsized_term in [Term::Str, Term::Slice(Box::new(Term::Bool))] {
        assert!(metadata(&unsized_term, &budget, 0).is_ok());
        assert!(sized(&unsized_term, &budget, 0).is_err());
        let borrowed = Term::Ref(ir::BorrowKind::Shared, Box::new(unsized_term));
        assert!(metadata(&borrowed, &budget, 0).is_ok());
        assert!(sized(&borrowed, &budget, 0).is_ok());
    }
    assert!(metadata(&Term::Slice(Box::new(Term::Str)), &budget, 0).is_err());
}

#[test]
fn finite_reference_inventory_preserves_raw_and_reference_term_policy() {
    for workers in [1, 8] {
        let provider = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let world = provider.plan.trait_world().unwrap();
        let budget = world.budget().clone();
        let component_key =
            ComponentKey::encode(world.nominal_realization_world(), &provider.roots, &budget)
                .unwrap();
        let engine = crate::engine::Engine::new(provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let component = ReadComponent {
            fact: engine
                .fact_of::<ComponentFact>(&component_key.0, 0)
                .unwrap(),
            cells: Arc::from(Vec::new()),
        };
        let marker = ir::CellTy::new(world.canonical_catalog(), ir::Ty::Bool, &budget).unwrap();
        let absent_cell = Term::Cell(marker.declaration(), Box::new(Term::Bool));
        let error = component.resolve(&absent_cell, &budget, 0).unwrap_err();
        assert!(error.0.contains("absent from its canonical read table"));
        let cases = [
            (Term::Bool, ir::Ty::Bool, false),
            (Term::Str, ir::Ty::Str, true),
            (
                Term::Slice(Box::new(Term::Bool)),
                ir::Ty::Slice {
                    element: Arc::new(ir::Ty::Bool),
                },
                true,
            ),
            (
                Term::Ref(ir::BorrowKind::Shared, Box::new(Term::Str)),
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(ir::Ty::Str),
                },
                true,
            ),
            (
                Term::Ref(
                    ir::BorrowKind::Shared,
                    Box::new(Term::Slice(Box::new(Term::Bool))),
                ),
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(ir::Ty::Slice {
                        element: Arc::new(ir::Ty::Bool),
                    }),
                },
                true,
            ),
            (
                Term::Ref(
                    ir::BorrowKind::Mut,
                    Box::new(Term::Slice(Box::new(Term::Bool))),
                ),
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Mut,
                    referent: Arc::new(ir::Ty::Slice {
                        element: Arc::new(ir::Ty::Bool),
                    }),
                },
                true,
            ),
            (
                Term::Ref(
                    ir::BorrowKind::Shared,
                    Box::new(Term::Slice(Box::new(Term::Tuple(vec![Term::Ref(
                        ir::BorrowKind::Shared,
                        Box::new(Term::Bool),
                    )])))),
                ),
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(ir::Ty::Slice {
                        element: Arc::new(ir::Ty::Tuple(Arc::from([ir::Ty::Ref {
                            kind: ir::BorrowKind::Shared,
                            referent: Arc::new(ir::Ty::Bool),
                        }]))),
                    }),
                },
                true,
            ),
            (
                Term::Ref(ir::BorrowKind::Shared, Box::new(Term::Bool)),
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(ir::Ty::Bool),
                },
                false,
            ),
            (
                Term::RawPtr(
                    ir::BorrowKind::Mut,
                    Box::new(Term::Ref(ir::BorrowKind::Shared, Box::new(Term::Bool))),
                ),
                ir::Ty::RawPtr {
                    kind: ir::BorrowKind::Mut,
                    pointee: Arc::new(ir::Ty::Ref {
                        kind: ir::BorrowKind::Shared,
                        referent: Arc::new(ir::Ty::Bool),
                    }),
                },
                true,
            ),
        ];
        for (term, ty, opaque) in cases {
            assert_eq!(component.resolve(&term, &budget, 0).unwrap(), ty);
            for allow_references in [false, true] {
                let mut work = 0;
                let mut slice_inventory = SliceInventory {
                    read: &component,
                    budget: &budget,
                    work: &mut work,
                    allow_references,
                    allow_universal_projections: false,
                    queued: Vec::new(),
                    pending: Vec::new(),
                };
                let finite = slice_inventory.term(&term, 0);
                let ordinary =
                    ir::slice_boundary_height(&ty, allow_references, false, &budget, 0, &mut 0);
                assert_eq!(finite.is_ok(), ordinary.is_ok());
                if let (Ok(finite), Ok(ordinary)) = (finite, ordinary) {
                    assert_eq!(finite, ordinary);
                }
            }
            let mut visits = 0;
            let mut inventory = ReferenceInventory {
                read: &component,
                budget: &budget,
                visits: &mut visits,
                queued: Vec::new(),
                pending: Vec::new(),
                opaque: false,
            };
            let height = inventory.term(&term, 0, 0).unwrap();
            assert_eq!(inventory.opaque, opaque);
            assert_eq!(
                ir::reference_type_summary(&ty, true, &budget, 0, &mut 0).unwrap(),
                (opaque, height)
            );
        }
        for invalid in [
            Term::Slice(Box::new(Term::Bool)),
            Term::RawPtr(
                ir::BorrowKind::Mut,
                Box::new(Term::Slice(Box::new(Term::Bool))),
            ),
            Term::Tuple(vec![Term::Ref(
                ir::BorrowKind::Shared,
                Box::new(Term::Bool),
            )]),
        ] {
            let mut work = 0;
            let mut inventory = SliceInventory {
                read: &component,
                budget: &budget,
                work: &mut work,
                allow_references: false,
                allow_universal_projections: false,
                queued: Vec::new(),
                pending: Vec::new(),
            };
            assert!(inventory.term(&invalid, 0).is_err());
        }
        let mut invalid = Term::Bool;
        for _ in 0..129 {
            invalid = Term::Ref(ir::BorrowKind::Shared, Box::new(invalid));
        }
        let invalid_array = Term::Array(Box::new(Term::Bool), ir::MAX_ARRAY_TYPE_LEN + 1);
        for invalid in [invalid, invalid_array] {
            let mut visits = 0;
            let mut inventory = ReferenceInventory {
                read: &component,
                budget: &budget,
                visits: &mut visits,
                queued: Vec::new(),
                pending: Vec::new(),
                opaque: false,
            };
            assert!(inventory.term(&invalid, 0, 0).is_err());
        }
    }
}

#[test]
fn equivalent_member_roots_share_one_actual_typed_scc_read() {
    for workers in [1, 8] {
        let provider = provider(
            "struct A{b:*mut B}struct B{a:*mut A}fn main(){}",
            &[0, 1],
            &[],
        );
        let engine = crate::engine::Engine::new(provider);
        let demand = engine.request(test_root());
        let report = engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0].component_identity(), reads[1].component_identity());
        let a = reads[0].definition().unwrap();
        let b = reads[1].definition().unwrap();
        assert_ne!(a, b, "distinct members must not share nominal identity");
        for (read, ty) in reads.iter().zip([&a, &b]) {
            read.check_definition(ty).unwrap();
        }
        assert!(reads[0].check_definition(&b).is_err());
        let components = report
            .node_evaluations
            .iter()
            .filter(|(key, _)| {
                key.kind == super::super::KIND
                    && key.params.get(1) == Some(&super::super::component_key::DOMAIN)
            })
            .collect::<Vec<_>>();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].0.params[3], 2);
        assert_eq!(components[0].1, 1);
        assert!(
            report
                .node_evaluations
                .iter()
                .all(|(key, _)| key.kind != 103)
        );
    }
}

#[test]
fn escaped_symbolic_fields_outlive_all_roots_without_leaking_component() {
    let provider = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
    let world = provider.plan.trait_world().unwrap();
    let budget = world.budget().clone();
    let component_key =
        ComponentKey::encode(world.nominal_realization_world(), &provider.roots, &budget).unwrap();
    let engine = crate::engine::Engine::new(provider);
    let demand = engine.request(test_root());
    engine.run(1).unwrap();
    demand.completion().unwrap();
    let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
    let root = reads[0].definition().unwrap();
    let ir::Ty::Struct(nominal) = &root else {
        panic!("struct root")
    };
    let fields = nominal.def().fields.clone();
    let fact = engine
        .fact_of::<ComponentFact>(&component_key.0, 0)
        .unwrap();
    let weak = Arc::downgrade(&fact);
    drop(fact);
    drop(root);
    drop(reads);
    drop(demand);
    drop(engine);
    assert!(
        weak.upgrade().is_some(),
        "escaped fields retain successful read"
    );
    let tiny = ir::PlanBudget::new(ir::PlanLimits {
        bytes: 1,
        ..ir::PlanLimits::default()
    });
    assert!(
        fields.read(&tiny).is_err(),
        "field materialization must be explicitly fallible and budgeted"
    );
    let materialized = fields.read(&budget).unwrap();
    let ir::Ty::RawPtr { pointee, .. } = &materialized[0].ty else {
        panic!("pointer field")
    };
    let ir::Ty::Struct(child) = pointee.as_ref() else {
        panic!("rooted child")
    };
    assert!(!child.is_back_edge());
    child
        .realization()
        .unwrap()
        .check_definition(pointee)
        .unwrap();
    drop(materialized);
    drop(fields);
    assert!(
        weak.upgrade().is_none(),
        "component read must not leak or self-own"
    );
}

#[test]
fn forged_component_members_fields_witnesses_and_external_receipts_reject() {
    let mutations: &[fn(&mut ComponentFact)] = &[
        |fact| fact.members = Arc::from([]),
        |fact| {
            let mut members = fact.members.to_vec();
            let mut fields = members[0].fields.to_vec();
            fields[0].1 = Term::Unit;
            members[0].fields = fields.into();
            fact.members = members.into();
        },
        |fact| fact.layout_order = Arc::from([]),
        |fact| fact.metadata_heights = Arc::from([usize::MAX]),
        |fact| fact.external = Arc::from([]),
        |fact| {
            let mut members = fact.members.to_vec();
            members.push(members[0].clone());
            fact.members = members.into();
        },
    ];
    for &forge in mutations {
        for workers in [1, 8] {
            let mut provider = provider(
                "struct Node{next:*mut Node,extra:Extra}struct Extra{n:i32}fn main(){}",
                &[0],
                &[],
            );
            provider.forge = forge;
            let engine = crate::engine::Engine::new(provider);
            let demand = engine.request(test_root());
            engine.run(workers).unwrap();
            assert!(demand.completion().is_err());
            assert!(demand.fact::<Vec<ReadNominal>>(0).is_err());
        }
    }
}

fn native_sequence_provider() -> (Provider, usize) {
    use crate::compiler::{frontend, source};
    let options = source::SourceOptions::default();
    let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "heap-carrier.rs",
            "fn main(){} fn foreign<T>(){} fn heap_invariant<T>(owner:&mut Vec<T>,value:T){} fn heap_shared<T>(owner:&Vec<T>,value:T){} fn heap_wrong_family<T,U>(owner:&mut Vec<T>,value:U){} fn heap_aggregate<T>(owner:&std::collections::HashMap<T,T>,value:T){}",
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
    let program = frontend::resolve(&frontend::parse_prepared(&prepared).unwrap()).unwrap();
    // Source spelling selects this fixture's declaration, never grants authority.
    // The producer must replay the embedded issuer and exact declaration itself.
    let index = program
        .structs
        .iter()
        .position(|decl| {
            let path = decl.name.segments();
            path.len() == 3
                && path
                    .iter()
                    .zip(["__justrust_std", "vec", "Vec"])
                    .all(|(&name, expected)| program.names.spelling(name).unwrap() == expected)
        })
        .expect("embedded native Vec");
    let main = program
        .functions
        .iter()
        .position(|function| {
            let path = function.name.segments();
            path.len() == 1 && program.names.spelling(path[0]).unwrap() == "main"
        })
        .expect("source main");
    let plan =
        Arc::new(frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap());
    let world = plan.trait_world().unwrap();
    let root = producer::key(
        world.nominal_realization_world(),
        plan.nominal_declaration(index).unwrap(),
        &[Term::Ref(ir::BorrowKind::Shared, Box::new(Term::Str))],
        world.budget(),
    )
    .unwrap();
    (
        Provider {
            plan,
            roots: vec![root],
            forge: |_| {},
        },
        main,
    )
}

#[test]
fn native_heap_carrier_coordinate_uses_exact_nominal_read() {
    for workers in [1, 8] {
        let (provider, main) = native_sequence_provider();
        let plan = provider.plan.clone();
        let facts = plan
            .instance_facts(&ir::CallTarget {
                declaration: plan.function_decl(main).unwrap(),
                type_args: Arc::from([]),
            })
            .unwrap();
        let engine = crate::engine::Engine::new(provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        facts.validate_ty(&root).unwrap();
        let leaf = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Str),
        };
        assert_eq!(root.owned_sequence_element(), Some(&leaf));
        let location = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([ir::CarrierStep::SliceElement])),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
        };
        assert_eq!(
            ir::carrier_locations(&root, facts.budget())
                .unwrap()
                .as_ref(),
            std::slice::from_ref(&location)
        );
        assert_eq!(
            ir::carrier_at(&root, &location, facts.budget()).unwrap(),
            leaf
        );
        let summary = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([
                ir::CarrierStep::Referent,
                ir::CarrierStep::SliceElement,
            ])),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
        };
        for (kind, slot_writable) in [(ir::BorrowKind::Shared, false), (ir::BorrowKind::Mut, true)]
        {
            let owner = ir::Ty::Ref {
                kind,
                referent: Arc::new(root.clone()),
            };
            // Fixed0 stops at the stored &str slot. Exclusive access to the
            // graph-issued Vec permits replacement, not a write through &str.
            assert_eq!(
                ir::carrier_at(&owner, &summary, facts.budget()).unwrap(),
                leaf,
            );
            assert_eq!(
                facts
                    .world()
                    .carrier_writable_for_tests(&owner, &summary)
                    .unwrap(),
                slot_writable,
                "slot replacement follows the original {kind:?} owner"
            );
            let mut shared_pointee = summary.clone();
            shared_pointee.channel = ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 1 });
            assert!(
                ir::carrier_at(&owner, &shared_pointee, facts.budget()).is_err(),
                "the str pointee is not the original stored reference carrier"
            );
            assert!(
                !facts
                    .world()
                    .carrier_writable_for_tests(&owner, &shared_pointee)
                    .unwrap(),
                "a shared element never grants pointee writes through {kind:?} owner"
            );
            let explicit_pointee = ir::CarrierLocation {
                path: ir::CarrierPath(Arc::from([
                    ir::CarrierStep::Referent,
                    ir::CarrierStep::SliceElement,
                    ir::CarrierStep::Referent,
                ])),
                channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
            };
            assert!(
                !facts
                    .world()
                    .carrier_writable_for_tests(&owner, &explicit_pointee)
                    .unwrap(),
                "an explicit shared-element dereference also clears pointee permission"
            );
            assert!(
                facts
                    .world()
                    .carrier_writable_for_tests(&owner, &location)
                    .is_err(),
                "a slot lens cannot omit the original owner dereference"
            );
        }
        let shared_barrier = ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(root.clone()),
            }),
        };
        let nested_slot = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([
                ir::CarrierStep::Referent,
                ir::CarrierStep::Referent,
                ir::CarrierStep::SliceElement,
            ])),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
        };
        assert!(
            !facts
                .world()
                .carrier_writable_for_tests(&shared_barrier, &nested_slot)
                .unwrap(),
            "a mutable outer reference cannot bypass a shared owner"
        );
        let mutable_leaf = ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(ir::Ty::Int(ir::IntTy::I32)),
        };
        let field = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([
                ir::CarrierStep::Referent,
                ir::CarrierStep::Field(0),
            ])),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
        };
        for (kind, writable) in [(ir::BorrowKind::Shared, false), (ir::BorrowKind::Mut, true)] {
            let owner = ir::Ty::Ref {
                kind,
                referent: Arc::new(ir::Ty::Tuple(Arc::from([mutable_leaf.clone()]))),
            };
            assert_eq!(
                facts
                    .world()
                    .carrier_writable_for_tests(&owner, &field)
                    .unwrap(),
                writable,
                "ordinary exact mutable-owner field authority remains intact"
            );
        }
        let ir::Ty::Struct(nominal) = root else {
            panic!("native owner must be nominal")
        };
        let unread = ir::Ty::Struct(
            ir::StructTy::new_parent_local(nominal.def().clone())
                .with_native_sequence_binding(nominal.native_sequence_binding().cloned()),
        );
        assert!(
            unread.owned_sequence_element().is_none(),
            "closed actuals require an issued graph read"
        );
        let unread_owner = ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(unread),
        };
        assert!(
            facts
                .world()
                .carrier_writable_for_tests(&unread_owner, &summary)
                .is_err(),
            "exclusive syntax cannot replace the missing original graph-issued heap authority"
        );
        let original = plan
            .nominal_realization_template(nominal.def().source_decl)
            .unwrap();
        assert!(
            original.owned_sequence_element().is_some(),
            "exact original own-binder template is issued"
        );
        let ir::Ty::Struct(original) = original else {
            panic!("native source template")
        };
        let program = plan.sema_program();
        for (name, expected) in [
            ("heap_invariant", 1usize),
            ("heap_shared", 0),
            ("heap_wrong_family", 0),
        ] {
            let index = program
                .functions
                .iter()
                .position(|function| {
                    let path = function.name.segments();
                    path.len() == 1 && program.names.spelling(path[0]).unwrap() == name
                })
                .unwrap();
            let callable = plan
                .callable_facts(plan.function_decl(index).unwrap())
                .unwrap();
            let view = callable
                .carrier_contract(ir::SignatureSubject::Body)
                .unwrap();
            let parameter = callable.env().ordered_params().next().unwrap();
            let heap_family = ir::CarrierLocation {
                path: ir::CarrierPath(Arc::from([
                    ir::CarrierStep::Referent,
                    ir::CarrierStep::SliceElement,
                ])),
                channel: ir::CarrierChannel::ParamTail {
                    parameter,
                    prefix: 0,
                },
            };
            let declared_families: Vec<_> = view
                .source_region_env()
                .carriers
                .iter()
                .filter(|row| row.parameter == Some(0) && row.location.path == heap_family.path)
                .collect();
            assert_eq!(declared_families.len(), 1, "source heap family for {name}");
            assert_eq!(declared_families[0].location, heap_family);
            assert_eq!(
                declared_families[0].region,
                ir::CarrierRegionTerm::Family { parameter },
                "source declaration retains its own element family for {name}"
            );
            assert_eq!(
                view.native_heap_invariants().len(),
                expected,
                "canonical lifetime constraint for {name}"
            );
            if expected != 0 {
                let row = &view.native_heap_invariants()[0];
                assert_eq!(row.storage.param, 0);
                assert_eq!(row.sources.len(), 1);
                assert_eq!(row.sources[0].param, 1);
                let parameter = callable.env().ordered_params().next().unwrap();
                let owner = plan
                    .intern_nominal(
                        nominal.def().source_decl,
                        &[ir::Ty::Param(parameter)],
                        ir::FactsCtx::Universal(callable.env()),
                        callable.budget(),
                    )
                    .unwrap();
                let receiver = ir::Ty::Ref {
                    kind: ir::BorrowKind::Mut,
                    referent: Arc::new(owner),
                };
                assert_eq!(
                    ir::carrier_at(&receiver, &row.storage.location, callable.budget()).unwrap(),
                    ir::Ty::Param(parameter),
                    "the writable family is the exact declaration-owned stored value"
                );
                assert!(
                    callable
                        .world()
                        .carrier_writable_for_tests(&receiver, &row.storage.location)
                        .unwrap(),
                    "original mutable ownership grants only a deferred slot envelope, rechecked after substitution"
                );
                let ir::Ty::Ref { referent, .. } = receiver else {
                    unreachable!()
                };
                let shared_receiver = ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent,
                };
                assert!(
                    !callable
                        .world()
                        .carrier_writable_for_tests(&shared_receiver, &row.storage.location)
                        .unwrap(),
                    "the identical lifetime relation cannot grant a write through shared ownership"
                );
            }
        }
        let aggregate_index = program
            .functions
            .iter()
            .position(|function| {
                let path = function.name.segments();
                path.len() == 1 && program.names.spelling(path[0]).unwrap() == "heap_aggregate"
            })
            .unwrap();
        let aggregate = plan
            .callable_facts(plan.function_decl(aggregate_index).unwrap())
            .unwrap();
        let signature = plan
            .signature(plan.function_decl(aggregate_index).unwrap())
            .unwrap()
            .signature;
        let descriptor = ir::InputCarrier {
            param: 0,
            location: ir::CarrierLocation {
                path: ir::CarrierPath(Arc::from([])),
                channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
            },
        };
        let families = ir::carrier_locations(&signature.params[0], aggregate.budget()).unwrap();
        let mut count = 0;
        for location in families.iter().filter(|location| {
            matches!(
                location.channel,
                ir::CarrierChannel::ParamTail { prefix: 0, .. }
            )
        }) {
            let stored = ir::InputCarrier {
                param: 0,
                location: location.clone(),
            };
            let retains = |signature: &ir::Signature,
                           descriptor: &ir::InputCarrier,
                           stored: &ir::InputCarrier| {
                aggregate
                    .world()
                    .shared_native_field_family_for_tests(signature, descriptor, stored)
            };
            assert!(retains(&signature, &descriptor, &stored).unwrap());
            count += 1;
            for mode in 0..5 {
                let mut changed_signature = signature.clone();
                let mut changed_descriptor = descriptor.clone();
                let mut changed_stored = stored.clone();
                match mode {
                    0 => changed_stored.param = 1,
                    1 => {
                        changed_descriptor.location.path = ir::CarrierPath(Arc::from([
                            ir::CarrierStep::Referent,
                            ir::CarrierStep::Field(0),
                        ]))
                    }
                    2 => {
                        let ir::CarrierChannel::ParamTail { parameter, .. } = location.channel
                        else {
                            unreachable!()
                        };
                        changed_stored.location.channel = ir::CarrierChannel::ParamTail {
                            parameter: ir::ParamId {
                                index: parameter.index + 1,
                                ..parameter
                            },
                            prefix: 0,
                        };
                    }
                    3 => {
                        let ir::Ty::Ref { referent, .. } = &signature.params[0] else {
                            unreachable!()
                        };
                        changed_signature.params[0] = ir::Ty::Ref {
                            kind: ir::BorrowKind::Mut,
                            referent: referent.clone(),
                        };
                    }
                    _ => {
                        changed_stored.location.path = ir::CarrierPath(Arc::from([
                            ir::CarrierStep::Referent,
                            ir::CarrierStep::SliceElement,
                        ]))
                    }
                }
                assert!(
                    !matches!(
                        retains(&changed_signature, &changed_descriptor, &changed_stored),
                        Ok(true)
                    ),
                    "aggregate forged mode {mode}"
                );
            }
        }
        assert_eq!(
            count, 2,
            "key and value native fields retain separate coordinates"
        );
        let foreign_index = program
            .functions
            .iter()
            .position(|function| {
                let path = function.name.segments();
                path.len() == 1 && program.names.spelling(path[0]).unwrap() == "foreign"
            })
            .unwrap();
        let foreign_facts = plan
            .callable_facts(plan.function_decl(foreign_index).unwrap())
            .unwrap();
        let foreign = foreign_facts.env().ordered_params().next().unwrap();
        assert!(
            !facts.env().contains(foreign),
            "foreign binder is genuinely distinct"
        );
        let source_application = plan
            .intern_nominal(
                nominal.def().source_decl,
                &[ir::Ty::Param(foreign)],
                ir::FactsCtx::Universal(foreign_facts.env()),
                facts.budget(),
            )
            .unwrap();
        assert_eq!(
            source_application.owned_sequence_element(),
            Some(&ir::Ty::Param(foreign)),
            "canonical source interner issues exact binder substitution evidence"
        );
        let ir::Ty::Struct(issued_source) = &source_application else {
            panic!("source application")
        };
        assert!(
            issued_source.realization().is_none(),
            "this is source schema authority, not executable read"
        );
        assert!(
            plan.intern_nominal(
                nominal.def().source_decl,
                &[ir::Ty::Param(foreign)],
                ir::FactsCtx::Universal(facts.env()),
                facts.budget()
            )
            .is_err(),
            "warm source application cannot admit foreign parameters in the wrong binder"
        );
        let retains_return =
            |owner: ir::Ty, kind: ir::BorrowKind, storage_param: usize, family: ir::ParamId| {
                let signature = ir::Signature {
                    params: vec![ir::Ty::Ref {
                        kind,
                        referent: Arc::new(owner),
                    }],
                    result: ir::Ty::Unit,
                    ref_contract: Arc::new(ir::RefContract::default()),
                };
                let descriptor = ir::InputCarrier {
                    param: 0,
                    location: ir::CarrierLocation {
                        path: ir::CarrierPath(Arc::from([])),
                        channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
                    },
                };
                let stored = ir::InputCarrier {
                    param: storage_param,
                    location: ir::CarrierLocation {
                        path: ir::CarrierPath(Arc::from([
                            ir::CarrierStep::Referent,
                            ir::CarrierStep::SliceElement,
                        ])),
                        channel: ir::CarrierChannel::ParamTail {
                            parameter: family,
                            prefix: 0,
                        },
                    },
                };
                facts
                    .world()
                    .shared_return_family_for_tests(&signature, &descriptor, &stored)
                    .map(|(write, retains)| {
                        assert!(
                            !write,
                            "native return retention never extends slice write eligibility"
                        );
                        retains
                    })
            };
        assert!(
            retains_return(
                source_application.clone(),
                ir::BorrowKind::Shared,
                0,
                foreign
            )
            .unwrap(),
            "exact source application retains its own shared result family"
        );
        for (kind, storage_param, family) in [
            (ir::BorrowKind::Mut, 0, foreign),
            (ir::BorrowKind::Shared, 1, foreign),
            (
                ir::BorrowKind::Shared,
                0,
                ir::ParamId {
                    index: foreign.index + 1,
                    ..foreign
                },
            ),
        ] {
            assert!(
                !matches!(
                    retains_return(source_application.clone(), kind, storage_param, family),
                    Ok(true)
                ),
                "changed receiver, parameter or family grants no native return retention"
            );
        }
        let transplanted = ir::Ty::Struct(
            ir::StructTy::new_parent_local(issued_source.def().clone())
                .with_native_sequence_binding(issued_source.native_sequence_binding().cloned()),
        );
        assert!(
            !matches!(
                retains_return(transplanted.clone(), ir::BorrowKind::Shared, 0, foreign),
                Ok(true)
            ),
            "retention replays the exact canonical allocation, not its copied fields"
        );
        assert!(
            transplanted.owned_sequence_element().is_none(),
            "same ID and fields do not replay retained allocation proof"
        );
        let source_closed = plan
            .intern_nominal(
                nominal.def().source_decl,
                std::slice::from_ref(&leaf),
                ir::FactsCtx::Concrete,
                facts.budget(),
            )
            .unwrap();
        assert_eq!(source_closed.owned_sequence_element(), Some(&leaf));
        let mut substitution_context = plan.as_ref().clone();
        let transported = ir::substitute_ty(
            &source_application,
            foreign_facts.env(),
            std::slice::from_ref(&leaf),
            ir::FactsCtx::Concrete,
            &mut substitution_context,
            facts.budget(),
        )
        .unwrap();
        assert_eq!(
            transported.owned_sequence_element(),
            Some(&leaf),
            "canonical transported source allocation retains its exact logical element"
        );
        assert_eq!(
            ir::carrier_locations(&transported, facts.budget())
                .unwrap()
                .as_ref(),
            std::slice::from_ref(&location)
        );
        let ir::Ty::Struct(transported_owner) = &transported else {
            panic!("native source transport")
        };
        assert!(
            transported_owner.realization().is_none(),
            "source coordinate does not mint a graph read"
        );
        let rebound_transport = ir::Ty::Struct(
            ir::StructTy::new_parent_local(transported_owner.def().clone())
                .with_native_sequence_binding(transported_owner.native_sequence_binding().cloned()),
        );
        assert!(
            rebound_transport.owned_sequence_element().is_none(),
            "copying transported fields cannot replay exact allocation proof"
        );
        assert!(
            ir::substitute_ty(
                &source_application,
                foreign_facts.env(),
                &[ir::Ty::Param(foreign)],
                ir::FactsCtx::Universal(facts.env()),
                &mut substitution_context,
                facts.budget(),
            )
            .is_err(),
            "warm transported schema does not admit a foreign caller binder"
        );
        let mut foreign_definition = original.def().clone();
        foreign_definition.type_args = Arc::from([ir::Ty::Param(foreign)]);
        let mut fields = foreign_definition
            .fields
            .read(facts.budget())
            .unwrap()
            .to_vec();
        fields[0].ty = ir::Ty::RawPtr {
            kind: ir::BorrowKind::Mut,
            pointee: Arc::new(ir::Ty::Param(foreign)),
        };
        foreign_definition.fields = fields.into();
        let foreign_schema = ir::Ty::Struct(
            ir::StructTy::new_parent_local(foreign_definition)
                .with_native_sequence_binding(original.native_sequence_binding().cloned()),
        );
        assert!(
            foreign_schema.owned_sequence_element().is_none(),
            "foreign-binder no-read schema needs authenticated substitution"
        );
        assert!(ir::carrier_at(&foreign_schema, &location, facts.budget()).is_err());
        let foreign_owner = ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(foreign_schema),
        };
        assert!(
            facts
                .world()
                .carrier_writable_for_tests(&foreign_owner, &summary)
                .is_err()
        );
        for mode in 0..5 {
            let mut changed = nominal.def().clone();
            match mode {
                0 => changed.name = "Vec".into(),
                1 => changed.source_decl.index += 1,
                2 => changed.type_args = Arc::from([ir::Ty::Bool]),
                3 => {
                    let mutable = ir::Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        referent: Arc::new(ir::Ty::Str),
                    };
                    changed.type_args = Arc::from([mutable.clone()]);
                    let mut fields = changed.fields.read(facts.budget()).unwrap().to_vec();
                    fields[0].ty = ir::Ty::RawPtr {
                        kind: ir::BorrowKind::Mut,
                        pointee: Arc::new(mutable),
                    };
                    changed.fields = fields.into();
                }
                _ => changed.fields = Vec::<ir::FieldDef>::new().into(),
            }
            let forged = ir::Ty::Struct(
                ir::StructTy::new_parent_local(changed)
                    .with_native_sequence_binding(nominal.native_sequence_binding().cloned())
                    .with_realization(reads[0].clone()),
            );
            assert!(
                forged.owned_sequence_element().is_none(),
                "changed native owner {mode}"
            );
            assert!(
                facts.validate_ty(&forged).is_err(),
                "counterfeit read {mode}"
            );
            let forged_owner = ir::Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent: Arc::new(forged),
            };
            assert!(
                facts
                    .world()
                    .carrier_writable_for_tests(&forged_owner, &summary)
                    .is_err(),
                "counterfeit owner {mode} cannot reach writable summary query"
            );
        }
        let first_failure = facts.budget().first_resource_failure();
        facts.budget().poison_ledger_for_tests();
        assert!(
            ir::substitute_ty(
                &source_application,
                foreign_facts.env(),
                std::slice::from_ref(&leaf),
                ir::FactsCtx::Concrete,
                &mut substitution_context,
                facts.budget(),
            )
            .is_err(),
            "warm source transport observes sticky poisoned ledger"
        );
        assert_eq!(facts.budget().first_resource_failure(), first_failure);
    }
}

#[test]
fn copied_native_vec_source_cannot_issue_heap_carriers() {
    for workers in [1, 8] {
        let provider = provider(
            "mod __justrust_std{pub mod vec{pub struct Vec<T>{ptr:*mut T,len:usize,cap:usize}}}fn main(){}",
            &[0],
            &[Term::Ref(ir::BorrowKind::Shared, Box::new(Term::Str))],
        );
        let budget = provider.plan.trait_world().unwrap().budget().clone();
        let engine = crate::engine::Engine::new(provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        reads[0].check_definition(&root).unwrap();
        assert!(
            root.owned_sequence_element().is_none(),
            "a real read of copied source is not a native issuer"
        );
        let ir::Ty::Struct(value) = &root else {
            panic!("source record")
        };
        assert!(value.native_sequence_binding().is_none());
        let location = ir::CarrierLocation {
            path: ir::CarrierPath(Arc::from([ir::CarrierStep::SliceElement])),
            channel: ir::CarrierChannel::Fixed(ir::ReferencePosition { depth: 0 }),
        };
        assert!(ir::carrier_at(&root, &location, &budget).is_err());
    }
}

#[test]
fn rooted_component_schema_replay_rejects_changed_definition_world_and_budget() {
    for workers in [1, 8] {
        let provider = provider(
            "struct Node{next:*mut Node}fn main(){}fn first<A>(){}fn second<B>(){}",
            &[0],
            &[],
        );
        let plan = provider.plan.clone();
        let facts = plan
            .instance_facts(&ir::CallTarget {
                declaration: plan.function_decl(0).unwrap(),
                type_args: Arc::from([]),
            })
            .unwrap();
        let engine = crate::engine::Engine::new(provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        facts.validate_ty(&root).unwrap();
        facts.validate_ty_metadata(&root).unwrap();
        let context = ir::FactsCtx::Validated(&facts);
        ir::validate_slice_carrier_with_facts(&root, context).unwrap();
        ir::validate_shared_slice_element(&root, context, facts.budget()).unwrap();
        let tuple_element = ir::Ty::Tuple(Arc::from([root.clone(), ir::Ty::Bool]));
        ir::validate_shared_slice_element(&tuple_element, context, facts.budget()).unwrap();
        let (opaque, height) =
            ir::reference_type_summary(&root, true, facts.budget(), 0, &mut 0).unwrap();
        assert!(
            opaque,
            "recursive raw pointer remains an opaque input envelope"
        );
        assert!(height > 0);
        ir::validate_stored_ref_contract(&[], &root, &ir::RefContract::default(), facts.budget())
            .unwrap();
        let depth_budget = ir::PlanBudget::default();
        assert!(
            ir::reference_type_summary(
                &root,
                true,
                &depth_budget,
                depth_budget.limits().depth,
                &mut 0
            )
            .is_err()
        );
        assert!(
            ir::slice_boundary_height(
                &root,
                true,
                false,
                &depth_budget,
                depth_budget.limits().depth,
                &mut 0,
            )
            .is_err()
        );
        let ir::Ty::Struct(nominal) = &root else {
            panic!("struct root")
        };
        let mut changed = nominal.def().clone();
        changed.fields = Vec::<ir::FieldDef>::new().into();
        let forged = ir::Ty::Struct(
            ir::StructTy::new_parent_local(changed).with_realization(reads[0].clone()),
        );
        assert_eq!(
            nominal.id(),
            match &forged {
                ir::Ty::Struct(n) => n.id(),
                _ => unreachable!(),
            }
        );
        assert!(
            ir::reference_type_summary(&forged, true, facts.budget(), 0, &mut 0).is_err(),
            "raw inventory authenticates the exact component definition"
        );
        assert!(ir::validate_slice_carrier_with_facts(&forged, context).is_err());
        assert!(
            facts.validate_ty(&forged).is_err(),
            "same ID must not authenticate changed allocation"
        );
        assert!(
            matches!(facts.validate_ty_metadata(&forged), Err(error) if error.kind() == traits::FactsErrorKind::InvalidMetadata),
            "metadata replay must authenticate the exact definition, not its ID"
        );
        let foreign = self::provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let foreign_facts = foreign
            .plan
            .instance_facts(&ir::CallTarget {
                declaration: foreign.plan.function_decl(0).unwrap(),
                type_args: Arc::from([]),
            })
            .unwrap();
        assert!(
            foreign_facts.validate_ty(&root).is_err(),
            "foreign world cannot replay component"
        );
        assert!(
            matches!(foreign_facts.validate_ty_metadata(&root), Err(error) if error.kind() == traits::FactsErrorKind::InvalidMetadata),
            "metadata replay cannot borrow a foreign world's component proof"
        );
        assert!(
            ir::validate_shared_slice_element(
                &root,
                ir::FactsCtx::Validated(&foreign_facts),
                foreign_facts.budget(),
            )
            .is_err()
        );
        let foreign_budget = ir::PlanBudget::default();
        assert!(
            facts
                .validate_ty_with_budget(&root, &foreign_budget)
                .is_err(),
            "changed budget authority cannot authorize replay"
        );
        assert!(
            matches!(
                facts.validate_ty_metadata_with_budget(&root, &foreign_budget),
                Err(error) if error.kind() == traits::FactsErrorKind::InvalidMetadata
            ),
            "metadata replay requires the original planning budget authority"
        );
        let first = plan.callable_facts(plan.function_decl(1).unwrap()).unwrap();
        let second = plan.callable_facts(plan.function_decl(2).unwrap()).unwrap();
        let parameter = first.env().ordered_params().next().unwrap();
        assert!(!second.env().contains(parameter));
        let contextual = ir::Ty::Tuple(Arc::from([root.clone(), ir::Ty::Param(parameter)]));
        let mut scratch = traits::TraitScratch::default();
        first
            .validate_ty_metadata_scratch(&contextual, first.budget(), &mut scratch)
            .unwrap();
        assert!(
            matches!(
                second.validate_ty_metadata_scratch(&contextual, second.budget(), &mut scratch),
                Err(error) if error.kind() == traits::FactsErrorKind::InvalidMetadata
            ),
            "warm component proof must not authenticate a parameter from another binder"
        );
        facts
            .validate_ty_metadata_scratch(&root, facts.budget(), &mut scratch)
            .unwrap();
        // Measure real replay-window peak admission, including scoped owners
        // that refund before return. Lift current to the historical highwater
        // with a real parent lease, so old fixture peaks cannot mask this one.
        let world = plan.trait_world().unwrap();
        let budget = facts.budget();
        let before = budget.usage_snapshot();
        assert_eq!(before.bytes, before.reservations.current_bytes);
        assert!(!before.reservations.counters_saturated);
        let lift_bytes = before
            .reservations
            .peak_bytes
            .checked_sub(before.bytes)
            .unwrap();
        let lift = budget.reserve_bytes(lift_bytes).unwrap();
        let window = budget.usage_snapshot();
        assert_eq!(window.bytes, window.reservations.current_bytes);
        assert_eq!(window.bytes, window.reservations.peak_bytes);
        reads[0].replay(&root, &world).unwrap();
        let measured = budget.usage_snapshot();
        assert_eq!(measured.bytes, measured.reservations.current_bytes);
        assert!(!measured.reservations.counters_saturated);
        let replay_net = measured.bytes.checked_sub(window.bytes).unwrap();
        let replay_peak = measured
            .reservations
            .peak_bytes
            .checked_sub(window.bytes)
            .unwrap();
        assert!(replay_peak >= replay_net && replay_net > 0);
        drop(lift);
        assert_eq!(
            budget.usage_snapshot().bytes,
            measured.bytes.checked_sub(lift_bytes).unwrap()
        );

        // Real filler leases leave exactly P, then P-1, of the unchanged byte
        // limit. P success plus P-1 failure proves the minimum sufficient
        // admission. No guessed slack, telemetry reset or raw counter edits.
        let limit = budget.limits().bytes;
        let before_exact = budget.usage_snapshot().bytes;
        let exact_fill = limit
            .checked_sub(before_exact)
            .and_then(|remaining| remaining.checked_sub(replay_peak))
            .unwrap();
        let exact_filler = budget.reserve_bytes(exact_fill).unwrap();
        assert_eq!(
            limit.checked_sub(budget.usage_snapshot().bytes),
            Some(replay_peak)
        );
        assert!(budget.first_resource_failure().is_none());
        reads[0].replay(&root, &world).unwrap();
        assert_eq!(
            budget.usage_snapshot().bytes,
            limit
                .checked_sub(replay_peak)
                .unwrap()
                .checked_add(replay_net)
                .unwrap()
        );
        drop(exact_filler);
        assert_eq!(
            budget.usage_snapshot().bytes,
            before_exact.checked_add(replay_net).unwrap()
        );
        assert_eq!(
            budget.usage_snapshot().bytes,
            budget.usage_snapshot().reservations.current_bytes
        );

        let short_headroom = replay_peak.checked_sub(1).unwrap();
        let before_short = budget.usage_snapshot().bytes;
        let short_fill = limit
            .checked_sub(before_short)
            .and_then(|remaining| remaining.checked_sub(short_headroom))
            .unwrap();
        let short_filler = budget.reserve_bytes(short_fill).unwrap();
        assert_eq!(
            limit.checked_sub(budget.usage_snapshot().bytes),
            Some(short_headroom)
        );
        assert!(budget.first_resource_failure().is_none());
        assert!(reads[0].replay(&root, &world).is_err());
        let short_failure = budget.first_resource_failure().unwrap();
        assert_eq!(short_failure.kind, ir::ResourceBudgetKind::Bytes);
        assert!(!budget.usage_snapshot().poisoned);
        let before_filler_drop = budget.usage_snapshot().bytes;
        drop(short_filler);
        assert_eq!(
            budget.usage_snapshot().bytes,
            before_filler_drop.checked_sub(short_fill).unwrap()
        );
        assert_eq!(budget.first_resource_failure(), Some(short_failure));
        let first_failure = facts.budget().first_resource_failure();
        facts.budget().poison_ledger_for_tests();
        assert!(reads[0].replay(&root, &world).is_err());
        let failure = facts
            .validate_ty_metadata_scratch(&root, facts.budget(), &mut scratch)
            .unwrap_err();
        assert_eq!(failure.kind(), traits::FactsErrorKind::Resource);
        assert!(facts.budget().usage_snapshot().poisoned);
        assert_eq!(facts.budget().first_resource_failure(), first_failure);
        assert!(ir::reference_type_summary(&root, true, facts.budget(), 0, &mut 0).is_err());
        assert!(ir::validate_slice_carrier_with_facts(&root, context).is_err());
        assert_eq!(facts.budget().first_resource_failure(), first_failure);
    }
}

#[test]
fn replay_edge_inventory_has_one_paid_buffer_and_rejects_growth_shortage_and_poison() {
    for count in [0usize, 1, 2, 17, 257] {
        let bytes = count
            .checked_mul(std::mem::size_of::<topology::Edge>())
            .unwrap();
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            bytes,
            ..ir::PlanLimits::default()
        });
        let mut edges = ReplayEdges::new(count, &budget).unwrap();
        let pointer = edges.edges.as_ptr();
        let capacity = edges.edges.capacity();
        assert!(capacity >= count);
        assert_eq!(budget.usage_snapshot().bytes, bytes);
        assert_eq!(budget.usage_snapshot().reservations.retentions, 1);
        for target in 0..count {
            edges
                .push(
                    topology::Edge {
                        target,
                        by_value: target % 2 == 0,
                    },
                    &budget,
                )
                .unwrap();
            assert_eq!(edges.edges.as_ptr(), pointer);
            assert_eq!(edges.edges.capacity(), capacity);
            assert_eq!(budget.usage_snapshot().bytes, bytes);
        }
        // There is no reserve/grow path, even if the allocator gave excess
        // capacity. A bad count cannot silently reallocate or add a payload.
        assert!(
            edges
                .push(
                    topology::Edge {
                        target: count,
                        by_value: false
                    },
                    &budget
                )
                .is_err()
        );
        assert_eq!(edges.edges.as_ptr(), pointer);
        assert_eq!(edges.edges.capacity(), capacity);
        assert_eq!(edges.edges.len(), count);
        let paid = budget.usage_snapshot().bytes;
        let edges = edges.finish().unwrap();
        assert_eq!(edges.len(), count);
        assert_eq!(edges.as_ptr(), pointer);
        assert_eq!(
            budget.usage_snapshot().bytes,
            paid,
            "escaping edges are never refunded"
        );
        drop(edges);
        assert_eq!(
            budget.usage_snapshot().bytes,
            paid,
            "original conservative retained lifetime is unchanged"
        );
        if count > 0 {
            let short = ir::PlanBudget::new(ir::PlanLimits {
                bytes: bytes - 1,
                ..ir::PlanLimits::default()
            });
            assert!(ReplayEdges::new(count, &short).is_err());
            assert_eq!(
                short.first_resource_failure().unwrap().kind,
                ir::ResourceBudgetKind::Bytes
            );
            assert_eq!(short.usage_snapshot().bytes, 0);
            assert_eq!(short.usage_snapshot().reservations.retentions, 0);
            let incomplete = ReplayEdges::new(count, &ir::PlanBudget::default()).unwrap();
            assert!(incomplete.finish().is_err());
        }
        assert!(budget.charge_work(usize::MAX).is_err());
        let first = budget.first_resource_failure();
        budget.poison_ledger_for_tests();
        assert!(ReplayEdges::new(count, &budget).is_err());
        assert_eq!(budget.first_resource_failure(), first);
    }
    let budget = ir::PlanBudget::default();
    let mut warm = ReplayEdges::new(2, &budget).unwrap();
    warm.push(
        topology::Edge {
            target: 0,
            by_value: true,
        },
        &budget,
    )
    .unwrap();
    assert!(budget.charge_work(usize::MAX).is_err());
    let first = budget.first_resource_failure();
    budget.poison_ledger_for_tests();
    assert!(
        warm.push(
            topology::Edge {
                target: 1,
                by_value: false
            },
            &budget
        )
        .is_err()
    );
    assert_eq!(warm.edges.len(), 1);
    assert_eq!(budget.first_resource_failure(), first);
}

#[test]
fn dependency_replay_borrows_actuals_and_preserves_occurrences_and_limits() {
    let provider = provider(
        "struct Node<T>{next:*mut Node<T>}fn main(){}",
        &[0],
        &[Term::Bool],
    );
    let declaration = provider.plan.nominal_declaration(0).unwrap();
    let term = Term::Tuple(vec![
        Term::Nominal(
            declaration,
            vec![Term::Nominal(declaration, vec![Term::Bool])],
        ),
        Term::Ref(
            ir::BorrowKind::Shared,
            Box::new(Term::Nominal(declaration, vec![Term::Bool])),
        ),
        // Even a zero array must replay its child equation and owning flag.
        Term::Array(Box::new(Term::Nominal(declaration, vec![Term::Bool])), 0),
        Term::RawPtr(
            ir::BorrowKind::Mut,
            Box::new(Term::Nominal(declaration, vec![Term::Bool])),
        ),
        Term::Slice(Box::new(Term::Nominal(declaration, vec![Term::Bool]))),
    ]);
    let mut expected = Vec::new();
    symbolic::dependencies(&term, true, &mut expected, &ir::PlanBudget::default(), 0).unwrap();
    let budget = ir::PlanBudget::new(ir::PlanLimits {
        bytes: 0,
        ..ir::PlanLimits::default()
    });
    let mut actual = Vec::new();
    symbolic::visit_dependencies(
        &term,
        true,
        &budget,
        0,
        &mut |declaration, args, by_value| {
            actual.push((declaration, args.to_vec(), by_value));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        actual.iter().map(|entry| entry.2).collect::<Vec<_>>(),
        vec![true, false, false, true, false, false]
    );
    assert_eq!(
        budget.usage_snapshot().bytes,
        0,
        "visitor owns no copied actual payloads"
    );
    let Term::Tuple(parts) = &term else {
        unreachable!()
    };
    let Term::Nominal(_, first_actuals) = &parts[0] else {
        unreachable!()
    };
    let mut first = true;
    symbolic::visit_dependencies(&parts[0], true, &budget, 0, &mut |_, args, _| {
        if first {
            assert!(std::ptr::eq(args, first_actuals.as_slice()));
            first = false;
        }
        Ok(())
    })
    .unwrap();
    let measured = ir::PlanBudget::default();
    symbolic::visit_dependencies(&term, true, &measured, 0, &mut |_, _, _| Ok(())).unwrap();
    let work = measured.usage_snapshot().work;
    assert!(work > 1);
    for (allowance, passes) in [(work, true), (work - 1, false)] {
        let budget = ir::PlanBudget::new(ir::PlanLimits {
            work: allowance,
            ..ir::PlanLimits::default()
        });
        assert_eq!(
            symbolic::visit_dependencies(&term, true, &budget, 0, &mut |_, _, _| Ok(())).is_ok(),
            passes
        );
    }
    assert!(
        symbolic::visit_dependencies(
            &term,
            true,
            &measured,
            measured.limits().depth,
            &mut |_, _, _| Ok(())
        )
        .is_err()
    );
    let failure = measured.first_resource_failure();
    measured.poison_ledger_for_tests();
    assert!(
        symbolic::visit_dependencies(&term, true, &measured, 0, &mut |_, _, _| panic!(
            "poison must precede callback"
        ))
        .is_err()
    );
    assert_eq!(measured.first_resource_failure(), failure);
}

#[test]
fn component_impl_head_preserves_original_trait_proof_facts() {
    for workers in [1, 8] {
        let mut provider = provider(
            "struct Node{next:*mut Node}struct Wrap<T>{value:T}trait Mark{fn mark(self);}impl<T> Mark for Wrap<T>{fn mark(self){}}fn main(){}",
            &[0],
            &[],
        );
        let plan = provider.plan.clone();
        let world = plan.trait_world().unwrap();
        let node = plan.nominal_declaration(0).unwrap();
        let wrapper = plan.nominal_declaration(1).unwrap();
        provider.roots.push(
            producer::key(
                world.nominal_realization_world(),
                wrapper,
                &[Term::Nominal(node, vec![])],
                world.budget(),
            )
            .unwrap(),
        );
        // This source has exactly one trait declaration. Use its issued source
        // identity, not the registry's diagnostic display-name spelling.
        let mut markers = plan.sema_declarations().iter().filter(|declaration| {
            declaration.kind == ir::DeclKind::Trait
                && matches!(
                    declaration.path.item,
                    crate::compiler::frontend::DeclItemName::Source(_)
                )
        });
        let marker = markers.next().expect("source Mark declaration").id;
        assert!(
            markers.next().is_none(),
            "fixture has exactly one source trait"
        );
        let facts = plan
            .instance_facts(&ir::CallTarget {
                declaration: plan.function_decl(0).unwrap(),
                type_args: Arc::from([]),
            })
            .unwrap();
        let engine = crate::engine::Engine::new(provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let wrapped = reads[1].definition().unwrap();
        let goal = ir::Predicate::Implements(ir::TraitRef {
            declaration: marker,
            self_ty: wrapped.clone(),
            args: Arc::from([]),
        });
        let method = plan
            .sema_declarations()
            .iter()
            .find(|declaration| {
                declaration.kind == ir::DeclKind::TraitItem && declaration.owner == Some(marker)
            })
            .expect("source Mark item")
            .id;
        let ir::Predicate::Implements(trait_ref) = &goal else {
            unreachable!()
        };
        let call = ir::TraitCall {
            trait_ref: trait_ref.clone(),
            item: method,
            method_args: Arc::from([]),
        };
        let mut scratch = traits::TraitScratch::default();
        for _ in 0..3 {
            assert!(matches!(
                facts
                    .service()
                    .resolve_trait_call_with(&call, &facts, &mut scratch)
                    .unwrap()
                    .callee(),
                ir::Callee::Direct(_)
            ));
            assert!(matches!(
                facts
                    .service()
                    .prove_goal_with(&goal, &facts, &mut scratch)
                    .unwrap(),
                traits::ProofOutcome::Proved(_)
            ));
        }
        let ir::Ty::Struct(value) = wrapped else {
            panic!("wrapper struct")
        };
        let mut changed = value.def().clone();
        changed.fields = Vec::<ir::FieldDef>::new().into();
        let forged = ir::Predicate::Implements(ir::TraitRef {
            declaration: marker,
            self_ty: ir::Ty::Struct(
                ir::StructTy::new_parent_local(changed).with_realization(reads[1].clone()),
            ),
            args: Arc::from([]),
        });
        assert!(
            matches!(facts.service().prove_goal_with(&forged, &facts, &mut scratch), Err(error) if error.kind() == traits::FactsErrorKind::InvalidMetadata)
        );
        let ir::Predicate::Implements(forged_ref) = &forged else {
            unreachable!()
        };
        let forged_call = ir::TraitCall {
            trait_ref: forged_ref.clone(),
            ..call.clone()
        };
        assert!(
            matches!(facts.service().resolve_trait_call_with(&forged_call, &facts, &mut scratch), Err(error) if error.kind() == traits::FactsErrorKind::InvalidMetadata)
        );
        let first_failure = facts.budget().first_resource_failure();
        facts.budget().poison_ledger_for_tests();
        assert!(
            matches!(facts.service().resolve_trait_call_with(&call, &facts, &mut scratch), Err(error) if error.kind() == traits::FactsErrorKind::Resource)
        );
        assert!(
            matches!(facts.service().prove_goal_with(&goal, &facts, &mut scratch), Err(error) if error.kind() == traits::FactsErrorKind::Resource)
        );
        assert_eq!(facts.budget().first_resource_failure(), first_failure);
    }
}

#[test]
fn forged_ordered_actuals_cannot_reuse_component_member_authority() {
    let mut provider = provider(
        "struct Node<A,B>{next:*mut Node<A,B>,a:A,b:B}fn main(){}",
        &[0],
        &[Term::Int(ir::IntTy::I32), Term::Bool],
    );
    provider.forge = |fact| {
        let mut members = fact.members.to_vec();
        let mut actuals = members[0].actuals.to_vec();
        actuals.swap(0, 1);
        members[0].actuals = actuals.into();
        fact.members = members.into();
    };
    let engine = crate::engine::Engine::new(provider);
    let demand = engine.request(test_root());
    engine.run(8).unwrap();
    assert!(demand.completion().is_err());
}

#[test]
fn finite_drop_presence_finishes_every_owning_equation_before_answering() {
    for workers in [1, 8] {
        let source_provider = provider(
            "struct Link<T>{value:*mut T}enum List{End,More(Link<List>)}fn main(){}",
            &[1],
            &[],
        );
        let budget = source_provider.plan.trait_world().unwrap().budget().clone();
        let crate::compiler::frontend::PublishedCallableBody::Source(publication) = source_provider
            .plan
            .publish_callable_body(source_provider.plan.function_decl(0).unwrap(), &budget)
            .unwrap()
        else {
            panic!("source callable required");
        };
        let engine = crate::engine::Engine::new(source_provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        reads[0].check_definition(&root).unwrap();
        let argument = crate::compiler::frontend::validate_argument_allocation_for_tests;
        let caller = ir::FactsCtx::Validated(publication.facts());
        assert_eq!(argument(&root, caller, &budget, 0).unwrap(), (false, 0));
        // Scalars never allocate a memo table. Component authority checks above
        // and below still run before the same allocation-free root return.
        assert_eq!(
            argument(&ir::Ty::Bool, caller, &budget, 0).unwrap(),
            (false, 0)
        );
        // Closed callers reuse the authenticated component height.
        assert!(argument(&root, ir::FactsCtx::Concrete, &budget, 0).is_ok());
        assert!(
            !publication
                .facts()
                .normal_drop_required(&root, &budget)
                .unwrap()
        );
        let ir::Ty::Enum(value) = &root else {
            panic!("enum root required");
        };
        let mut changed = value.def().clone();
        changed.variants = Arc::from([]);
        let forged = ir::Ty::Enum(ir::EnumTy::new(changed).with_realization(reads[0].clone()));
        assert!(argument(&forged, caller, &budget, 0).is_err());
        assert!(
            publication
                .facts()
                .normal_drop_required(&forged, &budget)
                .is_err()
        );
        // Rebinding the exact same declaration/fields to the same capability
        // still forges the rooted allocation and must not warm a positive cache.
        let rebound =
            ir::Ty::Enum(ir::EnumTy::new(value.def().clone()).with_realization(reads[0].clone()));
        assert!(argument(&rebound, caller, &budget, 0).is_err());
        assert!(
            publication
                .facts()
                .normal_drop_required(&rebound, &budget)
                .is_err()
        );
        let zero = ir::Ty::Array {
            element: Arc::new(root.clone()),
            len: 0,
        };
        assert!(
            !publication
                .facts()
                .normal_drop_required(&zero, &budget)
                .unwrap()
        );
        let zero_forged = ir::Ty::Array {
            element: Arc::new(rebound),
            len: 0,
        };
        assert!(
            publication
                .facts()
                .normal_drop_required(&zero_forged, &budget)
                .is_err()
        );
        let foreign_provider = provider(
            "struct Link<T>{value:*mut T}enum List{End,More(Link<List>)}fn main(){}",
            &[1],
            &[],
        );
        let foreign_budget = foreign_provider
            .plan
            .trait_world()
            .unwrap()
            .budget()
            .clone();
        let crate::compiler::frontend::PublishedCallableBody::Source(foreign_publication) =
            foreign_provider
                .plan
                .publish_callable_body(
                    foreign_provider.plan.function_decl(0).unwrap(),
                    &foreign_budget,
                )
                .unwrap()
        else {
            panic!("source callable required");
        };
        assert!(argument(&root, caller, &foreign_budget, 0).is_err());
        assert!(
            argument(
                &root,
                ir::FactsCtx::Validated(foreign_publication.facts()),
                &foreign_budget,
                0,
            )
            .is_err()
        );
        assert!(
            foreign_publication
                .facts()
                .normal_drop_required(&root, &foreign_budget)
                .is_err()
        );
        assert!(
            publication
                .facts()
                .normal_drop_required(&root, &foreign_budget)
                .is_err()
        );
        assert!(
            publication
                .facts()
                .normal_drop_required(&zero, &foreign_budget)
                .is_err()
        );
        let mut calls = 0;
        let answer = reads[0]
            .component_drop_presence(&budget, 0, &mut |_, own_only, _| {
                assert!(own_only);
                calls += 1;
                Ok((true, false))
            })
            .unwrap()
            .unwrap();
        assert_eq!(answer, (true, false));
        assert_eq!(calls, 2);
        let mut calls = 0;
        assert!(
            reads[0]
                .component_drop_presence(&budget, 0, &mut |_, _, _| {
                    calls += 1;
                    if calls == 2 {
                        Err(Error::new("later owning equation failed"))
                    } else {
                        Ok((true, false))
                    }
                })
                .unwrap()
                .is_err()
        );
        assert_eq!(calls, 2);
        let mut calls = 0;
        let answer = reads[0]
            .component_drop_presence(&budget, 0, &mut |_, _, _| {
                calls += 1;
                Ok((calls == 1, calls == 2))
            })
            .unwrap()
            .unwrap();
        assert_eq!(answer, (true, true));
        for tiny in [
            ir::PlanBudget::new(ir::PlanLimits {
                work: 1,
                ..ir::PlanLimits::default()
            }),
            ir::PlanBudget::new(ir::PlanLimits {
                bytes: 1,
                ..ir::PlanLimits::default()
            }),
        ] {
            assert!(
                reads[0]
                    .component_drop_presence(&tiny, 0, &mut |_, _, _| Ok((true, false)))
                    .unwrap()
                    .is_err()
            );
        }
        assert!(argument(&root, caller, &budget, budget.limits().depth + 1).is_err());
        assert_eq!(
            budget.first_resource_failure().unwrap().kind,
            ir::ResourceBudgetKind::Depth,
        );
        assert!(
            reads[0]
                .component_drop_presence(&budget, usize::MAX, &mut |_, _, _| {
                    panic!("depth failure must precede callbacks")
                })
                .unwrap()
                .is_err()
        );
        // Exercise the actual public query against its own exhausted ledger,
        // rather than only passing a tiny token to the inventory callback API.
        // Exhaust bytes through the real public query before work is exhausted.
        let used_bytes = budget.usage_for_tests(|usage| usage.bytes);
        budget.usage_for_tests(|usage| usage.bytes = budget.limits().bytes);
        assert!(
            publication
                .facts()
                .normal_drop_required(&root, &budget)
                .is_err()
        );
        assert!(
            publication
                .facts()
                .normal_drop_required(&zero, &budget)
                .is_err()
        );
        budget.usage_for_tests(|usage| usage.bytes = used_bytes);
        let limit = budget.limits().work;
        budget.usage_for_tests(|usage| usage.work = limit);
        assert!(
            publication
                .facts()
                .normal_drop_required(&root, &budget)
                .is_err()
        );
        assert!(
            publication
                .facts()
                .normal_drop_required(&zero, &budget)
                .is_err()
        );
        let failure = budget.first_resource_failure();
        budget.poison_ledger_for_tests();
        assert!(argument(&root, caller, &budget, 0).is_err());
        assert!(
            publication
                .facts()
                .normal_drop_required(&root, &budget)
                .is_err()
        );
        assert!(
            publication
                .facts()
                .normal_drop_required(&zero, &budget)
                .is_err()
        );
        assert_eq!(budget.first_resource_failure(), failure);

        // The zero is inside the component equation, not merely around its
        // public root. Its child must run in validation-only mode.
        let zero_provider = provider(
            "struct Leaf<T>{value:*mut T}struct Link<T>{value:[Leaf<T>;0]}enum List{End,More(Link<List>)}fn main(){}",
            &[2],
            &[],
        );
        let zero_budget = zero_provider.plan.trait_world().unwrap().budget().clone();
        let crate::compiler::frontend::PublishedCallableBody::Source(zero_publication) =
            zero_provider
                .plan
                .publish_callable_body(zero_provider.plan.function_decl(0).unwrap(), &zero_budget)
                .unwrap()
        else {
            panic!("source callable required");
        };
        let zero_engine = crate::engine::Engine::new(zero_provider);
        let zero_demand = zero_engine.request(test_root());
        zero_engine.run(workers).unwrap();
        zero_demand.completion().unwrap();
        let zero_reads = zero_demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let zero_root = zero_reads[0].definition().unwrap();
        assert!(
            !zero_publication
                .facts()
                .normal_drop_required(&zero_root, &zero_budget)
                .unwrap()
        );
        let mut calls = 0;
        let answer = zero_reads[0]
            .component_drop_presence(&zero_budget, 0, &mut |_, own_only, _| {
                assert!(own_only);
                calls += 1;
                // Leaf is reached only through the zero edge. Its own Drop
                // result cannot contribute, but its unresolved status must.
                Ok((calls > 2, calls > 2))
            })
            .unwrap()
            .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(answer, (false, true));
        let mut calls = 0;
        assert!(
            zero_reads[0]
                .component_drop_presence(&zero_budget, 0, &mut |_, _, _| {
                    calls += 1;
                    if calls > 2 {
                        Err(Error::new("zero-array child validation failed"))
                    } else {
                        Ok((false, false))
                    }
                },)
                .unwrap()
                .is_err()
        );
        assert_eq!(calls, 3);
    }
}

#[test]
fn finite_drop_presence_follows_rc_pointee_backedge_without_borrowing_pending_success() {
    for workers in [1, 8] {
        let source_provider = provider(
            "struct Node{next:__justrust_std::rc::Rc<Node>}mod __justrust_std{pub mod rc{pub struct Rc<T>{pub ptr:*mut T,pub strong:*mut usize}}}fn main(){}",
            &[0],
            &[],
        );
        let world = source_provider.plan.trait_world().unwrap();
        let budget = world.budget().clone();
        let crate::compiler::frontend::PublishedCallableBody::Source(publication) = source_provider
            .plan
            .publish_callable_body(source_provider.plan.function_decl(0).unwrap(), &budget)
            .unwrap()
        else {
            panic!("source callable required");
        };
        let engine = crate::engine::Engine::new(source_provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        assert!(reads[0].is_component());
        reads[0].replay(&root, &world).unwrap();
        assert!(
            publication
                .facts()
                .normal_drop_required(&root, &budget)
                .unwrap()
        );
        let mut ordinary = 0;
        let mut rc = 0;
        let answer = reads[0]
            .component_drop_presence(&budget, 0, &mut |ty, own_only, _| {
                assert!(own_only);
                if matches!(
                    ty.owned_pointer_pointee(),
                    Some((ir::OwnedPointerKind::Rc, _))
                ) {
                    rc += 1;
                    Ok((true, false))
                } else {
                    ordinary += 1;
                    Ok((false, false))
                }
            })
            .unwrap()
            .unwrap();
        assert_eq!(answer, (true, false));
        assert_eq!((ordinary, rc), (1, 1));
    }
}

#[test]
fn component_module_visibility_visits_source_authorities_without_type_materialization() {
    for workers in [1, 8] {
        for source in [
            "mod m{pub struct Node{pub next:*mut Node,hidden:bool}}fn main(){}",
            "mod m{pub enum Node{Next{next:*mut Node},End}}fn main(){}",
        ] {
            let (source_provider, modules) = provider_with_modules(source, &[0], &[]);
            let world = source_provider.plan.trait_world().unwrap();
            let budget = world.budget().clone();
            let engine = crate::engine::Engine::new(source_provider);
            let demand = engine.request(test_root());
            engine.run(workers).unwrap();
            demand.completion().unwrap();
            let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
            let root = reads[0].definition().unwrap();
            reads[0].replay(&root, &world).unwrap();
            let mut expected = Vec::new();
            match &root {
                ir::Ty::Struct(value) => {
                    expected.push((value.def().module, value.def().visibility.clone()));
                    for field in value.def().fields.read(&budget).unwrap().iter() {
                        expected.push((value.def().module, field.visibility.clone()));
                    }
                }
                ir::Ty::Enum(value) => {
                    expected.push((value.def().module, value.def().visibility.clone()));
                    for variant in value.def().variants.iter() {
                        for field in variant.fields.read(&budget).unwrap().iter() {
                            expected.push((value.def().module, field.visibility.clone()));
                        }
                    }
                }
                _ => unreachable!(),
            }
            let before = budget.usage_snapshot().bytes;
            for _ in 0..64 {
                let mut actual = Vec::new();
                reads[0]
                    .component_visit_source_module_visibility(
                        &root,
                        &budget,
                        &mut |module, visibility| {
                            visibility.validate_for(&modules, module).unwrap();
                            actual.push((module, visibility.clone()));
                            Ok(())
                        },
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(actual, expected);
            }
            assert_eq!(budget.usage_snapshot().bytes, before);
            let (_, foreign_modules) = provider_with_modules(source, &[0], &[]);
            assert!(
                reads[0]
                    .component_visit_source_module_visibility(
                        &root,
                        &budget,
                        &mut |module, visibility| {
                            visibility
                                .validate_for(&foreign_modules, module)
                                .map_err(|error| Error::new(error.to_string()))
                        }
                    )
                    .unwrap()
                    .is_err()
            );
            // Same display/source identities on another definition allocation
            // cannot borrow this component's authenticated source authorities.
            let forged = match &root {
                ir::Ty::Struct(value) => ir::Ty::Struct(
                    ir::StructTy::new(value.def().clone()).with_realization(reads[0].clone()),
                ),
                ir::Ty::Enum(value) => ir::Ty::Enum(
                    ir::EnumTy::new(value.def().clone()).with_realization(reads[0].clone()),
                ),
                _ => unreachable!(),
            };
            let mut called = false;
            assert!(
                reads[0]
                    .component_visit_source_module_visibility(&forged, &budget, &mut |_, _| {
                        called = true;
                        Ok(())
                    })
                    .unwrap()
                    .is_err()
            );
            assert!(!called);
        }
    }
}

#[test]
fn finite_shared_assignment_payload_handles_recursive_actuals_and_carrier_controls() {
    for workers in [1, 8] {
        for (marker, expected) in [
            ("bool", true),
            ("&'static bool", true),
            ("&'static mut bool", false),
            ("&'static [bool]", false),
            ("(&'static bool,&'static mut bool)", false),
            ("*mut &'static mut bool", true),
        ] {
            let source = format!(
                "struct Holder{{link:Link<Holder>,marker:{marker}}}struct Link<T>{{ptr:*mut T}}fn main(){{}}"
            );
            let source_provider = provider(&source, &[0], &[]);
            let world = source_provider.plan.trait_world().unwrap();
            let budget = world.budget().clone();
            let engine = crate::engine::Engine::new(source_provider);
            let demand = engine.request(test_root());
            engine.run(workers).unwrap();
            demand.completion().unwrap();
            let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
            let root = reads[0].definition().unwrap();
            assert!(reads[0].is_component());
            reads[0].replay(&root, &world).unwrap();
            let mut external = |_: &ir::Ty, _: usize| -> Result<bool, Error> {
                panic!("all nominal edges belong to this component")
            };
            assert_eq!(
                reads[0]
                    .component_shared_assignment_payload(&budget, 0, &mut external)
                    .unwrap()
                    .unwrap(),
                expected,
                "{marker}"
            );
            assert_eq!(
                world.shared_assignment_payload_for_tests(&root).unwrap(),
                expected
            );
            let ir::Ty::Struct(value) = &root else {
                panic!("struct root")
            };
            // Removing only the root read handle retains the immutable symbolic
            // fields and their ordinary nominal back edges. The finite query
            // must not need a read handle on every recursive occurrence.
            let ordinary = ir::Ty::Struct(ir::StructTy::new_parent_local(value.def().clone()));
            assert_eq!(
                world
                    .shared_assignment_payload_for_tests(&ordinary)
                    .unwrap(),
                expected
            );
            if expected {
                let mut changed = value.def().clone();
                let mut fields = changed.fields.read(&budget).unwrap().to_vec();
                fields.push(ir::FieldDef {
                    name: "not_a_shared_payload".into(),
                    ty: ir::Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        referent: Arc::new(ir::Ty::Bool),
                    },
                    visibility: value.def().visibility.clone(),
                    lifetime_bound: true,
                });
                changed.fields = fields.into();
                let forged = ir::Ty::Struct(ir::StructTy::new_parent_local(changed));
                let siblings = ir::Ty::Tuple(Arc::from([ordinary.clone(), forged]));
                assert!(
                    !world
                        .shared_assignment_payload_for_tests(&siblings)
                        .unwrap()
                );
            }
            let foreign = provider(&source, &[0], &[]);
            let foreign_world = foreign.plan.trait_world().unwrap();
            assert!(
                foreign_world
                    .shared_assignment_payload_for_tests(&root)
                    .is_err()
            );
            assert!(world.budget().first_resource_failure().is_none());
            let mut deep = ir::Ty::Bool;
            for _ in 0..=budget.limits().depth {
                deep = ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(deep),
                };
            }
            assert!(world.shared_assignment_payload_for_tests(&deep).is_err());
            let first_failure = budget.first_resource_failure();
            assert!(first_failure.is_some());
            assert!(
                world
                    .shared_assignment_payload_for_tests(&ordinary)
                    .is_err()
            );
            assert_eq!(budget.first_resource_failure(), first_failure);
        }
    }
}

#[test]
fn finite_ownership_preflight_keeps_equations_array_work_and_exact_root_authority() {
    use crate::compiler::ownership::component_preflight_for_tests as preflight;
    for workers in [1, 8] {
        let source_provider = provider(
            "struct Link<T>{value:*mut T}enum List{End,More(Link<List>)}fn main(){}",
            &[1],
            &[],
        );
        let budget = source_provider.plan.trait_world().unwrap().budget().clone();
        let engine = crate::engine::Engine::new(source_provider);
        let demand = engine.request(test_root());
        engine.run(workers).unwrap();
        demand.completion().unwrap();
        let reads = demand.fact::<Vec<ReadNominal>>(0).unwrap();
        let root = reads[0].definition().unwrap();
        assert!(reads[0].is_component());
        let exact = preflight(&root, Some(&budget), 1_000_000, 0).unwrap();
        assert!(exact > 1);
        assert_eq!(preflight(&root, Some(&budget), exact, 0).unwrap(), exact);
        assert!(preflight(&root, Some(&budget), exact - 1, 0).is_err());
        assert!(preflight(&root, None, 1_000_000, 0).is_err());
        assert!(preflight(&root, Some(&budget), 1_000_000, 129).is_err());
        let ir::Ty::Enum(value) = &root else {
            panic!("enum root")
        };
        let mut changed = value.def().clone();
        changed.variants = Arc::from([]);
        let forged = ir::Ty::Enum(ir::EnumTy::new(changed).with_realization(reads[0].clone()));
        assert!(preflight(&forged, Some(&budget), 1_000_000, 0).is_err());

        // Check the finite term policy against the ordinary ownership walker,
        // including repeated shared array elements and zero-length validation.
        let scalar_provider = provider("struct Node{next:*mut Node}fn main(){}", &[0], &[]);
        let scalar_world = scalar_provider.plan.trait_world().unwrap();
        let scalar_key = ComponentKey::encode(
            scalar_world.nominal_realization_world(),
            &scalar_provider.roots,
            scalar_world.budget(),
        )
        .unwrap();
        let scalar_engine = crate::engine::Engine::new(scalar_provider);
        let scalar_demand = scalar_engine.request(test_root());
        scalar_engine.run(workers).unwrap();
        scalar_demand.completion().unwrap();
        let component = ReadComponent {
            fact: scalar_engine
                .fact_of::<ComponentFact>(&scalar_key.0, 0)
                .unwrap(),
            cells: Arc::from(Vec::new()),
        };
        for len in [0, 1, 3, ir::MAX_ARRAY_LEN + 1] {
            let term = Term::Array(Box::new(Term::Bool), len);
            let ordinary = ir::Ty::Array {
                element: Arc::new(ir::Ty::Bool),
                len,
            };
            let expected = preflight(&ordinary, None, 1_000_000, 0).unwrap();
            let mut spent = 0usize;
            let mut event = |weight: usize, external: Option<(&ir::Ty, usize)>| {
                assert!(external.is_none());
                spent = spent.checked_add(weight).unwrap();
                Ok(())
            };
            let read = &component;
            let mut inventory = OwnershipInventory {
                read,
                budget: &budget,
                event: &mut event,
                queued: Vec::new(),
                pending: Vec::new(),
            };
            inventory.term(&term, 0).unwrap();
            assert_eq!(spent, expected);
        }
        let nested = Term::Array(
            Box::new(Term::Tuple(vec![Term::Bool])),
            ir::MAX_ARRAY_LEN + 1,
        );
        let mut event = |_: usize, _: Option<(&ir::Ty, usize)>| Ok(());
        let mut inventory = OwnershipInventory {
            read: &component,
            budget: &budget,
            event: &mut event,
            queued: Vec::new(),
            pending: Vec::new(),
        };
        assert!(inventory.term(&nested, 0).is_err());
        let mut deep = Term::Bool;
        for _ in 0..130 {
            deep = Term::Ref(ir::BorrowKind::Shared, Box::new(deep));
        }
        // Raw storage is an ownership leaf even for deeply nested pointee metadata.
        assert!(
            inventory
                .term(
                    &Term::RawPtr(ir::BorrowKind::Mut, Box::new(deep.clone())),
                    0
                )
                .is_ok()
        );
        let fresh = ir::PlanBudget::default();
        let mut depth_inventory = OwnershipInventory {
            read: &component,
            budget: &fresh,
            event: &mut event,
            queued: Vec::new(),
            pending: Vec::new(),
        };
        assert!(
            depth_inventory
                .term(&Term::Array(Box::new(deep), 0), 0)
                .is_err()
        );
        // Rejected reservation alone is not sticky poison. Exercise actual
        // poison and verify the original failure evidence survives a warm read.
        assert!(budget.charge_work(budget.limits().work + 1).is_err());
        let first = budget.first_resource_failure();
        budget.poison_ledger_for_tests();
        assert!(preflight(&root, Some(&budget), 1_000_000, 0).is_err());
        assert_eq!(budget.first_resource_failure(), first);
    }
}
