use super::*;
use crate::compiler::memory_layout::domain::tests::{fixture_locals, fixture_plan};
use crate::compiler::{frontend, memory_layout::domain::Local};

#[test]
fn semantic_size_domain_bounds_and_source_transfer_shapes_are_sealed() {
    let (locals, root, budget, plan) = fixture_plan(
        "fn size<T>()->usize{std::mem::size_of::<[T;3]>()}fn main(){}",
        "size",
    );
    let summary = source_summary(&plan, root, &budget);
    let required = summary.requirement(root).unwrap();
    assert_eq!(required.parameters, [false]);
    assert_eq!(
        required.semantic_limits,
        [Some(super::super::SEMANTIC_SIZE_LIMIT / 3)]
    );
    assert!(!required.deferred_dispatch);
    let mut forged = summary.as_ref().clone();
    Arc::make_mut(&mut forged.members)
        .iter_mut()
        .find(|(owner, _)| *owner == root)
        .unwrap()
        .1
        .semantic_limits[0] = None;
    assert!(forged.admit_symbolic(root).is_err());
    let caller = locals.iter().find(|local| local.owner == root).unwrap();
    let mut forged_local = caller.clone();
    let mut edges = forged_local.edges.to_vec();
    edges[0].arguments[0].2.arrays = Arc::from([2usize]);
    forged_local.edges = edges.into();
    assert!(forged_local.validate(&budget).is_err());
    let headers = headers(locals, &budget);
    let mut forged_header = headers
        .iter()
        .find(|header| header.owner == root)
        .unwrap()
        .as_ref()
        .clone();
    let transfers = Arc::make_mut(&mut forged_header.transfers);
    let arguments = Arc::make_mut(&mut transfers[0].arguments);
    arguments[0].2.arrays = Arc::from([2usize]);
    assert!(forged_header.validate_receipt().is_err());
}

#[test]
fn semantic_size_generic_reference_retains_metadata_shape_requirement() {
    let (locals, root, budget, plan) = fixture_plan(
        "fn width<T:?Sized>()->usize{std::mem::size_of::<&T>()}fn main(){}",
        "width",
    );
    let summary = source_summary(&plan, root, &budget);
    let required = summary.requirement(root).unwrap();
    assert_eq!(required.metadata_pointees, [true]);
    assert_eq!(required.thin_pointees, [false]);
    let mut local = locals
        .iter()
        .find(|local| local.owner == root)
        .unwrap()
        .clone();
    let mut edges = local.edges.to_vec();
    edges[0].arguments[0].6.deferred_dispatch = true;
    local.edges = edges.into();
    assert!(local.validate(&budget).is_err());
    let mut forged = summary.as_ref().clone();
    Arc::make_mut(&mut forged.members)
        .iter_mut()
        .find(|(owner, _)| *owner == root)
        .unwrap()
        .1
        .metadata_pointees[0] = false;
    assert!(forged.admit_symbolic(root).is_err());
    for (ty, deferred) in [
        (ir::Ty::Int(ir::IntTy::U8), false),
        (ir::Ty::Str, false),
        (ir::Ty::Tuple(Arc::from([ir::Ty::Int(ir::IntTy::U8)])), true),
    ] {
        let target = ir::CallTarget {
            declaration: root,
            type_args: Arc::from([ty]),
        };
        let facts = plan
            .callable_facts(root)
            .unwrap()
            .world()
            .instance_facts(&target)
            .unwrap();
        assert_eq!(
            summary
                .closed_requirements(&target, &facts, &budget)
                .unwrap(),
            deferred
        );
    }
}

