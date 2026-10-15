//! A deliberately scoped weighted graph of native fork/join task segments.
//! It is not a query DAG, compilation span, or estimate of available parallelism.
use crate::execution_graph_continuation::{PoolInventory, TaskEdge};
use crate::execution_graph_timing::ObservedCpuAssessment;
use anyhow::{Result, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Serialize)]
pub(crate) struct TaskDag {
    pub scope: &'static str,
    pub topology_complete: bool,
    pub timing_records_complete: bool,
    pub records_complete: bool,
    pub query_authority_valid: bool,
    pub attached_cpu_ns: Option<u64>,
    pub known_attached_cpu_ns: Option<u64>,
    pub excluded_task_null_cpu_ns: Option<u64>,
    pub excluded_task_null_segments: usize,
    pub excluded_query_cpu_ns: Option<u64>,
    pub excluded_query_segments: usize,
    pub excluded_opaque_cpu_ns: Option<u64>,
    pub excluded_opaque_segments: usize,
    pub unmeasured_segments: usize,
    pub nodes: Vec<Node>,
    pub edges: Vec<TaskEdge>,
    pub components: Vec<Component>,
    pub issues: Vec<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Serialize)]
pub(crate) struct Node {
    pub task: u64,
    pub cpu_ns: Option<u64>,
    pub known_cpu_ns: Option<u64>,
    pub measured_segments: usize,
    pub unavailable_segments: usize,
}
#[derive(Debug, Serialize)]
pub(crate) struct Component {
    pub root: u64,
    pub tasks: usize,
    pub edges: usize,
    pub records_complete: bool,
    pub cpu_ns: Option<u64>,
    pub known_cpu_ns: Option<u64>,
    pub longest_recorded_fork_join_chain_cpu_ns: Option<u64>,
    pub known_cpu_chain_ns: Option<u64>,
    pub chain: Vec<u64>,
}
fn add(sum: &mut u64, value: u64) -> Result<()> {
    *sum = sum
        .checked_add(value)
        .ok_or_else(|| anyhow::anyhow!("task DAG CPU sum overflow"))?;
    Ok(())
}

