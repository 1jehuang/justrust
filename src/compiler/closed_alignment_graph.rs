//! Closed alignment producers. Discovery consumes compact owner evidence only.
use super::{
    CompilerProvider, Context, Error, Facts, TEMPLATE_BODY, alignment_domain_graph,
    closure_body_key, ir, keys, memory_layout, scale_task_work_by_callable,
};
use crate::engine::ItemKey;
use keys::closed_domain::ClosedDomain as Domain;
use memory_layout::domain::{
    Local,
    compact::closed::{self, topology},
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
fn request_keys(
    cx: &Context,
    keys: &[Domain],
    budget: &ir::PlanBudget,
) -> Result<Vec<crate::engine::Handle>, Error> {
    for key in keys {
        key.charge_key(budget).map_err(Error::new)?;
    }
    budget
        .retain_bytes(
            keys.len()
                .saturating_mul(std::mem::size_of::<crate::engine::Handle>()),
        )
        .map_err(Error::new)?;
    Ok(cx.request_all(keys, 0))
}
async fn get<T: std::any::Any + Send + Sync>(
    facts: &Facts<'_>,
    key: Domain,
    budget: &ir::PlanBudget,
) -> Result<Arc<T>, Error> {
    key.charge_key(budget).map_err(Error::new)?;
    facts.get(key.key()).await
}

impl crate::engine::Fact for closed::Header {}
impl crate::engine::Fact for topology::Topology {}
impl crate::engine::Fact for topology::Summary {}

pub(super) async fn produce(
    provider: &CompilerProvider,
    request: Domain,
    facts: &Facts<'_>,
    cx: &Context,
    budget: &ir::PlanBudget,
) -> Result<(), Error> {
    match request {
        Domain::Header(words) => {
            let types = facts.types().await?;
            let target = facts.decode_instance(&words, &types, budget).await?;
            let index = types
                .callable_index(target.declaration)
                .map_err(Error::new)?;
            let template_handle = cx.request_item(&keys::Item::<TEMPLATE_BODY>(index)).get(0);
            let domain_key = keys::alignment_domain::AlignmentDomain::Topology(target.declaration);
            domain_key.charge_key(budget).map_err(Error::new)?;
            let domain_handle = cx.request_item(&domain_key).get(0);
            let mut template: Arc<super::TemplateBodyFact> = cx.read_fact(template_handle).await?;
            if let Some(closure) = &template.closure {
                types
                    .validate_closure_instance(closure, &target)
                    .map_err(Error::new)?;
                let mode = types.closure_instance_mode(&target).map_err(Error::new)?;
                template = facts
                    .get(closure_body_key(
                        TEMPLATE_BODY,
                        index,
                        closure.ty.kind(),
                        mode,
                    ))
                    .await?;
            }
            provider.validated_body(&types, index, &template)?;
            scale_task_work_by_callable(&types, target.declaration);
            let domain_topology: Arc<memory_layout::domain::compact::Topology> =
                cx.read_fact(domain_handle).await?;
            let summary_key = keys::alignment_domain::AlignmentDomain::Summary(
                domain_topology.component().members().to_vec(),
            );
            summary_key.charge_key(budget).map_err(Error::new)?;
            let summary: Arc<memory_layout::domain::compact::Summary> =
                facts.get(summary_key.key()).await?;
            let closed_facts = types.instance_facts(&target).map_err(Error::new)?;
            if !summary
                .closed_requirements(&target, &closed_facts, budget)
                .map_err(Error::new)?
            {
                return cx.publish_fact(
                    0,
                    closed::Header::no_continuation(target, &summary, &closed_facts, budget)
                        .map_err(Error::new)?,
                );
            }
            let original_facts = if let Some(closure) = &template.closure {
                types
                    .trait_world()
                    .map_err(Error::new)?
                    .closure_callable_facts_for(
                        closure.ty.clone(),
                        template.closure_mode.unwrap_or(closure.ty.kind()),
                    )
                    .map_err(|error| Error::new(error.into_legacy_diagnostic(budget)))?
            } else if let Some(publication) = &template.publication {
                publication.facts().clone()
            } else {
                types
                    .callable_facts(target.declaration)
                    .map_err(Error::new)?
            };
            let local = Local::capture(
                template.checked.clone(),
                template.publication.clone(),
                original_facts,
                budget,
            )
            .map_err(Error::new)?;
            let mut context = types.as_ref().clone();
            let continuation = local
                .closed_calls(
                    &target,
                    closed_facts,
                    &mut context,
                    budget,
                    Some(cx.authority()),
                )
                .map_err(Error::new)?;
            continuation
                .validate(&mut context, budget, Some(cx.authority()))
                .map_err(Error::new)?;
            let owners: Vec<_> = continuation.provider_declarations().collect();
            let summaries =
                alignment_domain_graph::provider_summaries(facts, &owners, budget).await?;
            let header =
                closed::Header::bind(continuation, summaries, budget).map_err(Error::new)?;
            cx.publish_fact(0, header)
        }
        Domain::Topology(words) => {
            // Closed key decoding can replay closure metadata. Only the owner
            // header producer may do that body-backed work, never discovery.
            let root_key = Domain::Header(words.clone());
            root_key.charge_key(budget).map_err(Error::new)?;
            let root_handle = cx.request_item(&root_key).get(0);
            let types = facts.types().await?;
            let root_header: Arc<closed::Header> = cx.read_fact(root_handle).await?;
            let root = root_header.target().clone();
            if crate::compiler::instances::graph_key::encode(&root, budget).map_err(Error::new)?
                != words
            {
                return Err(Error::new(
                    "closed topology header changed exact canonical key",
                ));
            }
            // Canonical coordinates identify declaration applications even when
            // selected cleanup and decoded headers use distinct nominal IDs.
            let mut requested = HashSet::from([words.clone()]);
            let mut available = HashMap::from([(words, root_header)]);
            let mut targets = Vec::new();
            loop {
                let mut demands = Vec::new();
                for target in targets.drain(..) {
                    budget.charge_work(1).map_err(Error::new)?;
                    let words = crate::compiler::instances::graph_key::encode(&target, budget)
                        .map_err(Error::new)?;
                    if !requested.contains(&words) {
                        budget
                            .retain_bytes(
                                3 * std::mem::size_of::<ir::CallTarget>()
                                    + std::mem::size_of::<Domain>()
                                    + std::mem::size_of::<Arc<closed::Header>>()
                                    + 2 * std::mem::size_of::<Vec<u64>>(),
                            )
                            .map_err(Error::new)?;
                        requested.insert(words.clone());
                        demands.push(Domain::Header(words));
                    }
                }
                let handles = request_keys(cx, &demands, budget)?;
                for (key, handle) in demands.iter().zip(handles) {
                    let header: Arc<closed::Header> = cx.read_fact(handle).await?;
                    let words =
                        crate::compiler::instances::graph_key::encode(header.target(), budget)
                            .map_err(Error::new)?;
                    if !matches!(key, Domain::Header(expected) if *expected == words) {
                        return Err(Error::new(
                            "closed alignment header changed requested canonical owner",
                        ));
                    }
                    if available.insert(words, header).is_some() {
                        return Err(Error::new("closed alignment discovery aliased owner keys"));
                    }
                }
                // Source contract owners may become available in this wave.
                // Resolve only sealed compact contracts, never read bodies.
                for header in available.values() {
                    for target in topology::continuations(header, &available, &types, budget)
                        .map_err(Error::new)?
                    {
                        let words = crate::compiler::instances::graph_key::encode(&target, budget)
                            .map_err(Error::new)?;
                        if !requested.contains(&words) {
                            budget
                                .retain_bytes(std::mem::size_of::<ir::CallTarget>())
                                .map_err(Error::new)?;
                            targets.push(target);
                        }
                    }
                }
                if targets.is_empty() {
                    break;
                }
            }
            let topology = topology::Topology::discover(
                root,
                available.into_values().collect(),
                &types,
                budget,
            )
            .map_err(Error::new)?;
            cx.publish_fact(0, topology)
        }
        Domain::Summary(members) => {
            let representative = members
                .first()
                .ok_or_else(|| Error::new("closed alignment SCC has no representative"))?;
            let topology_handles =
                request_keys(cx, &[Domain::Topology(representative.clone())], budget)?;
            let member_keys: Vec<_> = members.iter().cloned().map(Domain::Header).collect();
            let member_handles = request_keys(cx, &member_keys, budget)?;
            let topology: Arc<topology::Topology> =
                cx.read_fact(topology_handles[0].clone()).await?;
            if crate::compiler::instances::graph_key::encode(topology.root(), budget)
                .map_err(Error::new)?
                != *representative
                || topology.component().words() != members.as_slice()
            {
                return Err(Error::new(
                    "closed alignment summary key changed canonical SCC membership",
                ));
            }
            let outgoing_keys: Vec<_> = topology
                .outgoing()
                .iter()
                .map(|id| Domain::Summary(id.words().to_vec()))
                .collect();
            let outgoing_handles = request_keys(cx, &outgoing_keys, budget)?;
            let mut headers = Vec::new();
            for handle in member_handles {
                headers.push(cx.read_fact::<closed::Header>(handle).await?);
            }
            let mut outgoing = Vec::new();
            for handle in outgoing_handles {
                outgoing.push(cx.read_fact::<topology::Summary>(handle).await?);
            }
            let summary = topology::Summary::solve(&topology, headers, outgoing, budget)
                .map_err(Error::new)?;
            cx.publish_fact(0, summary)
        }
    }
}

pub(super) async fn summary(
    facts: &Facts<'_>,
    target: &ir::CallTarget,
    budget: &ir::PlanBudget,
) -> Result<Arc<topology::Summary>, Error> {
    let topology: Arc<topology::Topology> = get(
        facts,
        Domain::topology(target, budget).map_err(Error::new)?,
        budget,
    )
    .await?;
    if crate::compiler::instances::graph_key::encode(topology.root(), budget).map_err(Error::new)?
        != crate::compiler::instances::graph_key::encode(target, budget).map_err(Error::new)?
    {
        return Err(Error::new(
            "closed alignment topology changed requested owner",
        ));
    }
    get(
        facts,
        Domain::Summary(topology.component().words().to_vec()),
        budget,
    )
    .await
}
