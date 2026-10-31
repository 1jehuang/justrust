//! Compact, declaration-local alignment facts and SCC-local finite summaries.
//!
//! Only Header::capture may inspect a checked body. Neither a topology fact nor
//! a summary owns a body, a source publication, a PlanCtx, or another summary.
pub(in crate::compiler) mod closed;
#[cfg(test)]
pub(super) mod tests;
use super::{Argument, Local, Requirement, graph_components, ir, source_replay, traits};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
#[cfg(test)]
pub(in crate::compiler) use tests::closed_engine_fixture;

#[derive(Clone, PartialEq, Eq)]
struct Transfer {
    callee: ir::DeclId,
    occurrence: ir::ExprId,
    arguments: Arc<[Argument]>,
}

#[derive(Clone)]
struct HeaderReceipt {
    owner: ir::DeclId,
    direct: Requirement,
    transfers: Arc<[Transfer]>,
}

/// Sealed output of the sole body-reading producer for one declaration.
#[derive(Clone)]
pub(in crate::compiler) struct Header {
    owner: ir::DeclId,
    direct: Requirement,
    transfers: Arc<[Transfer]>,
    receipt: Arc<HeaderReceipt>,
}
impl Header {
    pub(in crate::compiler) fn capture(
        source: Arc<ir::CheckedFunction>,
        publication: Option<Arc<source_replay::CanonicalSourcePublication>>,
        facts: Arc<traits::ValidatedFacts>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        // Local is computation-local scratch, never part of the output.
        let local = Local::capture(source, publication, facts, budget)?;
        local.validate(budget)?;
        let mut transfers = Vec::new();
        for edge in local.edges.iter() {
            budget.charge_work(1)?;
            budget.retain_bytes(std::mem::size_of::<Transfer>())?;
            transfers.push(Transfer {
                callee: edge.target.declaration,
                occurrence: edge.occurrence,
                arguments: edge.arguments.clone().into(),
            });
        }
        let transfers: Arc<[Transfer]> = transfers.into();
        budget.retain_bytes(
            std::mem::size_of::<HeaderReceipt>()
                + local
                    .direct
                    .parameters
                    .len()
                    .saturating_mul(7 + 2 * std::mem::size_of::<Option<usize>>()),
        )?;
        let receipt = Arc::new(HeaderReceipt {
            owner: local.owner,
            direct: local.direct.clone(),
            transfers: transfers.clone(),
        });
        Ok(Self {
            owner: local.owner,
            direct: local.direct,
            transfers,
            receipt,
        })
    }
    pub(in crate::compiler) fn require_closed_replay(&mut self) {
        self.direct.deferred_dispatch = true;
        Arc::make_mut(&mut self.receipt).direct.deferred_dispatch = true;
    }
    fn validate_receipt(&self) -> Result<(), String> {
        if self.owner != self.receipt.owner
            || self.direct != self.receipt.direct
            || self.transfers != self.receipt.transfers
        {
            return Err("alignment header changed its immutable original source projection".into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn owner(&self) -> ir::DeclId {
        self.owner
    }
    pub(in crate::compiler) fn topology(
        &self,
        budget: &ir::PlanBudget,
    ) -> Result<Adjacency, String> {
        self.validate_receipt()?;
        let mut callees = Vec::new();
        let mut occurrences = HashSet::new();
        for transfer in self.transfers.iter() {
            if !occurrences.insert(transfer.occurrence) {
                return Err("alignment header repeated an authored call occurrence".into());
            }
            budget.charge_work(1)?;
            budget.retain_bytes(std::mem::size_of::<ir::DeclId>())?;
            callees.push(transfer.callee);
        }
        callees.sort();
        callees.dedup();
        Ok(Adjacency {
            owner: self.owner,
            callees: callees.into(),
        })
    }
}

/// Only adjacency, not even local query requirements, is exposed to discovery.
#[derive(Clone)]
pub(in crate::compiler) struct Adjacency {
    owner: ir::DeclId,
    callees: Arc<[ir::DeclId]>,
}
impl Adjacency {
    pub(in crate::compiler) fn callees(&self) -> &[ir::DeclId] {
        &self.callees
    }
}

/// Canonical component identity. Ordered members make equivalent discoveries
/// share the same summary producer regardless of traversal root or order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::compiler) struct ComponentId(Arc<[ir::DeclId]>);
impl ComponentId {
    fn new(mut members: Vec<ir::DeclId>, budget: &ir::PlanBudget) -> Result<Self, String> {
        budget.charge_work(members.len())?;
        budget.retain_bytes(
            members
                .len()
                .saturating_mul(std::mem::size_of::<ir::DeclId>()),
        )?;
        members.sort();
        if members.is_empty() || members.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("alignment component has empty or duplicate canonical members".into());
        }
        Ok(Self(members.into()))
    }
    pub(in crate::compiler) fn members(&self) -> &[ir::DeclId] {
        &self.0
    }
}

/// Discovery output contains only the requested owner's canonical SCC and its
/// immediate outgoing SCC identities. The complete traversal is node-local.
#[derive(Clone)]
pub(in crate::compiler) struct Topology {
    root: ir::DeclId,
    component: ComponentId,
    outgoing: Arc<[ComponentId]>,
}
impl Topology {
    pub(in crate::compiler) fn discover(
        root: ir::DeclId,
        graph: Vec<Adjacency>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        budget.retain_bytes(graph.len().saturating_mul(
            2 * std::mem::size_of::<Vec<usize>>()
                + 2 * std::mem::size_of::<usize>()
                + 1
                + std::mem::size_of::<ComponentId>(),
        ))?;
        let mut positions = HashMap::new();
        for (index, vertex) in graph.iter().enumerate() {
            budget.charge_work(1)?;
            budget.retain_bytes(std::mem::size_of::<(ir::DeclId, usize)>())?;
            if positions.insert(vertex.owner, index).is_some() {
                return Err("alignment topology repeats a declaration header".into());
            }
        }
        let root_index = *positions
            .get(&root)
            .ok_or("alignment topology omitted its root")?;
        let mut edges = vec![Vec::new(); graph.len()];
        let mut reverse = vec![Vec::new(); graph.len()];
        for (source, vertex) in graph.iter().enumerate() {
            for callee in vertex.callees.iter() {
                budget.charge_work(1)?;
                let destination = *positions
                    .get(callee)
                    .ok_or("alignment topology omitted a reachable header")?;
                budget.retain_bytes(2 * std::mem::size_of::<usize>())?;
                edges[source].push(destination);
                reverse[destination].push(source);
            }
        }
        // Reject unrelated or invented discovery inputs, not just missing ones.
        let mut visited = vec![false; graph.len()];
        let mut pending = vec![root_index];
        while let Some(source) = pending.pop() {
            budget.charge_work(1)?;
            if visited[source] {
                continue;
            }
            visited[source] = true;
            pending.extend(edges[source].iter().copied());
        }
        if visited.iter().any(|present| !present) {
            return Err("alignment topology included an unrelated declaration".into());
        }
        let components = graph_components(&edges, &reverse, budget)?;
        let mut membership = vec![usize::MAX; graph.len()];
        let mut identities = Vec::new();
        for (index, members) in components.iter().enumerate() {
            let mut owners = Vec::new();
            for &member in members {
                budget.charge_work(1)?;
                membership[member] = index;
                owners.push(graph[member].owner);
            }
            identities.push(ComponentId::new(owners, budget)?);
        }
        let own_component = membership[root_index];
        let mut outgoing = Vec::new();
        for &member in &components[own_component] {
            for &destination in &edges[member] {
                budget.charge_work(1)?;
                let component = membership[destination];
                if component != own_component {
                    budget.retain_bytes(std::mem::size_of::<ComponentId>())?;
                    outgoing.push(identities[component].clone());
                }
            }
        }
        outgoing.sort_by(|left, right| left.0.cmp(&right.0));
        outgoing.dedup();
        Ok(Self {
            root,
            component: identities[own_component].clone(),
            outgoing: outgoing.into(),
        })
    }
    pub(in crate::compiler) fn root(&self) -> ir::DeclId {
        self.root
    }
    pub(in crate::compiler) fn component(&self) -> &ComponentId {
        &self.component
    }
    pub(in crate::compiler) fn outgoing(&self) -> &[ComponentId] {
        &self.outgoing
    }
}

/// Compact output, not a recursively retained proof graph. All dependencies
/// are engine reads by the SCC producer. Each external summary is projected to
/// finite bits, never copied as a proof tree into this output.
#[derive(Clone)]
pub(in crate::compiler) struct Summary {
    component: ComponentId,
    members: Arc<[(ir::DeclId, Requirement)]>,
    original_members: Arc<[(ir::DeclId, Requirement)]>,
}
impl Summary {
    pub(in crate::compiler) fn solve(
        topology: &Topology,
        headers: Vec<Arc<Header>>,
        external: Vec<Arc<Summary>>,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        let id = &topology.component;
        budget.retain_bytes(headers.len().saturating_mul(
            std::mem::size_of::<(ir::DeclId, usize)>()
                + std::mem::size_of::<Requirement>()
                + std::mem::size_of::<Vec<usize>>()
                + std::mem::size_of::<usize>()
                + 1,
        ))?;
        let mut positions = HashMap::new();
        for (index, header) in headers.iter().enumerate() {
            budget.charge_work(1)?;
            header.validate_receipt()?;
            budget.retain_bytes(
                header
                    .direct
                    .parameters
                    .len()
                    .saturating_mul(4 * (7 + 2 * std::mem::size_of::<Option<usize>>())),
            )?;
            if positions.insert(header.owner, index).is_some() {
                return Err("alignment SCC repeats a local header".into());
            }
        }
        if headers.len() != id.0.len() || id.0.iter().any(|owner| !positions.contains_key(owner)) {
            return Err("alignment SCC changed its exact local header inventory".into());
        }
        let mut imported = HashMap::new();
        let mut external_ids = HashSet::new();
        for summary in &external {
            budget.charge_work(1)?;
            summary.validate_receipt()?;
            if !topology.outgoing.contains(&summary.component)
                || !external_ids.insert(summary.component.clone())
            {
                return Err("alignment SCC repeated or added an external summary".into());
            }
            for (owner, requirement) in summary.members.iter() {
                budget.charge_work(1)?;
                if imported.insert(*owner, requirement).is_some() {
                    return Err("alignment SCC external summaries overlap".into());
                }
            }
        }
        if external_ids.len() != topology.outgoing.len() {
            return Err("alignment SCC omitted an external summary".into());
        }
        let mut states: Vec<_> = headers.iter().map(|header| header.direct.clone()).collect();
        let mut reverse = vec![Vec::new(); headers.len()];
        for (caller, header) in headers.iter().enumerate() {
            for transfer in header.transfers.iter() {
                budget.charge_work(1)?;
                let expected = if let Some(&callee) = positions.get(&transfer.callee) {
                    budget.retain_bytes(std::mem::size_of::<usize>())?;
                    reverse[callee].push(caller);
                    states[callee].parameters.len()
                } else {
                    imported
                        .get(&transfer.callee)
                        .ok_or("alignment SCC omitted an outgoing callee")?
                        .parameters
                        .len()
                };
                if expected != transfer.arguments.len() {
                    return Err("alignment SCC transfer changed exact callee arity".into());
                }
            }
        }
        let mut pending: VecDeque<_> = (0..headers.len()).collect();
        let mut queued = vec![true; headers.len()];
        while let Some(caller) = pending.pop_front() {
            budget.charge_work(1)?;
            queued[caller] = false;
            let mut next = headers[caller].direct.clone();
            for transfer in headers[caller].transfers.iter() {
                budget.charge_work(1)?;
                let required = if let Some(&callee) = positions.get(&transfer.callee) {
                    &states[callee]
                } else {
                    imported[&transfer.callee]
                };
                next.deferred_dispatch |= required.deferred_dispatch;
                if required.unsupported.is_some() {
                    next.unsupported = required.unsupported;
                }
                for (slot, argument) in transfer.arguments.iter().enumerate() {
                    budget.charge_work(1)?;
                    if required.parameters[slot] {
                        next.merge(&argument.0, budget)?;
                    }
                    if required.raw_pointees[slot] {
                        next.merge(&argument.1, budget)?;
                    }
                    if required.drop_parameters[slot] {
                        next.merge(&argument.7, budget)?;
                    }
                    if let Some(limit) = required.semantic_limits[slot] {
                        argument.2.apply(limit, &mut next, budget)?;
                    }
                    if let Some(limit) = required.physical_limits[slot] {
                        argument.2.apply_physical(limit, &mut next, budget)?;
                    }
                    if required.metadata_pointees[slot] {
                        next.merge(&argument.6, budget)?;
                    }
                    if required.thin_pointees[slot] {
                        next.merge(&argument.3, budget)?;
                    }
                    if required.pointer_strides[slot] {
                        next.merge(&argument.4, budget)?;
                    }
                    if required.mut_ref_pointees[slot] {
                        next.merge(&argument.5, budget)?;
                    }
                }
            }
            if states[caller].merge(&next, budget)? {
                for &parent in &reverse[caller] {
                    budget.charge_work(1)?;
                    if !queued[parent] {
                        queued[parent] = true;
                        pending.push_back(parent);
                    }
                }
            }
        }
        let mut members = Vec::new();
        for &owner in id.0.iter() {
            budget.charge_work(1)?;
            budget.retain_bytes(std::mem::size_of::<(ir::DeclId, Requirement)>())?;
            members.push((owner, states[positions[&owner]].clone()));
        }
        let members: Arc<[(ir::DeclId, Requirement)]> = members.into();
        Ok(Self {
            component: id.clone(),
            original_members: members.clone(),
            members,
        })
    }
    #[cfg(test)]
    pub(in crate::compiler) fn component(&self) -> &ComponentId {
        &self.component
    }
    pub(in crate::compiler) fn admit_symbolic(&self, owner: ir::DeclId) -> Result<(), String> {
        self.requirement(owner)?.admit()
    }
    fn validate_receipt(&self) -> Result<(), String> {
        if self.members != self.original_members
            || self.members.len() != self.component.0.len()
            || self
                .members
                .iter()
                .map(|(owner, _)| owner)
                .ne(self.component.0.iter())
        {
            return Err("alignment SCC changed its immutable finite solve receipt".into());
        }
        Ok(())
    }
    fn requirement(&self, owner: ir::DeclId) -> Result<&Requirement, String> {
        self.validate_receipt()?;
        self.members
            .iter()
            .find(|(member, _)| *member == owner)
            .map(|(_, requirement)| requirement)
            .ok_or_else(|| "alignment summary changed its exact member identity".into())
    }
    pub(in crate::compiler) fn closed_requirements(
        &self,
        target: &ir::CallTarget,
        facts: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<bool, String> {
        if facts.target() != Some(target) {
            return Err("alignment summary changed its closed fact subject".into());
        }
        facts
            .service()
            .discharge_direct(target, facts)
            .map_err(|error| error.into_legacy_diagnostic(budget))?;
        let required = self.requirement(target.declaration)?;
        required.admit()?;
        if target.type_args.len() != required.parameters.len() {
            return Err("alignment summary changed closed argument arity".into());
        }
        let mut closed = Requirement::empty(0, budget)?;
        for (slot, argument) in target.type_args.iter().enumerate() {
            budget.charge_work(1)?;
            if required.parameters[slot] {
                super::normalize(argument, &[], &mut closed, budget, 0)?;
            }
            if required.raw_pointees[slot] {
                super::normalize_raw_pointee(argument, &[], &mut closed, budget)?;
            }
            if required.drop_parameters[slot] {
                super::normalize_drop(argument, facts, &[], &mut closed, budget, 0)?;
            }
            if let Some(limit) = required.semantic_limits[slot] {
                super::normalize_semantic_size(argument, &[], limit, &mut closed, budget)?;
            }
            if let Some(limit) = required.physical_limits[slot] {
                super::normalize_physical_size(argument, &[], limit, &mut closed, budget)?;
            }
            if required.metadata_pointees[slot] {
                super::normalize_metadata_pointee(argument, &[], &mut closed, budget)?;
            }
            if required.thin_pointees[slot] {
                super::normalize_thin_pointee(argument, &[], &mut closed, budget)?;
            }
            if required.pointer_strides[slot] {
                super::normalize_pointer_stride(argument, &[], &mut closed, budget, 0)?;
            }
            if required.mut_ref_pointees[slot] {
                super::normalize_mut_ref_pointee(argument, &[], &mut closed, budget)?;
            }
        }
        closed.admit()?;
        Ok(required.deferred_dispatch || closed.deferred_dispatch)
    }
}