#[test]
fn semantic_size_canonical_pointer_summary_and_array_transfer_are_sealed() {
    let cap = crate::compiler::backend::MAX_FRAME_BYTES as usize;
    // Select the real embedded declaration, not a user call through privacy.
    let (locals, owner, budget, mut plan) = fixture_plan("fn main(){}", "pointer_stride");
    let local = locals.iter().find(|local| local.owner == owner).unwrap();
    let publication = local.publication.as_ref().unwrap();
    publication
        .validate_source(&local.source, &local.facts, &budget)
        .unwrap();
    let query = &local.queries.first().unwrap().1;
    assert!(query.is_supported_pointer_stride());
    query
        .validate(ir::FactsCtx::Validated(&local.facts), &budget)
        .unwrap();
    let lowered = crate::compiler::cfg::lower_source_template(publication, &budget).unwrap();
    let template = crate::compiler::ownership::verify_template(lowered).unwrap();
    let summary = source_summary(&plan, owner, &budget);
    assert_eq!(summary.requirement(owner).unwrap().pointer_strides, [true]);
    let array = |element, len| ir::Ty::Array {
        element: Arc::new(element),
        len,
    };
    for (actual, expected) in [
        (array(ir::Ty::Int(ir::IntTy::U8), cap), Ok(cap)),
        (array(ir::Ty::Unit, ir::MAX_ARRAY_TYPE_LEN), Ok(0)),
        (
            array(ir::Ty::Int(ir::IntTy::U8), cap + 1),
            Err("array element extent exceeds native memory limit"),
        ),
        (
            array(array(ir::Ty::Int(ir::IntTy::U8), cap + 1), 0),
            Err("array element extent exceeds native memory limit"),
        ),
    ] {
        let target = ir::CallTarget {
            declaration: owner,
            type_args: Arc::from([actual.clone()]),
        };
        let facts = plan.instance_facts(&target).unwrap();
        let instantiated = ir::instantiate_checked(
            &template,
            std::slice::from_ref(&actual),
            &facts,
            &mut plan,
            &budget,
        )
        .unwrap();
        let closed_query = instantiated
            .exprs
            .iter()
            .find_map(|expression| match &expression.kind {
                ir::ExprKind::TypeAlignment(query) => Some(query),
                _ => None,
            })
            .expect("source-owned query survives exact verified substitution");
        closed_query
            .validate(ir::FactsCtx::Validated(&facts), &budget)
            .unwrap();
        let admitted = summary.closed_requirements(&target, &facts, &budget);
        let value = crate::compiler::memory_layout::query_value(
            closed_query,
            ir::FactsCtx::Validated(&facts),
            &budget,
        );
        match expected {
            Ok(bytes) => {
                assert!(!admitted.unwrap());
                assert_eq!(value.unwrap(), Some(bytes));
            }
            Err(message) => {
                assert!(admitted.unwrap_err().contains(message));
                assert!(value.unwrap_err().contains(message));
            }
        }
    }

    // Canonical metadata rejects these before source substitution or backend
    // count conversion. Even an outer zero cannot erase an invalid inner type.
    for count in [ir::MAX_ARRAY_TYPE_LEN + 1, i32::MAX as usize + 1] {
        for actual in [
            array(ir::Ty::Unit, count),
            array(array(ir::Ty::Unit, count), 0),
        ] {
            let target = ir::CallTarget {
                declaration: owner,
                type_args: Arc::from([actual]),
            };
            let error = local
                .facts
                .world()
                .instance_facts(&target)
                .expect_err("metadata count limit must reject canonical closed facts");
            assert_eq!(error.kind(), traits::FactsErrorKind::Resource);
            assert!(
                error
                    .into_legacy_diagnostic(&budget)
                    .contains("array metadata length limit exceeded")
            );
        }
    }

    // The public null<[Q;3]> source call contributes a real open argument
    // descriptor. It is sealed even when null itself does not consume stride.
    let (locals, root, budget, _) = fixture_plan(
        "fn open<Q>(){let _p=std::ptr::null::<[Q;3]>();}fn main(){}",
        "open",
    );
    let local = locals.iter().find(|local| local.owner == root).unwrap();
    local.validate(&budget).unwrap();
    let (edge_index, argument_index) = local
        .edges
        .iter()
        .enumerate()
        .find_map(|(edge_index, edge)| {
            edge.arguments
                .iter()
                .position(|argument| argument.4.physical_limits == [Some(cap / 3)])
                .map(|argument_index| (edge_index, argument_index))
        })
        .expect("genuine source array argument retains physical cap/3");
    let header = Header::capture(
        local.source.clone(),
        local.publication.clone(),
        local.facts.clone(),
        &budget,
    )
    .unwrap();
    header.validate_receipt().unwrap();
    for replacement in [None, Some(cap / 3 + 1)] {
        let mut forged_local = local.clone();
        let mut edges = forged_local.edges.to_vec();
        edges[edge_index].arguments[argument_index]
            .4
            .physical_limits[0] = replacement;
        forged_local.edges = edges.into();
        assert!(forged_local.validate(&budget).is_err());
        let mut forged_header = header.clone();
        let transfers = Arc::make_mut(&mut forged_header.transfers);
        let arguments = Arc::make_mut(&mut transfers[edge_index].arguments);
        arguments[argument_index].4.physical_limits[0] = replacement;
        assert!(forged_header.validate_receipt().is_err());
    }
}

