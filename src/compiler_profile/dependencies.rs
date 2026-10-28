//! Post-run structural diagnostics, not a proof of unnecessary dependencies.
use super::{json_string, phase};
use justrust_engine::Report;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Write};

const ROW_LIMIT: usize = 128;
const CLOSURE_EDGE_VISIT_LIMIT: usize = 4_000_000;

type Adjacency = BTreeMap<u64, BTreeSet<u64>>;

struct Dependencies {
    inputs: Adjacency,
    consumers: Adjacency,
    unattributed_edges: usize,
    missing_producers: usize,
    conflicting_task_owners: usize,
}

impl Dependencies {
    fn from_report(report: &Report) -> Self {
        let known: BTreeSet<_> = report.node_ids.iter().copied().collect();
        let mut owners = HashMap::new();
        let mut conflicts = BTreeSet::new();
        for event in &report.events {
            if let (Some(task), Some(node)) = (event.task, event.node)
                && let Some(previous) = owners.insert(task, node)
                && previous != node
            {
                conflicts.insert(task);
            }
        }
        let mut result = Self {
            inputs: BTreeMap::new(),
            consumers: BTreeMap::new(),
            unattributed_edges: 0,
            missing_producers: 0,
            conflicting_task_owners: conflicts.len(),
        };
        for &(task, producer, _) in &report.dependency_edges {
            let Some(&consumer) = owners.get(&task).filter(|_| !conflicts.contains(&task)) else {
                result.unattributed_edges += 1;
                continue;
            };
            if !known.contains(&consumer) {
                result.unattributed_edges += 1;
                continue;
            }
            if !known.contains(&producer) {
                result.missing_producers += 1;
                continue;
            }
            // Deduplicate output slots and forked consumer branches. Self-reads
            // remain direct edges; reachability below excludes the starting node.
            result.inputs.entry(consumer).or_default().insert(producer);
            result
                .consumers
                .entry(producer)
                .or_default()
                .insert(consumer);
        }
        result
    }

    fn complete(&self) -> bool {
        self.unattributed_edges == 0
            && self.missing_producers == 0
            && self.conflicting_task_owners == 0
    }
}

fn degree(edges: &Adjacency, node: u64) -> usize {
    edges.get(&node).map_or(0, BTreeSet::len)
}

/// Returns a lower bound if the shared traversal budget is exhausted.
fn reachable(edges: &Adjacency, start: u64, remaining: &mut usize) -> (usize, bool) {
    let mut visited = BTreeSet::from([start]);
    let mut stack = vec![start];
    while let Some(node) = stack.pop() {
        if let Some(neighbors) = edges.get(&node) {
            for &next in neighbors {
                if *remaining == 0 {
                    return (visited.len() - 1, false);
                }
                *remaining -= 1;
                if visited.insert(next) {
                    stack.push(next);
                }
            }
        }
    }
    (visited.len() - 1, true)
}

fn wait_intervals(report: &Report) -> (BTreeMap<u64, u64>, usize) {
    let mut suspended = HashMap::new();
    let mut times = BTreeMap::<u64, u64>::new();
    let mut unmatched = 0;
    for event in &report.events {
        let (Some(task), Some(node)) = (event.task, event.node) else {
            continue;
        };
        if event.kind == "suspended" {
            if suspended.insert(task, (node, event.at_ns)).is_some() {
                unmatched += 1;
            }
        } else if matches!(event.kind.as_str(), "queued" | "task-done" | "running")
            && let Some((owner, start)) = suspended.remove(&task)
        {
            let value = times.entry(owner).or_default();
            *value = value.saturating_add(event.at_ns.saturating_sub(start));
        }
    }
    (times, unmatched + suspended.len())
}

