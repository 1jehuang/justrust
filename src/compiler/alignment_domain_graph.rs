//! Graph producers for compact alignment domains. No producer here can read a
//! transitive body inventory: discovery reads only compact adjacency outputs,
//! and summary producers read member headers plus outgoing component summaries.
use super::{Context, Error, Facts, ir, keys, memory_layout, scale_task_work_by_callable};
use crate::engine::ItemKey;
use memory_layout::domain::compact::{Header, Summary, Topology};
use std::collections::HashSet;
use std::sync::Arc;

fn request_keys(
    cx: &Context,
    keys: &[keys::alignment_domain::AlignmentDomain],
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
    key: keys::alignment_domain::AlignmentDomain,
    budget: &ir::PlanBudget,
) -> Result<Arc<T>, Error> {
    key.charge_key(budget).map_err(Error::new)?;
    facts.get(key.key()).await
}

impl crate::engine::Fact for Header {}
impl crate::engine::Fact for Topology {}
impl crate::engine::Fact for Summary {}

pub(super) async fn produce(
    request: keys::alignment_domain::AlignmentDomain,
    facts: &Facts<'_>,
    cx: &Context,
    budget: &ir::PlanBudget,
) -> Result<(), Error> {
    use keys::alignment_domain::AlignmentDomain as Domain;
    match request {
        Domain::Header(owner) => {
            let types = facts.types().await?;
            let index = types.callable_index(owner).map_err(Error::new)?;
            scale_task_work_by_callable(&types, owner);
            let template = facts.template(index).await?;
            let callable = if let Some(closure) = &template.closure {
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
                types.callable_facts(owner).map_err(Error::new)?
            };
            let mut header = Header::capture(
                template.checked.clone(),
                template.publication.clone(),
                callable,
                budget,
            )
            .map_err(Error::new)?;
            if template.closure.is_some() {
                header.require_closed_replay();
            }
            if header.owner() != owner {
                return Err(Error::new("alignment header changed canonical key owner"));
            }
            cx.publish_fact(0, header)
        }
        Domain::Topology(owner) => {
            let mut seen = HashSet::new();
            let mut frontier = vec![owner];
            let mut adjacency = Vec::new();
            while !frontier.is_empty() {
                let mut keys = Vec::new();
                for next in frontier.drain(..) {
                    budget.charge_work(1).map_err(Error::new)?;
                    if !seen.contains(&next) {
                        budget
                            .retain_bytes(
                                std::mem::size_of::<ir::DeclId>()
                                    + std::mem::size_of::<Domain>()
                                    + std::mem::size_of::<Arc<Header>>(),
                            )
                            .map_err(Error::new)?;
                        seen.insert(next);
                        keys.push(Domain::Header(next));
                    }
                }
                // All independently known header producers are demanded first.
                let handles = request_keys(cx, &keys, budget)?;
                for (key, handle) in keys.iter().zip(handles) {
                    let header: Arc<Header> = cx.read_fact(handle).await?;
                    if !matches!(key, Domain::Header(expected) if *expected == header.owner()) {
                        return Err(Error::new(
                            "alignment discovery received a foreign owner header",
                        ));
                    }
                    let vertex = header.topology(budget).map_err(Error::new)?;
                    budget
                        .retain_bytes(
                            vertex
                                .callees()
                                .len()
                                .saturating_mul(std::mem::size_of::<ir::DeclId>())
                                + std::mem::size_of::<memory_layout::domain::compact::Adjacency>(),
                        )
                        .map_err(Error::new)?;
                    frontier.extend(vertex.callees().iter().copied());
                    adjacency.push(vertex);
                }
            }
            let topology = Topology::discover(owner, adjacency, budget).map_err(Error::new)?;
            cx.publish_fact(0, topology)
        }
        Domain::Summary(members) => {
            let representative = *members
                .first()
                .ok_or_else(|| Error::new("alignment summary has no canonical representative"))?;
            let topology_handles = request_keys(cx, &[Domain::Topology(representative)], budget)?;
            let member_keys: Vec<_> = members.iter().copied().map(Domain::Header).collect();
            let member_handles = request_keys(cx, &member_keys, budget)?;
            let topology: Arc<Topology> = cx.read_fact(topology_handles[0].clone()).await?;
            if topology.root() != representative
                || topology.component().members() != members.as_slice()
            {
                return Err(Error::new(
                    "alignment summary key changed its canonical SCC",
                ));
            }
            let external_keys: Vec<_> = topology
                .outgoing()
                .iter()
                .map(|id| Domain::Summary(id.members().to_vec()))
                .collect();
            let external_handles = request_keys(cx, &external_keys, budget)?;
            // Both sets, including every independent external summary, were
            // requested before awaiting the first result from either set.
            let mut headers = Vec::new();
            for handle in member_handles {
                headers.push(cx.read_fact::<Header>(handle).await?);
            }
            let mut external = Vec::new();
            for handle in external_handles {
                external.push(cx.read_fact::<Summary>(handle).await?);
            }
            let summary =
                Summary::solve(&topology, headers, external, budget).map_err(Error::new)?;
            cx.publish_fact(0, summary)
        }
    }
}

pub(super) async fn summary(
    facts: &Facts<'_>,
    owner: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<Arc<Summary>, Error> {
    use keys::alignment_domain::AlignmentDomain as Domain;
    let topology: Arc<Topology> = get(facts, Domain::Topology(owner), budget).await?;
    if topology.root() != owner {
        return Err(Error::new("alignment topology changed requested owner"));
    }
    get(
        facts,
        Domain::Summary(topology.component().members().to_vec()),
        budget,
    )
    .await
}

/// Resolve all provider component identities before requesting any summaries.
/// The second request wave is likewise complete before its first await.
pub(super) async fn provider_summaries(
    facts: &Facts<'_>,
    owners: &[ir::DeclId],
    budget: &ir::PlanBudget,
) -> Result<Vec<Arc<Summary>>, Error> {
    use keys::alignment_domain::AlignmentDomain as Domain;
    let topology_keys: Vec<_> = owners.iter().copied().map(Domain::Topology).collect();
    let handles = request_keys(facts.cx, &topology_keys, budget)?;
    let mut summary_keys = Vec::new();
    for (owner, handle) in owners.iter().zip(handles) {
        let topology: Arc<Topology> = facts.cx.read_fact(handle).await?;
        if topology.root() != *owner {
            return Err(Error::new(
                "alignment provider topology changed requested owner",
            ));
        }
        summary_keys.push(Domain::Summary(topology.component().members().to_vec()));
    }
    let handles = request_keys(facts.cx, &summary_keys, budget)?;
    let mut summaries = Vec::new();
    for handle in handles {
        summaries.push(facts.cx.read_fact::<Summary>(handle).await?);
    }
    Ok(summaries)
}