#[test]
fn semantic_size_growing_arrays_have_finite_scc_requirements() {
    let (_, root, budget, plan) = fixture_plan(
        "fn size<T>()->usize{if false{size::<[T;2]>()}else{std::mem::size_of::<T>()}}fn main(){}",
        "size",
    );
    let summary = source_summary(&plan, root, &budget);
    let required = summary.requirement(root).unwrap();
    assert_eq!(required.semantic_limits, [Some(0)]);
    assert!(!required.deferred_dispatch);
    for (ty, supported) in [(ir::Ty::Unit, true), (ir::Ty::Int(ir::IntTy::U8), false)] {
        let target = ir::CallTarget {
            declaration: root,
            type_args: Arc::from([ty]),
        };
        let facts = plan
            .callable_facts(root)
            .unwrap()
            .world()
            .instance_facts(&target)
            .unwrap();
        assert_eq!(
            summary
                .closed_requirements(&target, &facts, &budget)
                .is_ok(),
            supported
        );
    }
}

#[test]
fn semantic_size_argument_walk_retains_per_task_depth_work_and_byte_limits() {
    use super::super::{Requirement, SizeArgument};
    let ty = ir::Ty::Array {
        element: Arc::new(ir::Ty::Array {
            element: Arc::new(ir::Ty::Int(ir::IntTy::U16)),
            len: 3,
        }),
        len: 2,
    };
    for limits in [
        ir::PlanLimits {
            depth: 0,
            ..Default::default()
        },
        ir::PlanLimits {
            work: 0,
            ..Default::default()
        },
        ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        },
    ] {
        assert!(SizeArgument::capture(&ty, &[], &ir::PlanBudget::new(limits)).is_err());
    }
    let budget = ir::PlanBudget::default();
    let shape = SizeArgument::capture(&ty, &[], &budget).unwrap();
    let mut requirement = Requirement::empty(0, &budget).unwrap();
    shape.apply(12, &mut requirement, &budget).unwrap();
    requirement.admit().unwrap();
    shape.apply(11, &mut requirement, &budget).unwrap();
    assert!(requirement.admit().is_err());
}

