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
    let program = super::super::super::frontend::parse(source).unwrap();
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
    Provider {
        plan,
        roots,
        forge: |_| {},
    }
}
fn test_root() -> Key {
    crate::compiler::keys::params(996, 0, vec![0])
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
        };
        let cases = [
            (Term::Bool, ir::Ty::Bool, false),
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
        let first_failure = facts.budget().first_resource_failure();
        facts.budget().poison_ledger_for_tests();
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