pub(super) fn write(
    out: &mut impl Write,
    report: &Report,
    execution: &BTreeMap<u64, u64>,
    unmatched_execution: usize,
) -> io::Result<()> {
    let graph = Dependencies::from_report(report);
    let (waits, unmatched_waits) = wait_intervals(report);
    let mut rows: Vec<_> = report
        .node_ids
        .iter()
        .zip(&report.node_evaluations)
        .collect();
    // A transparent structural ranking, not an invented speedup estimate.
    // Canonical keys break ties before ephemeral node IDs.
    rows.sort_by(|(a, (ka, _)), (b, (kb, _))| {
        degree(&graph.consumers, **b)
            .cmp(&degree(&graph.consumers, **a))
            .then_with(|| degree(&graph.inputs, **b).cmp(&degree(&graph.inputs, **a)))
            .then_with(|| execution.get(b).cmp(&execution.get(a)))
            .then_with(|| ka.kind.cmp(&kb.kind))
            .then_with(|| ka.version.cmp(&kb.version))
            .then_with(|| ka.params.cmp(&kb.params))
            .then_with(|| a.cmp(b))
    });
    write!(
        out,
        "{{\"edge_attribution_complete\":{},\"unattributed_edges\":{},\"missing_producers\":{},\"conflicting_task_owners\":{},\"intervals_complete\":{},\"unmatched_wait_intervals\":{},\"closure_edge_visit_limit\":{CLOSURE_EDGE_VISIT_LIMIT},\"top_nodes_limit\":{ROW_LIMIT},\"omitted_nodes\":{},\"ranking\":\"direct consumers, direct dependencies, summed execution wall descending; canonical key tie-break\",\"interpretation\":\"observed read edges, not proof of necessity; node-level reachability merges output slots and can include early-publication cycles; wait is each consumer's own suspended wall, not time attributable to a producer or critical path; incomplete counts are lower bounds\",\"top_nodes\":[",
        graph.complete(),
        graph.unattributed_edges,
        graph.missing_producers,
        graph.conflicting_task_owners,
        report.capture_complete && unmatched_execution == 0 && unmatched_waits == 0,
        unmatched_waits,
        rows.len().saturating_sub(ROW_LIMIT)
    )?;
    let mut remaining = CLOSURE_EDGE_VISIT_LIMIT;
    for (index, (id, (key, _))) in rows.into_iter().take(ROW_LIMIT).enumerate() {
        if index != 0 {
            write!(out, ",")?;
        }
        let (inputs, inputs_complete) = reachable(&graph.inputs, *id, &mut remaining);
        let (consumers, consumers_complete) = reachable(&graph.consumers, *id, &mut remaining);
        write!(
            out,
            "{{\"id\":{id},\"kind\":{},\"version\":{},\"phase\":",
            key.kind, key.version
        )?;
        json_string(out, phase(key.kind))?;
        write!(
            out,
            ",\"direct_dependencies\":{},\"direct_consumers\":{},\"transitive_dependencies\":{inputs},\"transitive_consumers\":{consumers},\"closure_complete\":{},\"summed_wall_ns\":{},\"summed_wait_ns\":{},\"params\":[",
            degree(&graph.inputs, *id),
            degree(&graph.consumers, *id),
            graph.complete() && inputs_complete && consumers_complete,
            execution.get(id).copied().unwrap_or(0),
            waits.get(id).copied().unwrap_or(0)
        )?;
        for (index, param) in key.params.iter().enumerate() {
            if index != 0 {
                write!(out, ",")?;
            }
            write!(out, "{param}")?;
        }
        write!(out, "]}}")?;
    }
    write!(
        out,
        "],\"closure_edge_visits\":{}}}",
        CLOSURE_EDGE_VISIT_LIMIT - remaining
    )
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // Report carries private engine authority.
mod tests {
    use super::*;
    use justrust_engine::{Event, Key};

    fn event(at_ns: u64, kind: &str, task: u64, node: u64) -> Event {
        Event {
            at_ns,
            kind: kind.into(),
            task: Some(task),
            node: Some(node),
            demand: None,
            detail: String::new(),
        }
    }

    #[test]
    fn diamond_deduplicates_slots_and_branches_and_excludes_self_from_cycles() {
        let mut report = Report::default();
        report.node_ids = vec![1, 2, 3, 4];
        report.events = vec![
            event(0, "queued", 10, 1),
            event(0, "queued", 11, 1),
            event(0, "queued", 20, 2),
            event(0, "queued", 30, 3),
            event(0, "queued", 40, 4),
        ];
        report.dependency_edges = vec![
            (10, 2, 0),
            (10, 2, 1),
            (11, 2, 0),
            (10, 3, 0),
            (20, 4, 0),
            (30, 4, 0),
            (40, 1, 0),
        ];
        let graph = Dependencies::from_report(&report);
        assert!(graph.complete());
        assert_eq!(degree(&graph.inputs, 1), 2);
        assert_eq!(degree(&graph.consumers, 4), 2);
        assert_eq!(reachable(&graph.inputs, 1, &mut 100), (3, true));
        assert_eq!(reachable(&graph.consumers, 4, &mut 100), (3, true));
        assert_eq!(reachable(&graph.inputs, 1, &mut 0), (0, false));
    }

    #[test]
    fn missing_and_conflicting_attribution_is_not_reported_as_exact() {
        let mut report = Report::default();
        report.node_ids = vec![1, 2];
        report.events = vec![
            event(0, "queued", 10, 1),
            event(0, "queued", 10, 2),
            event(0, "queued", 20, 2),
        ];
        report.dependency_edges = vec![(10, 2, 0), (99, 1, 0), (20, 777, 0)];
        let graph = Dependencies::from_report(&report);
        assert!(!graph.complete());
        assert_eq!(graph.unattributed_edges, 2);
        assert_eq!(graph.missing_producers, 1);
        assert_eq!(graph.conflicting_task_owners, 1);
    }

    #[test]
    fn wait_intervals_are_per_branch_not_attributed_to_producers() {
        let mut report = Report::default();
        report.events = vec![
            event(10, "suspended", 1, 7),
            event(12, "suspended", 2, 7),
            event(20, "queued", 1, 7),
            event(25, "queued", 2, 7),
            event(30, "suspended", 3, 9),
        ];
        let (waits, unmatched) = wait_intervals(&report);
        assert_eq!(waits[&7], 23);
        assert_eq!(unmatched, 1);
    }

    #[test]
    fn ranking_is_bounded_and_partial_capture_is_explicit() {
        let mut report = Report::default();
        for id in 1..=140 {
            report.node_ids.push(id);
            report.node_evaluations.push((Key::new(143, vec![id]), 1));
            report.events.push(event(0, "queued", id, id));
            if id > 1 {
                report.dependency_edges.push((id, 1, 0));
            }
        }
        let mut output = Vec::new();
        write(&mut output, &report, &BTreeMap::new(), 0).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("\"omitted_nodes\":12"));
        assert!(text.contains("\"intervals_complete\":false"));
        assert!(text.contains("\"edge_attribution_complete\":true"));
        assert!(text.contains("\"top_nodes\":[{\"id\":1,"));
        assert!(text.contains("\"direct_consumers\":139"));
        assert_eq!(text.matches("\"closure_complete\":").count(), ROW_LIMIT);
    }
}
