//! Compact closed topology and SCC-local conjunction admission.
use super::super::super::{ContractStep, graph_components};
use super::{Header, ir};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone)]
pub(in crate::compiler) struct ComponentId {
    members: Arc<[ir::CallTarget]>,
    words: Arc<[Vec<u64>]>,
}
impl PartialEq for ComponentId {
    fn eq(&self, other: &Self) -> bool {
        self.words == other.words
    }
}
impl Eq for ComponentId {}
impl ComponentId {
    fn validate(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        if self.words.is_empty()
            || self.words.len() != self.members.len()
            || self.words.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err("closed alignment SCC changed canonical member coordinates".into());
        }
        for (target, expected) in self.members.iter().zip(self.words.iter()) {
            budget.charge_work(1)?;
            if crate::compiler::instances::graph_key::encode(target, budget)? != *expected {
                return Err(
                    "closed alignment SCC changed member declaration or ordered actuals".into(),
                );
            }
        }
        Ok(())
    }
    fn new(members: Vec<ir::CallTarget>, budget: &ir::PlanBudget) -> Result<Self, String> {
        budget.retain_bytes(
            members
                .len()
                .saturating_mul(std::mem::size_of::<(Vec<u64>, ir::CallTarget)>()),
        )?;
        let mut ordered = Vec::new();
        for member in members {
            budget.charge_work(1)?;
            ordered.push((
                crate::compiler::instances::graph_key::encode(&member, budget)?,
                member,
            ));
        }
        ordered.sort_by(|left, right| left.0.cmp(&right.0));
        if ordered.is_empty() || ordered.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err("closed alignment SCC has empty or duplicate canonical members".into());
        }
        let (words, members): (Vec<_>, Vec<_>) = ordered.into_iter().unzip();
        Ok(Self {
            members: members.into(),
            words: words.into(),
        })
    }
    #[cfg(test)]
    pub(in crate::compiler) fn members(&self) -> &[ir::CallTarget] {
        &self.members
    }
    pub(in crate::compiler) fn words(&self) -> &[Vec<u64>] {
        &self.words
    }
}
#[derive(Clone)]
struct Vertex {
    owner: ir::CallTarget,
    targets: Arc<[ir::CallTarget]>,
    query_sites: Arc<[ir::ExprId]>,
    virtual_sites: Arc<[ir::ExprId]>,
}
#[derive(Clone)]
pub(in crate::compiler) struct Topology {
    root: ir::CallTarget,
    component: ComponentId,
    outgoing: Arc<[ComponentId]>,
    vertices: Arc<[Vertex]>,
}