fn headers(locals: Vec<Local>, budget: &ir::PlanBudget) -> Vec<Arc<Header>> {
    locals
        .into_iter()
        .map(|local| {
            Arc::new(Header::capture(local.source, local.publication, local.facts, budget).unwrap())
        })
        .collect()
}
fn reachable(root: ir::DeclId, headers: &[Arc<Header>], budget: &ir::PlanBudget) -> Vec<Adjacency> {
    let by_owner: HashMap<_, _> = headers
        .iter()
        .map(|header| (header.owner(), header))
        .collect();
    let mut pending = vec![root];
    let mut seen = HashSet::new();
    let mut graph = Vec::new();
    while let Some(owner) = pending.pop() {
        if !seen.insert(owner) {
            continue;
        }
        let vertex = by_owner[&owner].topology(budget).unwrap();
        pending.extend(vertex.callees());
        graph.push(vertex);
    }
    graph
}
fn summaries(
    headers: &[Arc<Header>],
    budget: &ir::PlanBudget,
) -> HashMap<ComponentId, Arc<Summary>> {
    let mut pending: Vec<_> = headers
        .iter()
        .map(|header| {
            Topology::discover(
                header.owner(),
                reachable(header.owner(), headers, budget),
                budget,
            )
            .unwrap()
        })
        .collect();
    let mut result: HashMap<ComponentId, Arc<Summary>> = HashMap::new();
    while !pending.is_empty() {
        let before = pending.len();
        let mut deferred = Vec::new();
        for topology in pending {
            if result.contains_key(topology.component()) {
                continue;
            }
            if !topology.outgoing().iter().all(|id| result.contains_key(id)) {
                deferred.push(topology);
                continue;
            }
            let members = headers
                .iter()
                .filter(|header| topology.component().members().contains(&header.owner()))
                .cloned()
                .collect();
            let external = topology
                .outgoing()
                .iter()
                .map(|id| result[id].clone())
                .collect();
            let summary = Arc::new(Summary::solve(&topology, members, external, budget).unwrap());
            result.insert(topology.component().clone(), summary);
        }
        assert!(
            deferred.len() < before,
            "SCC summary dependency cycle escaped topology"
        );
        pending = deferred;
    }
    result
}
pub(in crate::compiler) fn source_summary(
    plan: &frontend::PlanCtx,
    owner: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Arc<Summary> {
    let headers = headers(fixture_locals(plan, owner, budget), budget);
    let topology = Topology::discover(owner, reachable(owner, &headers, budget), budget).unwrap();
    summaries(&headers, budget)[topology.component()].clone()
}
pub(in crate::compiler) fn source_requirement(
    plan: &frontend::PlanCtx,
    owner: ir::DeclId,
    budget: &ir::PlanBudget,
) -> (Vec<bool>, Vec<bool>) {
    let summary = source_summary(plan, owner, budget);
    let requirement = summary.requirement(owner).unwrap();
    (
        requirement.parameters.clone(),
        requirement.raw_pointees.clone(),
    )
}

pub(in crate::compiler) fn assert_propagated_raw_requirement_is_sealed(
    plan: &frontend::PlanCtx,
    owner: ir::DeclId,
    budget: &ir::PlanBudget,
) {
    let mut forged = source_summary(plan, owner, budget).as_ref().clone();
    let required = &mut Arc::make_mut(&mut forged.members)
        .iter_mut()
        .find(|(member, _)| *member == owner)
        .unwrap()
        .1;
    assert_eq!(required.raw_pointees, [true]);
    required.raw_pointees[0] = false;
    assert!(forged.admit_symbolic(owner).is_err());
}

pub(in crate::compiler) fn closed_engine_fixture(
    source_text: &str,
    name: &str,
    type_args: Arc<[ir::Ty]>,
) -> (
    crate::engine::Engine,
    ir::PlanBudget,
    Arc<frontend::PlanCtx>,
    ir::CallTarget,
) {
    use crate::compiler::{CompilerProvider, Input, Snapshot, keys, source};
    use crate::engine::ItemKey;
    let budget = ir::PlanBudget::default();
    let snapshot = Arc::new(Snapshot {
        input: Input::Text(source_text.into()),
        source_options: source::SourceOptions::default(),
        budget: budget.clone(),
    });
    let engine = crate::engine::Engine::new(CompilerProvider(snapshot));
    let check = engine.request(keys::CheckRoot.key());
    let report = engine.run(1).unwrap();
    assert_eq!(report.failures, 0, "{:?}", report.node_errors);
    check.completion().unwrap();
    let plan = engine
        .fact_of::<frontend::PlanCtx>(&keys::SignaturesDebtRoot.key(), 0)
        .unwrap();
    let declaration = (0..plan.callable_count()).map(|index| plan.callable_decl(index).unwrap())
        .find(|id| matches!(plan.persistent_decl_path(*id).unwrap().item(), frontend::PersistentDeclItem::Name(found) if found == name)).unwrap();
    let root = ir::CallTarget {
        declaration,
        type_args,
    };
    let topology_key = keys::closed_domain::ClosedDomain::topology(&root, &budget).unwrap();
    // CHECK has already demanded this exact closed root and its SCC summary.
    // Engine execution is single-use: inspect published facts without new demands.
    let topology = engine
        .fact_of::<closed::topology::Topology>(&topology_key.key(), 0)
        .unwrap();
    let summary_key =
        keys::closed_domain::ClosedDomain::Summary(topology.component().words().to_vec());
    engine
        .fact_of::<closed::topology::Summary>(&summary_key.key(), 0)
        .unwrap();
    (engine, budget, plan, root)
}

pub(in crate::compiler) fn closed_header(
    plan: &mut frontend::PlanCtx,
    target: ir::CallTarget,
    budget: &ir::PlanBudget,
) -> Arc<closed::Header> {
    let summary = source_summary(plan, target.declaration, budget);
    let facts = plan.instance_facts(&target).unwrap();
    if !summary
        .closed_requirements(&target, &facts, budget)
        .unwrap()
    {
        return Arc::new(
            closed::Header::no_continuation(target, &summary, &facts, budget).unwrap(),
        );
    }
    let locals = fixture_locals(plan, target.declaration, budget);
    let local = locals
        .iter()
        .find(|local| local.owner == target.declaration)
        .unwrap();
    let calls = local
        .closed_calls(&target, facts, plan, budget, None)
        .unwrap();
    calls.validate(plan, budget, None).unwrap();
    let owners: Vec<_> = calls.provider_declarations().collect();
    let summaries = owners
        .into_iter()
        .map(|owner| source_summary(plan, owner, budget))
        .collect();
    Arc::new(closed::Header::bind(calls, summaries, budget).unwrap())
}

#[test]
fn alignment_domain_compact_growing_recursion_has_one_finite_scc() {
    let (locals, owner, budget, _) = fixture_plan(
        "fn first<T>(n:usize)->usize{if n==0usize{std::mem::align_of::<T>()}else{second::<[T;2]>(n-1usize)}}fn second<T>(n:usize)->usize{first::<T>(n)}fn main(){}",
        "first",
    );
    let headers = headers(locals, &budget);
    let topology = Topology::discover(owner, reachable(owner, &headers, &budget), &budget).unwrap();
    assert_eq!(topology.component().members().len(), 2);
    assert!(!topology.outgoing().is_empty());
    let solved = summaries(&headers, &budget);
    let members: Vec<_> = headers
        .iter()
        .filter(|header| topology.component().members().contains(&header.owner()))
        .cloned()
        .collect();
    let external: Vec<_> = topology
        .outgoing()
        .iter()
        .map(|id| solved[id].clone())
        .collect();
    assert!(Summary::solve(&topology, members.clone(), vec![], &budget).is_err());
    let summary = Summary::solve(&topology, members.clone(), external.clone(), &budget).unwrap();
    assert_eq!(summary.requirement(owner).unwrap().parameters, [true]);
    assert_eq!(summary.component(), topology.component());
    let mut erased_bits = summary.clone();
    let sealed_members = Arc::make_mut(&mut erased_bits.members);
    sealed_members
        .iter_mut()
        .find(|(member, _)| *member == owner)
        .unwrap()
        .1
        .parameters[0] = false;
    assert!(erased_bits.admit_symbolic(owner).is_err());
    let mut forged_header = headers
        .iter()
        .find(|header| header.owner == owner)
        .unwrap()
        .as_ref()
        .clone();
    forged_header.direct.parameters[0] = !forged_header.direct.parameters[0];
    assert!(forged_header.topology(&budget).is_err());
    assert!(Summary::solve(&topology, vec![members[0].clone()], external, &budget).is_err());
    let denied = ir::PlanBudget::new(ir::PlanLimits {
        work: 0,
        ..Default::default()
    });
    assert!(Topology::discover(owner, reachable(owner, &headers, &budget), &denied).is_err());
    let no_storage = ir::PlanBudget::new(ir::PlanLimits {
        bytes: 0,
        ..Default::default()
    });
    assert!(Topology::discover(owner, reachable(owner, &headers, &budget), &no_storage).is_err());
}

#[test]
fn alignment_domain_compact_requires_exact_external_summary_and_transfer() {
    let (locals, owner, budget, _) = fixture_plan(
        "fn leaf<T>()->usize{std::mem::align_of::<T>()}fn caller<T>()->usize{leaf::<[T;3]>()}fn main(){}",
        "caller",
    );
    let headers = headers(locals, &budget);
    let topology = Topology::discover(owner, reachable(owner, &headers, &budget), &budget).unwrap();
    assert_eq!(topology.component().members(), &[owner]);
    assert_eq!(topology.outgoing().len(), 1);
    let solved = summaries(&headers, &budget);
    let members: Vec<_> = headers
        .iter()
        .filter(|header| header.owner == owner)
        .cloned()
        .collect();
    assert!(Summary::solve(&topology, members.clone(), vec![], &budget).is_err());
    let external = vec![solved[&topology.outgoing()[0]].clone()];
    let authentic = Summary::solve(&topology, members.clone(), external.clone(), &budget).unwrap();
    assert_eq!(authentic.requirement(owner).unwrap().parameters, [true]);
    let mut forged = members[0].as_ref().clone();
    let mut transfers = forged.transfers.to_vec();
    transfers[0].arguments = Arc::from([]);
    forged.transfers = transfers.into();
    assert!(Summary::solve(&topology, vec![Arc::new(forged)], external, &budget).is_err());
    let mut missing = reachable(owner, &headers, &budget);
    missing.pop();
    assert!(Topology::discover(owner, missing, &budget).is_err());
}

#[test]
fn alignment_domain_compact_closed_trait_cycle_requires_every_owner() {
    let (_, owner, budget, mut plan) = fixture_plan(
        "trait Step{fn step(self)->usize;}impl Step for u16{fn step(self)->usize{if self==0u16{std::mem::align_of::<u16>()}else{invoke(self-1u16)}}}fn invoke<T:Step>(x:T)->usize{x.step()}fn main(){}",
        "invoke",
    );
    let root = ir::CallTarget {
        declaration: owner,
        type_args: Arc::from([ir::Ty::Int(ir::IntTy::U16)]),
    };
    let symbolic = source_summary(&plan, owner, &budget);
    assert!(symbolic.requirement(owner).unwrap().deferred_dispatch);
    let mut erased = symbolic.as_ref().clone();
    Arc::make_mut(&mut erased.members)
        .iter_mut()
        .find(|(member, _)| *member == owner)
        .unwrap()
        .1
        .deferred_dispatch = false;
    assert!(erased.admit_symbolic(owner).is_err());
    let caller = closed_header(&mut plan, root.clone(), &budget);
    let targets: Vec<_> = caller.targets().cloned().collect();
    assert_eq!(targets.len(), 1);
    assert!(
        closed::topology::Topology::discover(root.clone(), vec![caller.clone()], &plan, &budget)
            .is_err()
    );
    use crate::compiler::keys::closed_domain::ClosedDomain;
    use crate::engine::ItemKey;
    let (engine, budget, _, root) = closed_engine_fixture(
        "trait Step{fn step(self)->usize;}impl Step for u16{fn step(self)->usize{if self==0u16{std::mem::align_of::<u16>()}else{invoke(self-1u16)}}}fn invoke<T:Step>(x:T)->usize{x.step()}fn main(){let _n=invoke(0u16);}",
        "invoke",
        Arc::from([ir::Ty::Int(ir::IntTy::U16)]),
    );
    let topology = engine
        .fact_of::<closed::topology::Topology>(
            &ClosedDomain::topology(&root, &budget).unwrap().key(),
            0,
        )
        .unwrap();
    assert_eq!(topology.component().members().len(), 2);
    assert!(!topology.outgoing().is_empty());
    let members: Vec<_> = topology
        .component()
        .words()
        .iter()
        .map(|words| {
            engine
                .fact_of::<closed::Header>(&ClosedDomain::Header(words.clone()).key(), 0)
                .unwrap()
        })
        .collect();
    let external: Vec<_> = topology
        .outgoing()
        .iter()
        .map(|id| {
            engine
                .fact_of::<closed::topology::Summary>(
                    &ClosedDomain::Summary(id.words().to_vec()).key(),
                    0,
                )
                .unwrap()
        })
        .collect();
    let summary =
        closed::topology::Summary::solve(&topology, members.clone(), external.clone(), &budget)
            .unwrap();
    summary.admit(&root, &budget).unwrap();
    assert!(closed::topology::Summary::solve(&topology, members.clone(), vec![], &budget).is_err());
    assert!(
        closed::topology::Summary::solve(&topology, vec![members[0].clone()], external, &budget)
            .is_err()
    );
}

#[test]
fn alignment_domain_compact_opaque_own_method_has_exact_source_contract() {
    let (_, owner, budget, mut plan) = fixture_plan(
        "struct V;trait Inspect{fn width<T>(&self)->usize;}impl Inspect for V{fn width<T>(&self)->usize{std::mem::align_of::<T>()}}fn make()->impl Inspect{V}fn main(){let v=make();let _n=v.width::<u16>();}",
        "main",
    );
    let root = ir::CallTarget {
        declaration: owner,
        type_args: Arc::from([]),
    };
    let caller = closed_header(&mut plan, root.clone(), &budget);
    let invocation = caller.opaque_calls().first().unwrap().clone();
    let producer = closed_header(&mut plan, invocation.owner_target().unwrap(), &budget);
    let facts = plan
        .instance_facts(&invocation.owner_target().unwrap())
        .unwrap();
    let super::super::ContractStep::Source(target) =
        producer.resolve(&invocation, &facts, &budget).unwrap()
    else {
        panic!("concrete authored provider became forwarded")
    };
    assert_eq!(target.type_args.as_ref(), &[ir::Ty::Int(ir::IntTy::U16)]);
    let method = closed_header(&mut plan, target, &budget);
    let topology = closed::topology::Topology::discover(
        root,
        vec![caller, producer.clone(), method],
        &plan,
        &budget,
    )
    .unwrap();
    assert!(!topology.outgoing().is_empty());
    let mut changed = invocation.clone();
    changed.call.method_args = Arc::from([]);
    assert!(producer.resolve(&changed, &facts, &budget).is_err());
    let mut foreign = invocation;
    foreign.call.item = owner;
    assert!(producer.resolve(&foreign, &facts, &budget).is_err());
}

#[test]
fn alignment_domain_compact_preserves_original_generic_reference_expectation() {
    use crate::compiler::keys::closed_domain::ClosedDomain;
    use crate::engine::ItemKey;
    let (engine, budget, _, root) = closed_engine_fixture(
        r#"fn id<T>(x:T)->T{x}fn main(){let a:String=String::from("hello");let r=id(&a);println!("{} {}",r,r.len());}"#,
        "main",
        Arc::from([]),
    );
    let topology = engine
        .fact_of::<closed::topology::Topology>(
            &ClosedDomain::topology(&root, &budget).unwrap().key(),
            0,
        )
        .unwrap();
    let summary = engine
        .fact_of::<closed::topology::Summary>(
            &ClosedDomain::Summary(topology.component().words().to_vec()).key(),
            0,
        )
        .unwrap();
    summary.admit(&root, &budget).unwrap();
    let mut foreign = root;
    foreign.declaration.index ^= 1;
    assert!(summary.admit(&foreign, &budget).is_err());
}

#[test]
fn alignment_domain_compact_virtual_use_retains_explicit_evidence() {
    let (_, owner, budget, mut plan) = fixture_plan(
        "trait Inspect{fn width(&self)->usize;}fn invoke(x:&dyn Inspect)->usize{x.width()}fn main(){}",
        "invoke",
    );
    let root = ir::CallTarget {
        declaration: owner,
        type_args: Arc::from([]),
    };
    let header = closed_header(&mut plan, root.clone(), &budget);
    assert_eq!(header.virtual_sites().len(), 1);
    let topology =
        closed::topology::Topology::discover(root.clone(), vec![header.clone()], &plan, &budget)
            .unwrap();
    let summary =
        closed::topology::Summary::solve(&topology, vec![header], vec![], &budget).unwrap();
    summary.admit(&root, &budget).unwrap();
    let mut wrong = root;
    wrong.declaration.index += 1;
    assert!(summary.admit(&wrong, &budget).is_err());
}