pub(crate) fn assess(
    inventory: Option<&PoolInventory>,
    timing: Option<&ObservedCpuAssessment>,
) -> Result<TaskDag> {
    let topology_complete = inventory.is_some_and(|p| p.structural_complete);
    let timing_records_complete = timing.is_some_and(|t| t.local_records_complete);
    let query_authority_valid =
        timing.is_some_and(|t| t.query_execution_cpu_ns.is_none() || t.query_clock_aligned);
    let endpoints_valid = timing.is_some_and(|t| t.endpoints_valid);
    let mut report = TaskDag {
        scope: "observed-fork-join-task-dag",
        topology_complete,
        timing_records_complete,
        records_complete: topology_complete && timing_records_complete && query_authority_valid,
        query_authority_valid,
        attached_cpu_ns: None,
        known_attached_cpu_ns: None,
        excluded_task_null_cpu_ns: None,
        excluded_task_null_segments: 0,
        excluded_query_cpu_ns: None,
        excluded_query_segments: 0,
        excluded_opaque_cpu_ns: None,
        excluded_opaque_segments: 0,
        unmeasured_segments: 0,
        nodes: Vec::new(), edges: Vec::new(), components: Vec::new(),
        issues: Vec::new(),
        limitations: vec!["only native fork/join prerequisites and retained exclusive CPU, not full work/span or a parallelism bound".into(), "task-null CPU and unsampled instrumentation gaps are excluded, never zero-cost coverage".into(), "known chains use only qualified measured CPU contributions, unknown node costs are omitted, not certified zero".into()],
    };
    if !topology_complete {
        report.records_complete = false;
        report
            .issues
            .push("unique complete original pool task lifecycle authority unavailable".into());
        return Ok(report);
    }
    let pool = inventory.expect("complete inventory");
    let mut nodes: BTreeMap<u64, Node> = pool
        .task_nodes
        .iter()
        .map(|&task| {
            (
                task,
                Node {
                    task,
                    cpu_ns: None,
                    known_cpu_ns: None,
                    measured_segments: 0,
                    unavailable_segments: 0,
                },
            )
        })
        .collect();
    let mut null_cpu = 0;
    let mut null_unavailable = 0;
    let mut null_measured = 0;
    let mut null_query_cpu = 0;
    let mut null_query_measured = 0;
    let mut null_query_unavailable = 0;
    let mut null_opaque_cpu = 0;
    let mut null_opaque_measured = 0;
    let mut null_opaque_unavailable = 0;
    if let Some(timing) = timing {
        for row in &timing.validated_segments {
            let query = row.query_context.is_some();
            let known = endpoints_valid && row.measured && (!query || query_authority_valid);
            if !row.measured {
                report.unmeasured_segments += 1;
            }
            if let Some(task) = row.task {
                let node = nodes
                    .get_mut(&task)
                    .ok_or_else(|| anyhow::anyhow!("timing task not in validated DAG"))?;
                if known {
                    let sum = node.known_cpu_ns.get_or_insert(0);
                    add(sum, row.cpu_ns.expect("measured validated row"))?;
                    node.measured_segments += 1;
                } else {
                    node.unavailable_segments += 1;
                }
            } else {
                report.excluded_task_null_segments += 1;
                if query {
                    report.excluded_query_segments += 1;
                } else {
                    report.excluded_opaque_segments += 1;
                }
                if known {
                    let cpu = row.cpu_ns.expect("measured validated row");
                    add(&mut null_cpu, cpu)?;
                    null_measured += 1;
                    if query {
                        add(&mut null_query_cpu, cpu)?;
                        null_query_measured += 1;
                    } else {
                        add(&mut null_opaque_cpu, cpu)?;
                        null_opaque_measured += 1;
                    }
                } else {
                    null_unavailable += 1;
                    if query {
                        null_query_unavailable += 1;
                    } else {
                        null_opaque_unavailable += 1;
                    }
                }
            }
        }
    }
    let mut attached = 0;
    let mut measured = 0;
    for node in nodes.values_mut() {
        measured += node.measured_segments;
        if let Some(cpu) = node.known_cpu_ns {
            add(&mut attached, cpu)?;
        }
        if report.records_complete && endpoints_valid && node.unavailable_segments == 0 {
            // A complete retained row inventory can establish an empty retained sum,
            // but does not establish zero CPU outside the sampling intervals.
            node.cpu_ns = Some(node.known_cpu_ns.unwrap_or(0));
        }
    }
    report.records_complete &= endpoints_valid;
    report.known_attached_cpu_ns = (measured > 0).then_some(attached);
    report.attached_cpu_ns = report.records_complete.then_some(attached);
    report.excluded_task_null_cpu_ns = (endpoints_valid
        && null_unavailable == 0
        && (null_measured > 0 || timing_records_complete))
        .then_some(null_cpu);
    report.excluded_query_cpu_ns = (endpoints_valid
        && null_query_unavailable == 0
        && (null_query_measured > 0 || timing_records_complete && query_authority_valid))
        .then_some(null_query_cpu);
    report.excluded_opaque_cpu_ns = (endpoints_valid
        && null_opaque_unavailable == 0
        && (null_opaque_measured > 0 || timing_records_complete))
        .then_some(null_opaque_cpu);
    report.edges = pool.task_edges.clone();
    let mut forward: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    let mut undirected: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    let mut indegrees: BTreeMap<u64, usize> = nodes.keys().map(|&id| (id, 0)).collect();
    for edge in &report.edges {
        ensure!(
            nodes.contains_key(&edge.from) && nodes.contains_key(&edge.to),
            "foreign DAG edge"
        );
        forward.entry(edge.from).or_default().insert(edge.to);
        undirected.entry(edge.from).or_default().insert(edge.to);
        undirected.entry(edge.to).or_default().insert(edge.from);
        *indegrees.get_mut(&edge.to).expect("validated endpoint") += 1;
    }
    let mut ready: BTreeSet<u64> = indegrees
        .iter()
        .filter_map(|(&id, &n)| (n == 0).then_some(id))
        .collect();
    let roots = ready.clone();
    let mut best: BTreeMap<u64, u64> = BTreeMap::new();
    let mut previous = BTreeMap::new();
    let mut visited = 0;
    while let Some(id) = ready.pop_first() {
        visited += 1;
        let mut cost = best.get(&id).copied().unwrap_or(0);
        add(&mut cost, nodes[&id].known_cpu_ns.unwrap_or(0))?;
        best.insert(id, cost);
        if let Some(children) = forward.get(&id) {
            for &child in children {
                let old = best.get(&child).copied();
                if old.is_none_or(|old| cost > old)
                    || old == Some(cost) && previous.get(&child).is_none_or(|&old| id < old)
                {
                    best.insert(child, cost);
                    previous.insert(child, id);
                }
                let degree = indegrees.get_mut(&child).expect("validated endpoint");
                *degree -= 1;
                if *degree == 0 {
                    ready.insert(child);
                }
            }
        }
    }
    ensure!(visited == nodes.len(), "cyclic task DAG");
    let mut remaining: BTreeSet<u64> = nodes.keys().copied().collect();
    while let Some(first) = remaining.pop_first() {
        let mut members = BTreeSet::from([first]);
        let mut queue = vec![first];
        while let Some(id) = queue.pop() {
            if let Some(neighbors) = undirected.get(&id) {
                for &other in neighbors {
                    if remaining.remove(&other) {
                        members.insert(other);
                        queue.push(other);
                    }
                }
            }
        }
        let root = *members
            .intersection(&roots)
            .next()
            .expect("acyclic component root");
        let end = *members
            .iter()
            .max_by_key(|&&id| (best[&id], std::cmp::Reverse(id)))
            .expect("nonempty component");
        let mut sum = 0;
        let mut rows = 0;
        for id in &members {
            add(&mut sum, nodes[id].known_cpu_ns.unwrap_or(0))?;
            rows += nodes[id].measured_segments;
        }
        let known = rows > 0 && endpoints_valid;
        let mut chain = Vec::new();
        if known {
            let mut cursor = Some(end);
            while let Some(id) = cursor {
                chain.push(id);
                cursor = previous.get(&id).copied();
            }
            chain.reverse();
        }
        report.components.push(Component {
            root,
            tasks: members.len(),
            edges: members
                .iter()
                .map(|id| forward.get(id).map_or(0, BTreeSet::len))
                .sum(),
            records_complete: report.records_complete,
            cpu_ns: report.records_complete.then_some(sum),
            known_cpu_ns: known.then_some(sum),
            longest_recorded_fork_join_chain_cpu_ns: (report.records_complete && known)
                .then_some(best[&end]),
            known_cpu_chain_ns: known.then_some(best[&end]),
            chain,
        });
    }
    if !report.records_complete {
        report.issues.push("partial timing inventory, unavailable measurements, or query authority: exact retained weights/chains unknown".into());
    }
    report.nodes = nodes.into_values().collect();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture() -> Value {
        let mut pool = crate::execution_graph_continuation::tests::schema3();
        for event in pool["events"].as_array_mut().unwrap() {
            event["at_ns"] = json!(event["at_ns"].as_u64().unwrap() * 100);
        }
        let rows: Vec<_> = [(1, 110, 2), (2, 410, 5), (3, 410, 5), (4, 910, 3)].into_iter().map(|(task, start, cpu)| json!({
            "segment_id":task,"thread_id":task,"task":task,"owner_kind":"task",
            "start_ns":start,"end_ns":start+30,"cpu_start_ns":100,"cpu_end_ns":100+cpu,"cpu_ns":cpu,"status":"measured"
        })).collect();
        pool["timing"]["segments"] = json!(rows);
        pool["timing"]["segment_allocator_high_water"] = json!(5);
        pool["timing"]["expected_segments"] = json!(4);
        pool["timing"]["thread_allocator_high_water"] = json!(5);
        pool
    }
    fn collect(pool: &Value) -> Result<TaskDag> {
        let bytes = serde_json::to_vec(pool)?;
        let local = crate::execution_graph_continuation::collect_task_inventory(
            &bytes,
            pool["schema"] == 4,
        )?;
        let collected = crate::execution_graph_continuation::collect_bytes_with_query_ownership(
            &bytes,
            pool["schema"] == 4,
        )?;
        assess(Some(&local), collected.assessment.observed_cpu.as_ref())
    }
    #[test]
    fn original_fork_join_graph_has_disjoint_weights_and_deterministic_chain() {
        let pool = fixture();
        let a = collect(&pool).unwrap();
        assert!(a.records_complete && a.issues.is_empty());
        assert_eq!(a.nodes.len(), 4);
        assert_eq!(
            a.edges,
            vec![
                TaskEdge {
                    from: 1,
                    to: 2,
                    kind: "fork"
                },
                TaskEdge {
                    from: 1,
                    to: 3,
                    kind: "fork"
                },
                TaskEdge {
                    from: 2,
                    to: 4,
                    kind: "join"
                },
                TaskEdge {
                    from: 3,
                    to: 4,
                    kind: "join"
                }
            ]
        );
        assert_eq!(a.attached_cpu_ns, Some(15));
        assert_eq!(
            a.components[0].longest_recorded_fork_join_chain_cpu_ns,
            Some(10)
        );
        assert_eq!(a.components[0].chain, [1, 2, 4]);
        let mut reordered = pool.clone();
        reordered["events"].as_array_mut().unwrap().reverse();
        reordered["timing"]["segments"]
            .as_array_mut()
            .unwrap()
            .reverse();
        assert_eq!(
            serde_json::to_value(a).unwrap(),
            serde_json::to_value(collect(&reordered).unwrap()).unwrap()
        );
    }
    #[test]
    fn unknown_timing_is_never_zero_cpu_and_loss_preserves_only_known_subset() {
        let original = fixture();
        let mut no_timing = original.clone();
        no_timing["schema"] = json!(2);
        no_timing.as_object_mut().unwrap().remove("timing");
        let a = collect(&no_timing).unwrap();
        assert!(a.topology_complete && !a.records_complete);
        assert!(
            a.nodes
                .iter()
                .all(|n| n.cpu_ns.is_none() && n.known_cpu_ns.is_none())
        );
        assert!(a.components[0].known_cpu_chain_ns.is_none() && a.components[0].chain.is_empty());
        let mut lost = original.clone();
        lost["timing"]["segments"].as_array_mut().unwrap().remove(1);
        lost["timing"]["lost_segments"] = json!(1);
        lost["timing"]["records_complete"] = json!(false);
        let a = collect(&lost).unwrap();
        assert!(!a.records_complete && a.nodes.iter().all(|n| n.cpu_ns.is_none()));
        assert_eq!(a.known_attached_cpu_ns, Some(10));
        assert!(a.attached_cpu_ns.is_none());
        assert_eq!(a.components[0].known_cpu_chain_ns, Some(10));
        assert!(
            a.components[0]
                .longest_recorded_fork_join_chain_cpu_ns
                .is_none()
        );
    }
    #[test]
    fn independent_roots_are_not_ordered_and_task_null_mass_is_excluded() {
        let mut pool = fixture();
        pool["task_allocator_high_water"] = json!(6);
        pool["expected_tasks"] = json!(5);
        pool["events"].as_array_mut().unwrap().extend([
            json!({"kind":"root","task":5,"at_ns":0}),
            json!({"kind":"start","task":5,"at_ns":1}),
            json!({"kind":"finish","task":5,"at_ns":1001,"outcome":"normal"}),
        ]);
        let mut row = pool["timing"]["segments"][0].clone();
        row["segment_id"] = json!(5);
        row["thread_id"] = json!(5);
        row["task"] = Value::Null;
        row["owner_kind"] = json!("opaque");
        pool["timing"]["segments"].as_array_mut().unwrap().push(row);
        pool["timing"]["segment_allocator_high_water"] = json!(6);
        pool["timing"]["thread_allocator_high_water"] = json!(6);
        pool["timing"]["expected_segments"] = json!(5);
        let a = collect(&pool).unwrap();
        assert_eq!(a.components.len(), 2);
        assert_eq!(a.edges.len(), 4);
        assert_eq!(a.attached_cpu_ns, Some(15));
        assert_eq!(a.excluded_task_null_cpu_ns, Some(2));
        assert_eq!(a.excluded_opaque_cpu_ns, Some(2));
        assert_eq!(a.nodes[4].cpu_ns, Some(0));
        assert!(a.components[1].known_cpu_chain_ns.is_none());
    }
    #[test]
    fn schema4_labels_without_admission_authority_do_not_authorize_alternate_weights() {
        let mut pool = fixture();
        pool["schema"] = json!(4);
        pool["timing"]["schema"] = json!(2);
        for row in pool["timing"]["segments"].as_array_mut().unwrap() {
            row["query_context"] = Value::Null;
            row["query_job"] = Value::Null;
            row["query_name"] = Value::Null;
        }
        let row = &mut pool["timing"]["segments"][0];
        row["owner_kind"] = json!("query_execution");
        row["query_context"] = json!(1);
        row["query_job"] = json!(1);
        row["query_name"] = json!("type_of");
        let a = collect(&pool).unwrap();
        assert!(a.topology_complete && !a.query_authority_valid && !a.records_complete);
        assert_eq!(a.known_attached_cpu_ns, Some(13));
        assert!(a.nodes[0].known_cpu_ns.is_none() && a.nodes[0].cpu_ns.is_none());
        assert_eq!(a.nodes[0].unavailable_segments, 1);
        assert_eq!(a.components[0].known_cpu_chain_ns, Some(8));
        assert!(
            a.components[0]
                .longest_recorded_fork_join_chain_cpu_ns
                .is_none()
        );
    }
    #[test]
    fn original_byte_validation_rejects_shortcuts_foreign_tasks_and_overlap() {
        for (pointer, value) in [
            ("/events/9/child", json!(3)),
            ("/timing/segments/0/task", json!(99)),
            ("/timing/segments/1/start_ns", json!(0)),
            ("/timing/segments/1/thread_id", json!(3)),
        ] {
            let mut pool = fixture();
            *pool.pointer_mut(pointer).unwrap() = value;
            assert!(collect(&pool).is_err(), "{pointer}");
        }
    }
}