/// Finds only independently missing source-owner contracts. Discovery can
/// request the entire returned wave before awaiting, without optimistic cycles.
pub(in crate::compiler) fn continuations(
    header: &Header,
    available: &HashMap<Vec<u64>, Arc<Header>>,
    context: &crate::compiler::frontend::PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<Vec<ir::CallTarget>, String> {
    budget.retain_bytes(
        header
            .selected
            .len()
            .saturating_mul(std::mem::size_of::<ir::CallTarget>())
            + header
                .opaque_calls()
                .len()
                .saturating_mul(2 * std::mem::size_of::<super::super::super::OpaqueCall>()),
    )?;
    let mut demands: Vec<_> = header.targets().cloned().collect();
    let mut pending = header.opaque_calls().to_vec();
    let mut seen = Vec::new();
    while let Some(invocation) = pending.pop() {
        budget.charge_work(1)?;
        if seen.contains(&invocation) {
            continue;
        }
        let owner = invocation.owner_target()?;
        budget.retain_bytes(
            2 * std::mem::size_of::<ir::CallTarget>()
                + std::mem::size_of::<super::super::super::OpaqueCall>(),
        )?;
        demands.push(owner.clone());
        let owner_words = crate::compiler::instances::graph_key::encode(&owner, budget)?;
        if let Some(producer) = available.get(&owner_words) {
            let facts = context.instance_facts(&owner)?;
            match producer.resolve(&invocation, &facts, budget)? {
                ContractStep::Source(target) => demands.push(target),
                ContractStep::Forwarded(next) => pending.push(next),
            }
        }
        seen.push(invocation);
    }
    Ok(demands)
}
impl Topology {
    pub(in crate::compiler) fn discover(
        root: ir::CallTarget,
        headers: Vec<Arc<Header>>,
        context: &crate::compiler::frontend::PlanCtx,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        budget.retain_bytes(headers.len().saturating_mul(
            std::mem::size_of::<Vertex>()
                + std::mem::size_of::<(Vec<u64>, usize)>()
                + std::mem::size_of::<(Vec<u64>, Arc<Header>)>()
                + 2 * std::mem::size_of::<Vec<usize>>(),
        ))?;
        let mut available = HashMap::new();
        let mut indices = HashMap::new();
        for (index, header) in headers.iter().enumerate() {
            budget.charge_work(1)?;
            let words = crate::compiler::instances::graph_key::encode(header.target(), budget)?;
            if available.insert(words.clone(), header.clone()).is_some() {
                return Err("closed alignment topology repeats an owner".into());
            }
            indices.insert(words, index);
        }
        let root_words = crate::compiler::instances::graph_key::encode(&root, budget)?;
        let root_index = *indices
            .get(&root_words)
            .ok_or("closed alignment topology omitted root")?;
        let mut edges = vec![Vec::new(); headers.len()];
        let mut reverse = vec![Vec::new(); headers.len()];
        let mut vertices = Vec::new();
        for (index, header) in headers.iter().enumerate() {
            let selected = continuations(header, &available, context, budget)?;
            // Every opaque contract owner must exist, including forwarded
            // providers. Absence is never interpreted as empty dispatch.
            let mut pending = header.opaque_calls().to_vec();
            let mut seen = Vec::new();
            while let Some(invocation) = pending.pop() {
                budget.charge_work(1)?;
                if seen.contains(&invocation) {
                    continue;
                }
                let owner = invocation.owner_target()?;
                let owner_words = crate::compiler::instances::graph_key::encode(&owner, budget)?;
                let producer = available
                    .get(&owner_words)
                    .ok_or("closed alignment topology omitted opaque contract owner")?;
                let facts = context.instance_facts(&owner)?;
                if let ContractStep::Forwarded(next) =
                    producer.resolve(&invocation, &facts, budget)?
                {
                    pending.push(next);
                }
                seen.push(invocation);
            }
            budget.retain_bytes(selected.len().saturating_mul(
                std::mem::size_of::<ir::CallTarget>() + 2 * std::mem::size_of::<usize>(),
            ))?;
            let mut targets = Vec::new();
            for target in &selected {
                budget.charge_work(1)?;
                let words = crate::compiler::instances::graph_key::encode(target, budget)?;
                let next = *indices
                    .get(&words)
                    .ok_or("closed alignment topology omitted selected continuation")?;
                // Retain the authenticated header representative, not an
                // allocation-specific nominal application from cleanup.
                targets.push(headers[next].target().clone());
                edges[index].push(next);
                reverse[next].push(index);
            }
            vertices.push(Vertex {
                owner: header.target.clone(),
                targets: targets.into(),
                query_sites: header.queries.iter().map(|(site, _)| *site).collect(),
                virtual_sites: header.virtual_sites.clone(),
            });
        }
        let mut seen = vec![false; headers.len()];
        let mut pending = vec![root_index];
        while let Some(index) = pending.pop() {
            budget.charge_work(1)?;
            if seen[index] {
                continue;
            }
            seen[index] = true;
            pending.extend(edges[index].iter().copied());
        }
        if seen.iter().any(|seen| !seen) {
            return Err("closed alignment topology added unrelated owner".into());
        }
        let components = graph_components(&edges, &reverse, budget)?;
        let mut membership = vec![usize::MAX; headers.len()];
        let mut ids = Vec::new();
        for (component, members) in components.iter().enumerate() {
            for &member in members {
                membership[member] = component;
            }
            ids.push(ComponentId::new(
                members
                    .iter()
                    .map(|&index| headers[index].target.clone())
                    .collect(),
                budget,
            )?);
        }
        let own = membership[root_index];
        let mut outgoing_indices = HashSet::new();
        let mut local_vertices = Vec::new();
        for &member in &components[own] {
            local_vertices.push(vertices[member].clone());
            for &target in &edges[member] {
                budget.charge_work(1)?;
                if membership[target] != own {
                    outgoing_indices.insert(membership[target]);
                }
            }
        }
        budget.retain_bytes(
            local_vertices
                .len()
                .saturating_mul(std::mem::size_of::<(Vec<u64>, Vertex)>()),
        )?;
        let mut ordered_vertices = Vec::new();
        for vertex in local_vertices {
            let words = crate::compiler::instances::graph_key::encode(&vertex.owner, budget)?;
            ordered_vertices.push((words, vertex));
        }
        ordered_vertices.sort_by(|left, right| left.0.cmp(&right.0));
        let local_vertices: Vec<_> = ordered_vertices
            .into_iter()
            .map(|(_, vertex)| vertex)
            .collect();
        let mut outgoing: Vec<_> = outgoing_indices
            .into_iter()
            .map(|index| ids[index].clone())
            .collect();
        outgoing.sort_by(|left, right| left.words.cmp(&right.words));
        Ok(Self {
            root: headers[root_index].target().clone(),
            component: ids[own].clone(),
            outgoing: outgoing.into(),
            vertices: local_vertices.into(),
        })
    }
    pub(in crate::compiler) fn root(&self) -> &ir::CallTarget {
        &self.root
    }
    pub(in crate::compiler) fn component(&self) -> &ComponentId {
        &self.component
    }
    pub(in crate::compiler) fn outgoing(&self) -> &[ComponentId] {
        &self.outgoing
    }
}

type OccurrenceEvidence = (Vec<u64>, Arc<[ir::ExprId]>, Arc<[ir::ExprId]>);

/// Successful conjunction for exactly one closed SCC, after all its outgoing
/// SCC summaries are read. No reachable body/header/proof collection retained.
#[derive(Clone)]
pub(in crate::compiler) struct Summary {
    component: ComponentId,
    evidence: Arc<[OccurrenceEvidence]>,
}
impl Summary {
    pub(in crate::compiler) fn solve(
        topology: &Topology,
        headers: Vec<Arc<Header>>,
        external: Vec<Arc<Summary>>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        topology.component.validate(budget)?;
        let root_words = crate::compiler::instances::graph_key::encode(topology.root(), budget)?;
        if !topology.component.words.contains(&root_words) {
            return Err("closed SCC changed its canonical topology root".into());
        }
        budget.retain_bytes(
            headers
                .len()
                .saturating_mul(std::mem::size_of::<(Vec<u64>, &Arc<Header>)>()),
        )?;
        budget.retain_bytes(
            external
                .len()
                .saturating_mul(std::mem::size_of::<ComponentId>()),
        )?;
        let mut local = HashMap::new();
        for header in &headers {
            budget.charge_work(1)?;
            let words = crate::compiler::instances::graph_key::encode(header.target(), budget)?;
            if local.insert(words, header).is_some() {
                return Err("closed SCC repeated local header".into());
            }
        }
        if local.len() != topology.component.words.len()
            || topology
                .component
                .words
                .iter()
                .any(|owner| !local.contains_key(owner))
        {
            return Err("closed SCC changed canonical local member inventory".into());
        }
        let mut external_ids = Vec::new();
        for expected in topology.outgoing.iter() {
            expected.validate(budget)?;
        }
        for summary in &external {
            budget.charge_work(1)?;
            summary.validate(budget)?;
            if !topology.outgoing.contains(&summary.component)
                || external_ids.contains(&summary.component)
            {
                return Err("closed SCC added or repeated outgoing summary".into());
            }
            external_ids.push(summary.component.clone());
        }
        if external_ids.len() != topology.outgoing.len() {
            return Err("closed SCC omitted outgoing summary".into());
        }
        let mut evidence = Vec::new();
        for vertex in topology.vertices.iter() {
            budget.charge_work(1)?;
            let owner_words = crate::compiler::instances::graph_key::encode(&vertex.owner, budget)?;
            let header = local
                .get(&owner_words)
                .ok_or("closed SCC topology changed local owner")?;
            let sites: Vec<_> = header.queries.iter().map(|(site, _)| *site).collect();
            if sites.as_slice() != vertex.query_sites.as_ref()
                || header.virtual_sites != vertex.virtual_sites
            {
                return Err("closed SCC changed local query or virtual occurrence receipt".into());
            }
            for target in vertex.targets.iter() {
                budget.charge_work(1)?;
                let words = crate::compiler::instances::graph_key::encode(target, budget)?;
                if !local.contains_key(&words)
                    && !external
                        .iter()
                        .any(|summary| summary.component.words.contains(&words))
                {
                    return Err("closed SCC omitted selected continuation admission".into());
                }
            }
            budget.retain_bytes(std::mem::size_of::<(
                Vec<u64>,
                Arc<[ir::ExprId]>,
                Arc<[ir::ExprId]>,
            )>())?;
            evidence.push((
                owner_words,
                vertex.query_sites.clone(),
                vertex.virtual_sites.clone(),
            ));
        }
        if evidence.len() != local.len() {
            return Err("closed SCC omitted local occurrence evidence".into());
        }
        let summary = Self {
            component: topology.component.clone(),
            evidence: evidence.into(),
        };
        summary.validate(budget)?;
        Ok(summary)
    }
    fn validate(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        self.component.validate(budget)?;
        budget.charge_work(self.evidence.len())?;
        if self.evidence.len() != self.component.words.len()
            || self
                .evidence
                .iter()
                .zip(self.component.words.iter())
                .any(|((owner, _, _), expected)| owner != expected)
        {
            return Err("closed SCC changed its exact ordered occurrence owner inventory".into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn admit(
        &self,
        owner: &ir::CallTarget,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        self.validate(budget)?;
        let words = crate::compiler::instances::graph_key::encode(owner, budget)?;
        if self.component.words.binary_search(&words).is_err() {
            return Err("closed SCC admission changed canonical owner".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::memory_layout::domain::{
        compact::tests::closed_header, tests::fixture_plan,
    };
    #[test]
    fn alignment_domain_compact_closed_receipts_reject_erased_scc_and_virtual_evidence() {
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
            Topology::discover(root.clone(), vec![header.clone()], &plan, &budget).unwrap();
        let summary = Summary::solve(&topology, vec![header.clone()], vec![], &budget).unwrap();
        summary.admit(&root, &budget).unwrap();
        let mut foreign_root = topology.clone();
        foreign_root.root.declaration.index = foreign_root.root.declaration.index.wrapping_add(1);
        assert!(Summary::solve(&foreign_root, vec![header.clone()], vec![], &budget).is_err());
        let mut erased = summary;
        erased.evidence = Arc::from([]);
        assert!(erased.admit(&root, &budget).is_err());
        let mut erased_scc = topology;
        erased_scc.vertices = Arc::from([]);
        assert!(Summary::solve(&erased_scc, vec![header.clone()], vec![], &budget).is_err());
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(Summary::solve(&erased_scc, vec![header], vec![], &denied).is_err());
    }

    #[test]
    fn alignment_domain_summary_seals_canonical_ordered_actuals_and_inventory() {
        let (_, owner, budget, mut plan) = fixture_plan(
            "fn width<T,U>()->usize{std::mem::align_of::<&T>()}fn main(){}",
            "width",
        );
        let root = ir::CallTarget {
            declaration: owner,
            type_args: Arc::from([ir::Ty::Int(ir::IntTy::U8), ir::Ty::Int(ir::IntTy::U16)]),
        };
        let header = closed_header(&mut plan, root.clone(), &budget);
        let topology =
            Topology::discover(root.clone(), vec![header.clone()], &plan, &budget).unwrap();
        let summary = Summary::solve(&topology, vec![header], vec![], &budget).unwrap();
        summary.admit(&root, &budget).unwrap();
        let mut reordered = root.clone();
        Arc::make_mut(&mut reordered.type_args).swap(0, 1);
        assert!(summary.admit(&reordered, &budget).is_err());
        let mut forged_member = summary.clone();
        Arc::make_mut(&mut forged_member.component.members)[0] = reordered;
        assert!(forged_member.admit(&root, &budget).is_err());
        let mut missing = summary.clone();
        missing.evidence = Arc::from([]);
        assert!(missing.admit(&root, &budget).is_err());
        let mut repeated = summary.clone();
        repeated.evidence = vec![summary.evidence[0].clone(); 2].into();
        assert!(repeated.admit(&root, &budget).is_err());
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        assert!(summary.admit(&root, &denied).is_err());
    }

    #[test]
    fn alignment_domain_cleanup_uses_canonical_nominal_application_coordinates() {
        use crate::compiler::memory_layout::domain::compact::tests::closed_engine_fixture;
        use crate::compiler::{instances::graph_key, keys::closed_domain::ClosedDomain};
        use crate::engine::ItemKey;
        let (engine, budget, plan, root) = closed_engine_fixture(
            "struct D<T>(T);impl<T> Drop for D<T>{fn drop(&mut self){let _n=std::mem::align_of::<T>();}}fn consume<T>(_x:T){}fn main(){consume(D(7u16));}",
            "main",
            Arc::from([]),
        );
        let mut pending = vec![root.clone()];
        let mut available = HashMap::new();
        let mut reconstructed_nominal = false;
        while let Some(selected) = pending.pop() {
            let words = graph_key::encode(&selected, &budget).unwrap();
            if available.contains_key(&words) {
                continue;
            }
            let decoded = graph_key::decode(&words, &plan, &budget).unwrap();
            assert_eq!(graph_key::encode(&decoded, &budget).unwrap(), words);
            reconstructed_nominal |= decoded != selected;
            let header = engine
                .fact_of::<Header>(&ClosedDomain::Header(words.clone()).key(), 0)
                .unwrap();
            pending.extend(header.targets().cloned());
            available.insert(words, header);
        }
        assert!(
            reconstructed_nominal,
            "fixture must cross nominal realization identities"
        );
        let headers: Vec<_> = available.into_values().collect();
        Topology::discover(root.clone(), headers.clone(), &plan, &budget).unwrap();
        assert!(
            Topology::discover(
                root.clone(),
                vec![
                    headers
                        .iter()
                        .find(|h| h.target() == &root)
                        .unwrap()
                        .clone()
                ],
                &plan,
                &budget
            )
            .is_err()
        );
        let mut foreign = headers;
        let victim = foreign.iter_mut().find(|h| h.target() != &root).unwrap();
        Arc::make_mut(victim).target.declaration.index ^= 1;
        assert!(Topology::discover(root, foreign, &plan, &budget).is_err());
    }
}
