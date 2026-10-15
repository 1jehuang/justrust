//! Independent validation of native execution graph receipts.
//! A structurally valid Cargo-unit receipt is not a complete compiler graph.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;
use std::path::Path;

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
const MAX_NODES: usize = 100_000;
const MAX_EDGES: usize = 1_000_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    producer: String,
    scope: String,
    capture_id: String,
    structural_complete: bool,
    full_pipeline_complete: bool,
    issues: Vec<String>,
    declared_nodes: usize,
    declared_edges: usize,
    events: Vec<Event>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    opaque_boundaries: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    id: u64,
    kind: String,
    start_ns: Option<u64>,
    end_ns: Option<u64>,
    outcome: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Edge {
    from: u64,
    to: u64,
    artifact: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    unit: u64,
    event: String,
    at_ns: u64,
    #[serde(default)]
    artifact: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
}

#[derive(Debug, Serialize)]
struct Assessment {
    schema: u32,
    producer: String,
    scope: String,
    capture_id: String,
    structural_complete: bool,
    full_pipeline_complete: bool,
    nodes: usize,
    edges: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    descriptive_parent_edges: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    control_causal_edges: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_structural_complete: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    waits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query_reads: Option<crate::execution_graph_reads::ReadAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_call_provenance: Option<crate::execution_graph_reads::CacheCallAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dispatch_result_provenance: Option<crate::execution_graph_reads::DispatchResultAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_bindings: Option<crate::execution_graph_task_bindings::BindingAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_cpu: Option<crate::execution_graph_timing::ObservedCpuAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query_cpu_ownership: Option<crate::execution_graph_timing::QueryCpuOwnership>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_task_dag: Option<crate::execution_graph_task_dag::TaskDag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_flow_dag: Option<crate::execution_graph_flow::FlowAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latch_readiness_dag: Option<crate::execution_graph_readiness::ReadinessAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    clock_alignment: Option<crate::execution_graph_query::ClockAlignmentAssessment>,
    /// Strict native prefix/endorsement assessment, never compiler-launch authority.
    #[serde(skip_serializing_if = "Option::is_none")]
    resource: Option<serde_json::Value>,
    /// Original-byte directory reconciliation is emitted once to bound output.
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_attachments: Option<crate::execution_graph_resource_attachment::AttachmentAssessment>,
    issues: Vec<String>,
    opaque_boundaries: Vec<String>,
    /// No CPU or execution-span estimate is inferred from elapsed lifetimes.
    cpu_work_ns: Option<u64>,
    execution_span_ns: Option<u64>,
}

fn assess(r: Receipt) -> Result<Assessment> {
    if r.schema != 1 || r.producer != "cargo-scheduler" || r.scope != "cargo-unit-scheduler" {
        bail!("unsupported execution graph receipt schema, producer or scope");
    }
    if r.nodes.len() > MAX_NODES || r.edges.len() > MAX_EDGES {
        bail!("execution graph exceeds node or edge budget");
    }
    let mut issues = r.issues;
    if r.declared_nodes != r.nodes.len() || r.declared_edges != r.edges.len() {
        issues.push("declaration counts do not match recorded graph".into());
    }
    if r.capture_id.is_empty() {
        issues.push("missing capture identity".into());
    }
    if r.full_pipeline_complete {
        issues.push("Cargo-unit producer cannot attest full pipeline completeness".into());
    }
    if r.opaque_boundaries.is_empty() {
        issues.push("missing explicit opaque boundaries".into());
    }
    let mut nodes = BTreeMap::new();
    for n in &r.nodes {
        if nodes.insert(n.id, n).is_some() {
            issues.push(format!("duplicate node {}", n.id));
        }
        if n.kind != "cargo-unit" {
            issues.push(format!("node {} is not a declared Cargo unit", n.id));
        }
        let lifecycle_ok = match n.outcome.as_deref() {
            Some("not_started") => n.start_ns.is_none() && n.end_ns.is_none(),
            Some("success" | "failure" | "fresh" | "cancelled") => {
                matches!((n.start_ns, n.end_ns), (Some(start), Some(end)) if end >= start)
            }
            _ => false,
        };
        if !lifecycle_ok {
            issues.push(format!("node {} has incomplete or invalid lifecycle", n.id));
        }
    }
    if r.events.len() > MAX_EDGES {
        bail!("execution graph exceeds event budget");
    }
    let mut starts: BTreeMap<u64, usize> = BTreeMap::new();
    let mut terminals: BTreeMap<u64, usize> = BTreeMap::new();
    let mut ready_artifacts: BTreeMap<(u64, &str), (u64, usize)> = BTreeMap::new();
    let mut start_order = BTreeMap::new();
    let mut terminal_order = BTreeMap::new();
    let mut stages: BTreeMap<u64, u8> = BTreeMap::new();
    let mut previous_event_ns = 0;
    for (event_index, e) in r.events.iter().enumerate() {
        if e.at_ns < previous_event_ns {
            issues.push("event timestamps are not monotonic".into());
        }
        previous_event_ns = e.at_ns;
        let Some(node) = nodes.get(&e.unit) else {
            issues.push("event has missing node".into());
            continue;
        };
        match e.event.as_str() {
            "start" => {
                let stage = stages.entry(e.unit).or_default();
                if *stage != 0 {
                    issues.push("start event violates lifecycle order".into());
                }
                *stage = 1;
                start_order.insert(e.unit, event_index);
                *starts.entry(e.unit).or_default() += 1;
                if node.start_ns != Some(e.at_ns) || e.artifact.is_some() || e.outcome.is_some() {
                    issues.push("start event disagrees with node lifecycle".into());
                }
            }
            "terminal" => {
                let stage = stages.entry(e.unit).or_default();
                let expected = if node.outcome.as_deref() == Some("not_started") {
                    0
                } else {
                    1
                };
                if *stage != expected {
                    issues.push("terminal event violates lifecycle order".into());
                }
                *stage = 2;
                terminal_order.insert(e.unit, event_index);
                *terminals.entry(e.unit).or_default() += 1;
                if e.outcome != node.outcome
                    || e.artifact.is_some()
                    || (node.outcome.as_deref() != Some("not_started")
                        && node.end_ns != Some(e.at_ns))
                {
                    issues.push("terminal event disagrees with node lifecycle".into());
                }
            }
            "artifact-ready" => {
                if stages.get(&e.unit).copied() != Some(1) {
                    issues.push("readiness event outside started lifecycle".into());
                }
                if let Some(artifact @ "rmeta") = e.artifact.as_deref() {
                    if ready_artifacts
                        .insert((e.unit, artifact), (e.at_ns, event_index))
                        .is_some()
                    {
                        issues.push("duplicate artifact readiness event".into());
                    }
                    if !matches!((node.start_ns, node.end_ns), (Some(start), Some(end)) if start <= e.at_ns && e.at_ns <= end)
                        || e.outcome.is_some()
                    {
                        issues.push("artifact readiness outside task lifetime".into());
                    }
                } else {
                    issues.push("unknown readiness artifact".into());
                }
            }
            _ => issues.push("unknown lifecycle event".into()),
        }
    }
    for node in &r.nodes {
        let expected_starts = usize::from(node.outcome.as_deref() != Some("not_started"));
        if starts.get(&node.id).copied().unwrap_or(0) != expected_starts
            || terminals.get(&node.id).copied().unwrap_or(0) != 1
        {
            issues.push(format!(
                "node {} has missing or duplicate lifecycle events",
                node.id
            ));
        }
    }
    let mut indegree: BTreeMap<u64, usize> = nodes.keys().map(|&id| (id, 0)).collect();
    let mut successors: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let mut edges = BTreeSet::new();
    for e in &r.edges {
        if !matches!(e.artifact.as_str(), "all" | "rmeta") {
            issues.push("unknown prerequisite artifact".into());
        }
        if !edges.insert((e.from, e.to, e.artifact.as_str())) {
            issues.push("duplicate prerequisite edge".into());
        }
        if !nodes.contains_key(&e.from) || !nodes.contains_key(&e.to) {
            issues.push("prerequisite edge has missing endpoint".into());
            continue;
        }
        // Edges point from the dependent to its prerequisite. A metadata edge
        // may unblock before the producer finishes, preserving Cargo pipelining.
        if let Some(start) = nodes[&e.from].start_ns {
            let prerequisite = nodes[&e.to];
            let available = ready_artifacts
                .get(&(e.to, e.artifact.as_str()))
                .copied()
                .or_else(|| {
                    if matches!(prerequisite.outcome.as_deref(), Some("success" | "fresh")) {
                        prerequisite.end_ns.zip(terminal_order.get(&e.to).copied())
                    } else {
                        None
                    }
                });
            if available.is_none_or(|(ready, ready_index)| {
                ready > start
                    || start_order
                        .get(&e.from)
                        .is_none_or(|&start_index| ready_index >= start_index)
            }) {
                issues.push("dependent started without recorded prerequisite readiness".into());
            }
        }
        *indegree.get_mut(&e.to).unwrap() += 1;
        successors.entry(e.from).or_default().push(e.to);
    }
    let mut ready: VecDeque<u64> = indegree
        .iter()
        .filter_map(|(&id, &degree)| (degree == 0).then_some(id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop_front() {
        visited += 1;
        for &next in successors.get(&id).into_iter().flatten() {
            let degree = indegree.get_mut(&next).unwrap();
            *degree -= 1;
            if *degree == 0 {
                ready.push_back(next);
            }
        }
    }
    if visited != nodes.len() {
        issues.push("cyclic prerequisite graph".into());
    }
    if !r.structural_complete && issues.is_empty() {
        issues.push("producer did not attest scoped structural completeness".into());
    }
    issues.sort();
    issues.dedup();
    Ok(Assessment {
        schema: 1,
        producer: r.producer,
        scope: r.scope,
        capture_id: r.capture_id,
        structural_complete: r.structural_complete && issues.is_empty(),
        full_pipeline_complete: false,
        nodes: r.nodes.len(),
        edges: r.edges.len(),
        descriptive_parent_edges: None,
        control_causal_edges: None,
        wait_structural_complete: None,
        waits: None,
        query_reads: None,
        cache_call_provenance: None,
        dispatch_result_provenance: None,
        task_bindings: None,
        resource: None,
        resource_attachments: None,
        observed_cpu: None,
        query_cpu_ownership: None,
        observed_task_dag: None,
        execution_flow_dag: None,
        latch_readiness_dag: None,
        clock_alignment: None,
        issues,
        opaque_boundaries: r.opaque_boundaries,
        cpu_work_ns: None,
        execution_span_ns: None,
    })
}

pub(crate) fn read_bytes(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        bail!("receipt must be a regular, non-symlink file");
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > max_bytes {
        bail!("receipt is not a regular file within the byte budget");
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        bail!("receipt exceeds byte budget");
    }
    Ok(bytes)
}

#[cfg(test)]
fn read_receipt(path: &Path) -> Result<Assessment> {
    let bytes = read_bytes(path, MAX_BYTES)?;
    assess_receipt_bytes(&bytes, &BTreeMap::new())
}

/// Collect independently validated pool authorities before any query summary is
/// flattened. Duplicate identities are deliberately unavailable to all queries.
pub(crate) fn collect_pool_inventories(
    documents: &[Vec<u8>],
) -> Result<BTreeMap<String, crate::execution_graph_continuation::PoolInventory>> {
    collect_pool_inventories_with_query_ownership(documents, false)
}

fn collect_pool_inventories_with_query_ownership(
    documents: &[Vec<u8>],
    query_ownership: bool,
) -> Result<BTreeMap<String, crate::execution_graph_continuation::PoolInventory>> {
    collect_pool_inventories_with_options(documents, query_ownership, false, false)
}
fn collect_pool_inventories_with_options(
    documents: &[Vec<u8>],
    query_ownership: bool,
    execution_flow: bool,
    latch_readiness: bool,
) -> Result<BTreeMap<String, crate::execution_graph_continuation::PoolInventory>> {
    let mut pools = BTreeMap::new();
    let mut identities = BTreeMap::<String, usize>::new();
    for bytes in documents {
        let header: serde_json::Value = serde_json::from_slice(bytes)?;
        if let Some(id) = header["capture_id"].as_str() {
            *identities.entry(id.to_owned()).or_default() += 1;
        }
        if header["producer"] != "rustc-thread-pool-join" {
            continue;
        }
        let pool = crate::execution_graph_continuation::collect_bytes_with_options(
            bytes,
            query_ownership,
            execution_flow,
            latch_readiness,
        )?;
        let id = pool.assessment.capture_id;
        if let Some(inventory) = pool.inventory {
            pools.insert(id, inventory);
        }
    }
    pools.retain(|id, pool| {
        identities.get(id) == Some(&1)
            && pool
                .query_capture_id
                .as_ref()
                .is_some_and(|query_id| identities.get(query_id) == Some(&1))
    });
    Ok(pools)
}

pub(crate) fn collect_query(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
) -> Result<crate::execution_graph_query::QueryAssessment> {
    collect_query_with_policy(
        bytes,
        pools,
        crate::execution_graph_query::CapturePolicy::strict(),
    )
}

fn collect_query_with_policy(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
    policy: crate::execution_graph_query::CapturePolicy,
) -> Result<crate::execution_graph_query::QueryAssessment> {
    collect_query_with_analysis(bytes, pools, policy, false)
}

fn collect_query_with_analysis(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
    policy: crate::execution_graph_query::CapturePolicy,
    cache_call_provenance: bool,
) -> Result<crate::execution_graph_query::QueryAssessment> {
    collect_query_with_options(bytes, pools, policy, cache_call_provenance, false)
}

fn collect_query_with_options(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
    policy: crate::execution_graph_query::CapturePolicy,
    cache_call_provenance: bool,
    dispatch_result_provenance: bool,
) -> Result<crate::execution_graph_query::QueryAssessment> {
    Ok(collect_query_capture_with_options(
        bytes,
        pools,
        policy,
        cache_call_provenance,
        dispatch_result_provenance,
    )?
    .assessment)
}

fn collect_query_capture_with_options(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
    policy: crate::execution_graph_query::CapturePolicy,
    cache_call_provenance: bool,
    dispatch_result_provenance: bool,
) -> Result<crate::execution_graph_query::CollectedQuery> {
    let mut query = if dispatch_result_provenance {
        crate::execution_graph_query::collect_bytes_with_dispatch_analysis(
            bytes,
            policy,
            cache_call_provenance,
        )?
    } else {
        crate::execution_graph_query::collect_bytes_with_analysis(
            bytes,
            policy,
            cache_call_provenance,
        )?
    };
    if let Some(bindings) = &query.bindings {
        query.assessment.task_bindings =
            Some(crate::execution_graph_task_bindings::reconcile_bindings(
                &query.assessment.capture_id,
                bindings,
                pools,
            ));
    }
    query.reconcile_clock_alignment(pools);
    Ok(query)
}

fn assess_receipt_bytes(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
) -> Result<Assessment> {
    assess_receipt_bytes_with_policy(
        bytes,
        pools,
        crate::execution_graph_query::CapturePolicy::strict(),
    )
}

fn assess_receipt_bytes_with_policy(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
    policy: crate::execution_graph_query::CapturePolicy,
) -> Result<Assessment> {
    assess_receipt_bytes_with_analysis(bytes, pools, policy, false)
}

fn assess_receipt_bytes_with_analysis(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
    policy: crate::execution_graph_query::CapturePolicy,
    cache_call_provenance: bool,
) -> Result<Assessment> {
    assess_receipt_bytes_with_options(bytes, pools, policy, cache_call_provenance, false)
}

fn assess_receipt_bytes_with_options(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
    policy: crate::execution_graph_query::CapturePolicy,
    cache_call_provenance: bool,
    dispatch_result_provenance: bool,
) -> Result<Assessment> {
    if let Some(producer) = resource_producer(bytes)? {
        return assess_resource_document(bytes, &producer);
    }
    #[derive(Deserialize)]
    struct ProducerTag {
        producer: String,
    }
    let document: ProducerTag =
        serde_json::from_slice(bytes).context("invalid execution graph document")?;
    match document.producer.as_str() {
        "cargo-scheduler" => assess(serde_json::from_slice(bytes)?),
        "rustc-query-admission-entry" => {
            let entry = crate::execution_graph_compiler::parse_entry(bytes)?;
            Ok(Assessment {
                schema: entry.schema,
                producer: entry.producer,
                scope: entry.scope,
                capture_id: entry.capture_id,
                structural_complete: false,
                full_pipeline_complete: false,
                nodes: 0,
                edges: 0,
                descriptive_parent_edges: None,
                control_causal_edges: None,
                wait_structural_complete: None,
                waits: None,
                query_reads: None,
                cache_call_provenance: None,
                dispatch_result_provenance: None,
                task_bindings: None,
                resource: None,
                resource_attachments: None,
                observed_cpu: None,
                query_cpu_ownership: None,
                observed_task_dag: None,
                execution_flow_dag: None,
                latch_readiness_dag: None,
                clock_alignment: None,
                issues: vec!["requires explicit --compiler-manifest reconciliation".into()],
                opaque_boundaries: vec!["compiler-launch-authority".into()],
                cpu_work_ns: None,
                execution_span_ns: None,
            })
        }
        "rustc-query-admission" => {
            let query = collect_query_with_options(
                bytes,
                pools,
                policy,
                cache_call_provenance,
                dispatch_result_provenance,
            )?;
            Ok(Assessment {
                schema: query.schema,
                producer: query.producer,
                scope: query.scope,
                capture_id: query.capture_id,
                structural_complete: query.structural_complete,
                full_pipeline_complete: query.full_pipeline_complete,
                nodes: query.nodes,
                edges: query.edges,
                descriptive_parent_edges: Some(query.descriptive_parent_edges),
                control_causal_edges: Some(query.control_causal_edges),
                wait_structural_complete: query.wait_structural_complete,
                waits: query.waits,
                query_reads: query.query_reads,
                cache_call_provenance: query.cache_call_provenance,
                dispatch_result_provenance: query.dispatch_result_provenance,
                task_bindings: query.task_bindings,
                resource: None,
                resource_attachments: None,
                observed_cpu: None,
                query_cpu_ownership: None,
                observed_task_dag: None,
                execution_flow_dag: None,
                latch_readiness_dag: None,
                clock_alignment: query.clock_alignment,
                issues: query.issues,
                opaque_boundaries: query.opaque_boundaries,
                cpu_work_ns: query.cpu_work_ns,
                execution_span_ns: query.execution_span_ns,
            })
        }
        "rustc-thread-pool-join" => {
            let join = crate::execution_graph_continuation::collect_bytes_with_options(
                bytes,
                policy.query_cpu_ownership,
                policy.execution_flow_dag,
                policy.latch_readiness_dag,
            )?
            .assessment;
            Ok(Assessment {
                schema: join.schema,
                producer: join.producer,
                scope: join.scope,
                capture_id: join.capture_id,
                structural_complete: join.structural_complete,
                full_pipeline_complete: join.full_pipeline_complete,
                nodes: join.nodes,
                edges: join.edges,
                descriptive_parent_edges: None,
                control_causal_edges: None,
                wait_structural_complete: None,
                waits: None,
                query_reads: None,
                cache_call_provenance: None,
                dispatch_result_provenance: None,
                task_bindings: None,
                resource: None,
                resource_attachments: None,
                observed_cpu: join.observed_cpu,
                query_cpu_ownership: None,
                observed_task_dag: None,
                execution_flow_dag: None,
                latch_readiness_dag: None,
                clock_alignment: None,
                issues: join.issues,
                opaque_boundaries: join.opaque_boundaries,
                cpu_work_ns: join.cpu_work_ns,
                execution_span_ns: join.execution_span_ns,
            })
        }
        "rustc-frontend-allowance" | "rustc-frontend-admission-attachment" => {
            assess_resource_document(bytes, &document.producer)
        }
        _ => bail!("unsupported execution graph producer"),
    }
}

/// Classification is not authority. Every matching document is reparsed from
/// its original bytes by the strict resource parser, including duplicate keys.
pub(crate) fn resource_producer(bytes: &[u8]) -> Result<Option<String>> {
    let header: serde_json::Value = serde_json::from_slice(bytes)?;
    Ok(header["producer"]
        .as_str()
        .filter(|producer| {
            matches!(
                *producer,
                "rustc-frontend-allowance" | "rustc-frontend-admission-attachment"
            )
        })
        .map(str::to_owned))
}

/// A malformed optional resource document must not erase independent query or
/// pool assessments. Its strict-parser failure remains visible on its own row.
fn assess_resource_document(bytes: &[u8], producer: &str) -> Result<Assessment> {
    let header: serde_json::Value = serde_json::from_slice(bytes)?;
    let parsed = if producer == "rustc-frontend-allowance" {
        crate::execution_graph_resource::assess_bytes(bytes)
            .and_then(|assessment| serde_json::to_value(assessment).map_err(Into::into))
    } else {
        crate::execution_graph_resource_attachment::assess_bytes(bytes)
            .and_then(|assessment| serde_json::to_value(assessment).map_err(Into::into))
    };
    let (resource, issues, structural_complete) = match parsed {
        Ok(resource) => {
            let valid = if producer == "rustc-frontend-allowance" {
                resource["prefix_protocol_valid"] == true
                    && resource["prefix_records_complete"] == true
            } else {
                resource["valid"] == true
            };
            let issues = resource["issues"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|issue| issue.as_str().map(str::to_owned))
                .collect();
            (resource, issues, valid)
        }
        Err(error) => {
            let message = format!("invalid resource authority: {error:#}");
            (
                serde_json::json!({"valid":false,"error":message}),
                vec![message],
                false,
            )
        }
    };
    Ok(Assessment {
        schema: header["schema"]
            .as_u64()
            .and_then(|v| v.try_into().ok())
            .unwrap_or(0),
        producer: producer.to_owned(),
        scope: header["scope"]
            .as_str()
            .unwrap_or("unknown-resource-scope")
            .to_owned(),
        // This header value is a display label only. Reconciliation uses strict
        // original-byte parsing, never this potentially malformed identity.
        capture_id: header["capture_id"].as_str().unwrap_or("").to_owned(),
        structural_complete,
        full_pipeline_complete: false,
        nodes: 0,
        edges: 0,
        descriptive_parent_edges: None,
        control_causal_edges: None,
        wait_structural_complete: None,
        waits: None,
        query_reads: None,
        cache_call_provenance: None,
        dispatch_result_provenance: None,
        task_bindings: None,
        resource: Some(resource),
        resource_attachments: None,
        observed_cpu: None,
        query_cpu_ownership: None,
        observed_task_dag: None,
        execution_flow_dag: None,
        latch_readiness_dag: None,
        clock_alignment: None,
        issues,
        opaque_boundaries: vec!["bounded-resource-prefix-not-full-lifecycle".into()],
        cpu_work_ns: None,
        execution_span_ns: None,
    })
}

fn is_owner_document(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| {
            [
                ".manifest.json",
                ".declaration.json",
                ".execute-entered.json",
                ".execute-result.json",
            ]
            .iter()
            .any(|suffix| name.ends_with(suffix))
                || (name.starts_with("compiler-owner-") && name.contains(".event-"))
        })
}

#[cfg(test)]
mod compiler_entry_routing_tests {
    use super::*;

    #[test]
    fn independent_entry_requires_parent_reconciliation() {
        let directory = crate::paths::home()
            .unwrap()
            .join(format!("graph-compiler-entry-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("entry.json");
        std::fs::write(
            &file,
            serde_json::to_vec(&serde_json::json!({
                "schema":1,"producer":"rustc-query-admission-entry",
                "scope":"compiler-entry-to-query-capture-binding","capture_id":"entry",
                "run_id":"run","invocation_id":"launch","parent_invocation_id":"parent",
                "parent_task_id":"cargo-unit-0","expected_query_captures":1,
                "query_capture_ids":["query"],"query_receipt_schema":3,
                "full_pipeline_complete":false,"cpu_work_ns":null,"execution_span_ns":null
            }))
            .unwrap(),
        )
        .unwrap();
        let assessment = read_receipt(&file).unwrap();
        assert!(!assessment.structural_complete);
        assert!(!assessment.full_pipeline_complete);
        assert!(assessment.issues[0].contains("--compiler-manifest"));
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn compiler_journals_are_authority_documents_not_standalone_receipts() {
        assert!(is_owner_document(Path::new(
            "compiler-owner-root.event-1.json"
        )));
        assert!(is_owner_document(Path::new(
            "compiler-owner-root.manifest.json"
        )));
        assert!(!is_owner_document(Path::new("query.json")));
    }
}

fn validate_owner_journal(
    selected: &crate::execution_graph_manifest::Manifest,
    journal: &crate::execution_graph_manifest::Manifest,
    filename: &str,
) -> Result<&'static str> {
    use crate::execution_graph_manifest::{OwnerOutcome, TicketLifecycle};
    if !filename.ends_with(".manifest.json")
        && journal
            .owners
            .iter()
            .any(|owner| owner.outcome != OwnerOutcome::Pending)
    {
        bail!("nonfinal owner journal has a terminal outcome");
    }
    if selected.owners.len() != 1 || journal.owners.len() != 1 {
        bail!("native owner journal requires exactly one scoped owner");
    }
    let owner = &selected.owners[0];
    let observed = &journal.owners[0];
    if owner.tickets.len() != 1 || observed.tickets.len() != 1 {
        bail!("native owner journal requires exactly one required ticket");
    }
    let ticket = &owner.tickets[0];
    let event = &observed.tickets[0];
    if selected.schema != journal.schema
        || selected.profile != journal.profile
        || selected.run_id != journal.run_id
        || selected.authority_root != journal.authority_root
        || owner.invocation_id != observed.invocation_id
        || owner.parent_invocation_id != observed.parent_invocation_id
        || owner.parent_task_id != observed.parent_task_id
        || ticket.invocation_id != event.invocation_id
        || ticket.parent_task_id != event.parent_task_id
        || ticket.policy != event.policy
        || ticket.capture_id != event.capture_id
    {
        bail!("owner journal conflicts with immutable expected producer identities");
    }
    if observed.lost_events != 0 {
        bail!("owner journal records lost observations");
    }
    if filename.ends_with(".declaration.json") {
        if observed.sealed
            || observed.interrupted
            || event.execute_entered
            || event.lifecycle != TicketLifecycle::Declared
        {
            bail!("invalid initial owner declaration");
        }
        Ok("declaration")
    } else if filename.ends_with(".execute-entered.json") {
        if observed.sealed
            || observed.interrupted
            || !event.execute_entered
            || event.lifecycle != TicketLifecycle::ExecuteEntered
            || !ticket.execute_entered
        {
            bail!("owner execute-enter journal conflicts with final lifecycle");
        }
        Ok("execute-entered")
    } else if filename.ends_with(".execute-result.json") {
        if observed.sealed
            || observed.interrupted
            || !event.execute_entered
            || !ticket.execute_entered
            || event.lifecycle != ticket.lifecycle
        {
            bail!("owner execute-result journal conflicts with final lifecycle");
        }
        Ok("execute-result")
    } else {
        if serde_json::to_value(selected)? != serde_json::to_value(journal)? {
            bail!("conflicting final or interrupted owner manifest");
        }
        Ok("manifest")
    }
}

pub(crate) fn compiler_scheduler_summary(
    directory: &Path,
    parent_id: &str,
    scheduler_id: &str,
) -> Result<(crate::execution_graph_compiler::SchedulerSummary, u64)> {
    use crate::execution_graph_manifest as protocol;
    for id in [parent_id, scheduler_id] {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            bail!("invalid parent capture identity");
        }
    }
    let mut used = 0u64;
    let mut load = |path: &Path, limit: u64| -> Result<Vec<u8>> {
        let bytes = read_bytes(path, limit)?;
        used = used.saturating_add(bytes.len() as u64);
        if used > MAX_BYTES {
            bail!("parent reconciliation exceeds total byte budget");
        }
        Ok(bytes)
    };
    let manifest = protocol::parse_manifest(&load(
        &directory.join(format!("owner-{parent_id}.manifest.json")),
        protocol::MAX_BYTES as u64,
    )?)?;
    if manifest.authority_root != parent_id
        || manifest.owners.len() != 1
        || manifest.owners[0].tickets.len() != 1
    {
        bail!("compiler registry has a mismatched scheduler authority");
    }
    let mut suffixes = vec!["declaration"];
    if manifest.owners[0].tickets[0].execute_entered {
        suffixes.extend(["execute-entered", "execute-result"]);
    }
    for suffix in suffixes {
        let name = format!("owner-{parent_id}.{suffix}.json");
        let journal =
            protocol::parse_manifest(&load(&directory.join(&name), protocol::MAX_BYTES as u64)?)?;
        validate_owner_journal(&manifest, &journal, &name)?;
    }
    let envelope = protocol::parse_envelope(&load(
        &directory.join(format!("{scheduler_id}.envelope.json")),
        protocol::MAX_BYTES as u64,
    )?)?;
    let receipt: Receipt = serde_json::from_slice(&load(
        &directory.join(format!("{scheduler_id}.json")),
        MAX_BYTES,
    )?)?;
    if receipt.capture_id != scheduler_id {
        bail!("scheduler filename and capture identity disagree");
    }
    let unit_ids = receipt.nodes.iter().map(|node| node.id).collect();
    let raw = assess(receipt)?;
    let summary = protocol::ReceiptSummary {
        schema: raw.schema,
        capture_id: raw.capture_id,
        structural_complete: raw.structural_complete,
        full_pipeline_complete: raw.full_pipeline_complete,
    };
    let coverage = protocol::reconcile(&manifest, &[envelope], &[summary])?;
    Ok((
        crate::execution_graph_compiler::SchedulerSummary {
            run_id: manifest.run_id,
            authority_root: manifest.authority_root,
            capture_id: scheduler_id.to_owned(),
            unit_ids,
            structural_complete: coverage.scheduler_entry_complete,
        },
        used,
    ))
}

fn command_manifest(path: &Path, manifest_path: &Path, json: bool) -> Result<()> {
    use crate::execution_graph_manifest::{
        self as protocol, ReceiptEnvelope, ReceiptSummary, reconcile,
    };
    let manifest =
        protocol::parse_manifest(&read_bytes(manifest_path, protocol::MAX_BYTES as u64)?)?;
    if !std::fs::symlink_metadata(path)?.is_dir() {
        bail!("manifest reconciliation requires a non-symlink capture directory");
    }
    let mut total_bytes = std::fs::symlink_metadata(manifest_path)?.len();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(path)? {
        files.push(entry?.path());
        if files.len() > MAX_FILES {
            bail!("capture directory exceeds reconciliation file budget");
        }
    }
    files.sort();
    let mut envelopes: Vec<ReceiptEnvelope> = Vec::new();
    let mut summaries = Vec::new();
    let mut journal_kinds = BTreeSet::new();
    let mut excluded_roots = BTreeSet::new();
    let mut receipt_files = Vec::new();
    let mut documents = Vec::new();
    for file in files {
        if !file.extension().is_some_and(|s| s == "json") {
            continue;
        }
        total_bytes = total_bytes.saturating_add(std::fs::symlink_metadata(&file)?.len());
        if total_bytes > MAX_BYTES {
            bail!("manifest reconciliation exceeds total byte budget");
        }
        documents.push(read_bytes(&file, MAX_BYTES)?);
        receipt_files.push(file);
    }
    let pools = collect_pool_inventories(&documents)?;
    for (file, bytes) in receipt_files.iter().zip(&documents) {
        if file
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("compiler-owner-"))
        {
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let authority = if name.contains(".event-") {
                crate::execution_graph_compiler::parse_journal(bytes)?.authority_root
            } else {
                crate::execution_graph_compiler::parse_manifest(bytes)?.authority_root
            };
            excluded_roots.insert(authority);
            continue;
        }
        if is_owner_document(file) {
            let journal = protocol::parse_manifest(bytes)?;
            if journal.authority_root != manifest.authority_root {
                excluded_roots.insert(journal.authority_root);
            } else {
                let filename = file
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();
                let kind = validate_owner_journal(&manifest, &journal, filename)?;
                if !journal_kinds.insert(kind) {
                    bail!("duplicate selected-owner journal observation");
                }
            }
            continue;
        }
        if file
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.ends_with(".envelope.json"))
        {
            envelopes.push(protocol::parse_envelope(bytes)?);
        } else {
            let receipt = assess_receipt_bytes(bytes, &pools)?;
            summaries.push(ReceiptSummary {
                schema: receipt.schema,
                capture_id: receipt.capture_id,
                structural_complete: receipt.structural_complete,
                full_pipeline_complete: receipt.full_pipeline_complete,
            });
        }
    }
    if manifest.owners.len() != 1 || manifest.owners[0].tickets.len() != 1 {
        bail!("CLI reconciliation currently supports one native BuildRunner authority");
    }
    if !journal_kinds.contains("declaration") {
        bail!("missing independent initial owner declaration");
    }
    if manifest.owners[0].tickets[0].execute_entered
        && (!journal_kinds.contains("execute-entered") || !journal_kinds.contains("execute-result"))
    {
        bail!("missing independent owner execution observations");
    }
    let mut assessment = reconcile(&manifest, &envelopes, &summaries)?;
    assessment.excluded_authority_roots = excluded_roots.into_iter().collect();
    if !json {
        println!("Native owner expected-producer assessment, not full-pipeline coverage:");
    }
    println!("{}", serde_json::to_string_pretty(&assessment)?);
    Ok(())
}

#[allow(dead_code)] // Strict compatibility entry point.
pub fn command(path: &Path, json: bool, manifest: Option<&Path>) -> Result<()> {
    command_with_policy(path, json, manifest, false)
}

pub(crate) fn command_with_policy(
    path: &Path,
    json: bool,
    manifest: Option<&Path>,
    large_capture: bool,
) -> Result<()> {
    command_with_analysis(path, json, manifest, large_capture, false)
}

pub(crate) fn command_with_analysis(
    path: &Path,
    json: bool,
    manifest: Option<&Path>,
    large_capture: bool,
    cache_call_provenance: bool,
) -> Result<()> {
    command_with_options(
        path,
        json,
        manifest,
        large_capture,
        cache_call_provenance,
        false,
    )
}

pub(crate) fn command_with_options(
    path: &Path,
    json: bool,
    manifest: Option<&Path>,
    large_capture: bool,
    cache_call_provenance: bool,
    dispatch_result_provenance: bool,
) -> Result<()> {
    command_with_query_cpu_ownership(
        path,
        json,
        manifest,
        large_capture,
        cache_call_provenance,
        dispatch_result_provenance,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn command_with_query_cpu_ownership(
    path: &Path,
    json: bool,
    manifest: Option<&Path>,
    large_capture: bool,
    cache_call_provenance: bool,
    dispatch_result_provenance: bool,
    query_cpu_ownership: bool,
) -> Result<()> {
    command_with_observed_task_dag(
        path,
        json,
        manifest,
        large_capture,
        cache_call_provenance,
        dispatch_result_provenance,
        query_cpu_ownership,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn command_with_observed_task_dag(
    path: &Path,
    json: bool,
    manifest: Option<&Path>,
    large_capture: bool,
    cache_call_provenance: bool,
    dispatch_result_provenance: bool,
    query_cpu_ownership: bool,
    observed_task_dag: bool,
) -> Result<()> {
    command_with_execution_flow_dag(
        path,
        json,
        manifest,
        large_capture,
        cache_call_provenance,
        dispatch_result_provenance,
        query_cpu_ownership,
        observed_task_dag,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn command_with_execution_flow_dag(
    path: &Path,
    json: bool,
    manifest: Option<&Path>,
    large_capture: bool,
    cache_call_provenance: bool,
    dispatch_result_provenance: bool,
    query_cpu_ownership: bool,
    observed_task_dag: bool,
    execution_flow_dag: bool,
) -> Result<()> {
    command_with_latch_readiness_dag(
        path,
        json,
        manifest,
        large_capture,
        cache_call_provenance,
        dispatch_result_provenance,
        query_cpu_ownership,
        observed_task_dag,
        execution_flow_dag,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn command_with_latch_readiness_dag(
    path: &Path,
    json: bool,
    manifest: Option<&Path>,
    large_capture: bool,
    cache_call_provenance: bool,
    dispatch_result_provenance: bool,
    query_cpu_ownership: bool,
    observed_task_dag: bool,
    execution_flow_dag: bool,
    latch_readiness_dag: bool,
) -> Result<()> {
    if latch_readiness_dag && manifest.is_some() {
        bail!("--latch-readiness-dag is not supported with --manifest");
    }
    if execution_flow_dag && manifest.is_some() {
        bail!("--execution-flow-dag is not supported with --manifest");
    }
    if observed_task_dag && manifest.is_some() {
        bail!("--observed-task-dag is not supported with --manifest");
    }
    if query_cpu_ownership && manifest.is_some() {
        bail!("--query-cpu-ownership is not supported with --manifest");
    }
    if dispatch_result_provenance && manifest.is_some() {
        bail!("--dispatch-result-provenance is not supported with --manifest");
    }
    if cache_call_provenance && manifest.is_some() {
        bail!("--cache-call-provenance is not supported with --manifest");
    }
    if large_capture && manifest.is_some() {
        bail!("--large-capture is not supported with --manifest");
    }
    if let Some(manifest) = manifest {
        return command_manifest(path, manifest, json);
    }
    let mut policy = if large_capture {
        crate::execution_graph_query::CapturePolicy::large()
    } else {
        crate::execution_graph_query::CapturePolicy::strict()
    };
    policy.query_cpu_ownership = query_cpu_ownership;
    policy.observed_task_dag = observed_task_dag;
    policy.execution_flow_dag = execution_flow_dag;
    policy.latch_readiness_dag = latch_readiness_dag;
    let assessments = collect_path_with_options(
        path,
        policy,
        cache_call_provenance,
        dispatch_result_provenance,
    )?;
    render_assessments(assessments, json)
}

#[cfg(test)]
fn collect_path(path: &Path) -> Result<Vec<Assessment>> {
    collect_path_with_policy(path, crate::execution_graph_query::CapturePolicy::strict())
}

#[cfg(test)]
fn collect_path_with_policy(
    path: &Path,
    policy: crate::execution_graph_query::CapturePolicy,
) -> Result<Vec<Assessment>> {
    collect_path_with_analysis(path, policy, false)
}

#[cfg(test)]
fn collect_path_with_analysis(
    path: &Path,
    policy: crate::execution_graph_query::CapturePolicy,
    cache_call_provenance: bool,
) -> Result<Vec<Assessment>> {
    collect_path_with_options(path, policy, cache_call_provenance, false)
}

fn collect_path_with_options(
    path: &Path,
    policy: crate::execution_graph_query::CapturePolicy,
    cache_call_provenance: bool,
    dispatch_result_provenance: bool,
) -> Result<Vec<Assessment>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("capture path must not be a symlink");
    }
    let mut files = if metadata.is_dir() {
        let mut files = Vec::new();
        let mut entries = 0usize;
        for entry in std::fs::read_dir(path)? {
            entries += 1;
            if entries > MAX_FILES {
                bail!("capture directory exceeds file budget");
            }
            let path = entry?.path();
            if path.extension().is_some_and(|s| s == "json") {
                files.push(path);
                if files.len() > MAX_FILES {
                    bail!("capture directory exceeds receipt budget");
                }
            }
        }
        files
    } else {
        vec![path.to_owned()]
    };
    files.sort();
    if files.is_empty() {
        bail!("no receipts: capture completeness is unknown, not complete");
    }
    let mut identities = BTreeMap::new();
    let mut assessments: Vec<Assessment> = Vec::new();
    let mut total_bytes = 0u64;
    let mut documents = Vec::new();
    for file in files {
        let bytes = read_bytes(&file, policy.max_bytes as u64)?;
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > policy.max_bytes as u64 {
            bail!("capture directory exceeds total receipt byte budget");
        }
        let resource = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|header| {
                matches!(
                    header["producer"].as_str(),
                    Some("rustc-frontend-allowance" | "rustc-frontend-admission-attachment")
                )
            });
        if !resource
            && (is_owner_document(&file)
                || file
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.ends_with(".envelope.json")))
        {
            continue;
        }
        documents.push(bytes);
    }
    if documents.is_empty() {
        bail!("no receipts: capture completeness is unknown, not complete");
    }
    // Collect exact authorities from original bytes before flattening any row.
    let resource_attachments =
        crate::execution_graph_resource_attachment::collect_documents_with_scan_budget(
            &documents,
            None,
            policy.max_bytes,
        )?;
    let pools = collect_pool_inventories_with_options(
        &documents,
        policy.query_cpu_ownership,
        policy.execution_flow_dag,
        policy.latch_readiness_dag,
    )?;
    let mut query_authorities = BTreeMap::new();
    let mut wait_authorities = BTreeMap::new();
    if policy.query_cpu_ownership || policy.execution_flow_dag || policy.latch_readiness_dag {
        let mut counts = BTreeMap::<String, usize>::new();
        for bytes in &documents {
            let header: serde_json::Value = serde_json::from_slice(bytes)?;
            if let Some(id) = header["capture_id"].as_str() {
                *counts.entry(id.to_owned()).or_default() += 1;
            }
            if header["producer"] == "rustc-query-admission" {
                let query = collect_query_capture_with_options(
                    bytes,
                    &pools,
                    policy,
                    cache_call_provenance,
                    dispatch_result_provenance,
                )?;
                let aligned = query
                    .assessment
                    .clock_alignment
                    .as_ref()
                    .is_some_and(|c| c.aligned);
                if policy.latch_readiness_dag
                    && let Some(waits) = &query.wait_inventory
                {
                    let authority = waits.original_authority();
                    wait_authorities.insert(authority.capture_id.clone(), authority);
                }
                if let Some(inventory) = query.execution_inventory {
                    query_authorities.insert(inventory.capture_id.clone(), (inventory, aligned));
                }
            }
        }
        query_authorities.retain(|id, _| counts.get(id) == Some(&1));
        wait_authorities.retain(|id, _| counts.get(id) == Some(&1));
    }
    for bytes in &documents {
        let mut assessment = assess_receipt_bytes_with_options(
            bytes,
            &pools,
            policy,
            cache_call_provenance,
            dispatch_result_provenance,
        )?;
        if let Some(&previous) = identities.get(&assessment.capture_id) {
            let previous: &mut Assessment = &mut assessments[previous];
            poison_duplicate_capture(previous);
            poison_duplicate_capture(&mut assessment);
        } else {
            identities.insert(assessment.capture_id.clone(), assessments.len());
        }
        assessments.push(assessment);
    }
    if policy.query_cpu_ownership || policy.execution_flow_dag || policy.latch_readiness_dag {
        let mut reports = BTreeMap::new();
        for assessment in &mut assessments {
            if assessment.producer != "rustc-thread-pool-join"
                || !matches!(assessment.schema, 4..=6)
            {
                continue;
            }
            if let Some(cpu) = &mut assessment.observed_cpu {
                let query_id = cpu.clock_bridge.query_capture_id.clone();
                let authority = query_authorities
                    .get(&query_id)
                    .filter(|_| pools.contains_key(&assessment.capture_id));
                let report = cpu.reconcile_query_ownership(
                    authority.map(|(q, _)| q),
                    authority.is_some_and(|(_, aligned)| *aligned),
                )?;
                if query_authorities.get(&query_id).is_some_and(|(q, _)| {
                    q.clock
                        .as_ref()
                        .is_some_and(|clock| clock.pool_capture_id == assessment.capture_id)
                }) {
                    reports.insert(query_id, report);
                }
            }
        }
        for assessment in &mut assessments {
            if policy.query_cpu_ownership && assessment.producer == "rustc-query-admission" {
                assessment.query_cpu_ownership =
                    Some(reports.remove(&assessment.capture_id).unwrap_or_else(|| {
                        crate::execution_graph_timing::QueryCpuOwnership::unavailable()
                    }));
            }
        }
    }
    if policy.observed_task_dag {
        let mut counts = BTreeMap::<String, usize>::new();
        let mut task_pools = BTreeMap::new();
        for bytes in &documents {
            let header: serde_json::Value = serde_json::from_slice(bytes)?;
            if let Some(id) = header["capture_id"].as_str() {
                *counts.entry(id.to_owned()).or_default() += 1;
            }
            if header["producer"] == "rustc-thread-pool-join" {
                let inventory =
                    crate::execution_graph_continuation::collect_task_inventory_with_options(
                        bytes,
                        policy.query_cpu_ownership,
                        policy.execution_flow_dag,
                        policy.latch_readiness_dag,
                    )?;
                task_pools.insert(inventory.capture_id.clone(), inventory);
            }
        }
        task_pools.retain(|id, _| counts.get(id) == Some(&1));
        for assessment in &mut assessments {
            if assessment.producer == "rustc-thread-pool-join" {
                assessment.observed_task_dag = Some(crate::execution_graph_task_dag::assess(
                    task_pools.get(&assessment.capture_id),
                    assessment.observed_cpu.as_ref(),
                )?);
            }
        }
    }
    if policy.execution_flow_dag || policy.latch_readiness_dag {
        let mut counts = BTreeMap::<String, usize>::new();
        for bytes in &documents {
            let header: serde_json::Value = serde_json::from_slice(bytes)?;
            if let Some(id) = header["capture_id"].as_str() {
                *counts.entry(id.to_owned()).or_default() += 1;
            }
        }
        for (bytes, assessment) in documents.iter().zip(&mut assessments) {
            if assessment.producer != "rustc-thread-pool-join"
                || !matches!(assessment.schema, 5 | 6)
            {
                continue;
            }
            // Each original pool is independently parsed and locally validated,
            // even when its identity will subsequently be made unavailable.
            let pool = crate::execution_graph_continuation::collect_task_inventory_with_options(
                bytes,
                policy.query_cpu_ownership,
                policy.execution_flow_dag,
                policy.latch_readiness_dag,
            )?;
            let flow =
                crate::execution_graph_continuation::execution_flow_from_validated_bytes(bytes)?
                    .expect("mandatory schema5/6 flow");
            let timing = assessment
                .observed_cpu
                .as_ref()
                .expect("mandatory schema5 timing2");
            let query_id = pool
                .query_capture_id
                .as_deref()
                .expect("schema5 query identity");
            let authority = query_authorities
                .get(query_id)
                .filter(|_| pools.contains_key(&pool.capture_id));
            let context = crate::execution_graph_flow::FlowContext {
                pool_capture_id: &pool.capture_id,
                query_capture_id: query_id,
                observer: pool.observer,
                pool_sealed: pool.sealed,
                structural_complete: pool.structural_complete,
                timing,
                query_inventory: authority.map(|(q, _)| q),
                clock_aligned: authority.is_some_and(|(_, aligned)| *aligned)
                    && timing.query_clock_aligned,
                task_high_water: pool.task_high_water,
                task_intervals: &pool.task_intervals,
                task_edges: &pool.task_edges,
                task_outcomes: &pool.task_outcomes,
            };
            let duplicate = counts.get(&pool.capture_id) != Some(&1)
                || counts.get(query_id).is_some_and(|n| *n > 1);
            match flow {
                crate::execution_graph_continuation::FlowEnvelope::V1(flow) => {
                    let report = crate::execution_graph_flow::assess(flow, context)?;
                    assessment.execution_flow_dag = Some(if duplicate {
                        crate::execution_graph_flow::FlowAssessment::unavailable(
                            &pool.capture_id,
                            pool.observer,
                            "duplicate or cross-producer capture identity makes execution flow authority unavailable",
                        )
                    } else {
                        report
                    });
                }
                crate::execution_graph_continuation::FlowEnvelope::V2(flow) => {
                    let report = crate::execution_graph_readiness::assess(
                        flow,
                        crate::execution_graph_readiness::ReadinessContext {
                            flow: context,
                            waits: authority.and_then(|_| wait_authorities.get(query_id)),
                        },
                    )?;
                    assessment.latch_readiness_dag = Some(if duplicate {
                        crate::execution_graph_readiness::ReadinessAssessment::unavailable(
                            &pool.capture_id,
                            pool.observer,
                            "duplicate or cross-producer capture identity makes readiness authority unavailable",
                        )
                    } else {
                        report
                    });
                }
            }
        }
    }
    if let Some(first) = assessments.first_mut() {
        first.resource_attachments = Some(resource_attachments);
    }
    Ok(assessments)
}

fn poison_duplicate_capture(assessment: &mut Assessment) {
    assessment.structural_complete = false;
    assessment.issues.push("duplicate capture identity".into());
    if let Some(calls) = &mut assessment.cache_call_provenance {
        calls.suppress_incomplete_admission_authority();
    }
    if let Some(dispatch) = &mut assessment.dispatch_result_provenance {
        dispatch.suppress_incomplete_admission_authority();
    }
    if let Some(resource) = &mut assessment.resource {
        resource["capture_authority_valid"] = serde_json::json!(false);
        if assessment.producer == "rustc-frontend-allowance" {
            resource["prefix_records_complete"] = serde_json::json!(false);
            resource["local_resource_edges"] = serde_json::json!([]);
        } else {
            resource["valid"] = serde_json::json!(false);
        }
    }
}

fn render_assessments(assessments: Vec<Assessment>, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&assessments)?);
    } else {
        for a in assessments {
            println!(
                "{}: {} nodes, {} edges, scoped structure {}, full pipeline INCOMPLETE",
                a.scope,
                a.nodes,
                a.edges,
                if a.structural_complete {
                    "validated"
                } else {
                    "INCOMPLETE"
                }
            );
            if let Some(parents) = a.descriptive_parent_edges {
                println!("  admission parents: {parents} descriptive links, not semantic reads");
                match a.wait_structural_complete {
                    Some(complete) => println!(
                        "  waits: {} observed, {} notification -> resume control-causal links, inventory {}",
                        a.waits.unwrap_or(0),
                        a.control_causal_edges.unwrap_or(0),
                        if complete { "validated" } else { "INCOMPLETE" }
                    ),
                    None => println!("  waits: unknown (v1 has no independent wait inventory)"),
                }
            }
            if let Some(reads) = &a.query_reads {
                println!(
                    "  direct reads: {} tracked, {} resolved job links, records {}, semantic coverage INCOMPLETE",
                    reads.tracked_reads,
                    reads.resolved_job_edges,
                    if reads.records_complete {
                        "validated"
                    } else {
                        "INCOMPLETE"
                    }
                );
            }
            if let Some(calls) = &a.cache_call_provenance {
                println!(
                    "  cache-call provenance: {} resolved of {} requests, {} unresolved (availability only, not an execution DAG)",
                    calls.resolved_requests, calls.observed_requests, calls.unresolved_requests
                );
            }
            if let Some(dispatch) = &a.dispatch_result_provenance {
                println!(
                    "  dispatch-result provenance: {} resolved of {} requests, {} unresolved (availability only, not an execution DAG)",
                    dispatch.resolved_requests,
                    dispatch.observed_requests,
                    dispatch.unresolved_requests
                );
            }
            if let Some(ownership) = &a.query_cpu_ownership {
                println!(
                    "  query execution CPU (observed exclusive segments only): {}",
                    serde_json::to_string(ownership)?
                );
            }
            if let Some(bindings) = &a.task_bindings {
                // Query point bindings are not task DAG edges.
                println!(
                    "  task point mapping: {} bound, {} unbound, graph binding {} (not query slices)",
                    bindings.bound_points,
                    bindings.unbound_points,
                    if bindings.graph_binding_complete {
                        "validated"
                    } else {
                        "INCOMPLETE"
                    }
                );
                for issue in &bindings.issues {
                    println!("  task binding: {issue}");
                }
            }
            if let Some(dag) = &a.observed_task_dag {
                println!(
                    "  observed fork/join task DAG: {} tasks, {} native edges, known attached CPU {:?} ns, excluded task-null CPU {:?} ns (not full work/span or parallelism)",
                    dag.nodes.len(),
                    dag.edges.len(),
                    dag.known_attached_cpu_ns,
                    dag.excluded_task_null_cpu_ns
                );
                for component in &dag.components {
                    println!(
                        "    root {}: known recorded CPU chain {:?} ns, exact retained chain {:?} ns, tasks {:?}",
                        component.root,
                        component.known_cpu_chain_ns,
                        component.longest_recorded_fork_join_chain_cpu_ns,
                        component.chain
                    );
                }
                for issue in &dag.issues {
                    println!("  task DAG: {issue}");
                }
            }
            if let Some(flow) = &a.execution_flow_dag {
                println!(
                    "  observed query/join execution flow (not full work/span or parallelism): {}",
                    serde_json::to_string(flow)?
                );
            }
            if let Some(readiness) = &a.latch_readiness_dag {
                println!(
                    "  observed query/join/latch readiness (unknown gaps, not full work/span or parallelism): {}",
                    serde_json::to_string(readiness)?
                );
            }
            if let Some(resource) = &a.resource {
                println!("  resource authority: {}", serde_json::to_string(resource)?);
            }
            if let Some(attachments) = &a.resource_attachments {
                println!(
                    "  resource attachments (directory-wide): {}",
                    serde_json::to_string(attachments)?
                );
            }
            for issue in a.issues {
                println!("  issue: {issue}");
            }
            for boundary in a.opaque_boundaries {
                println!("  opaque: {boundary}");
            }
        }
        println!("CPU work, execution span and thread ceiling: unknown");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cache_call_provenance_root_projection_is_opt_in_and_duplicate_safe() {
        use crate::execution_graph_query::{CapturePolicy, tests::cache_call_fixture};
        let directory = resource_test_directory("cache-call-provenance");
        let file = directory.join("a.json");
        let bytes = serde_json::to_vec(&cache_call_fixture()).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        let legacy = collect_path(&file).unwrap();
        let explicit_off =
            collect_path_with_analysis(&file, CapturePolicy::strict(), false).unwrap();
        assert_eq!(
            serde_json::to_vec(&legacy).unwrap(),
            serde_json::to_vec(&explicit_off).unwrap()
        );
        assert!(
            serde_json::to_value(&legacy).unwrap()[0]
                .get("cache_call_provenance")
                .is_none()
        );
        for policy in [CapturePolicy::strict(), CapturePolicy::large()] {
            let analyzed = collect_path_with_analysis(&file, policy, true).unwrap();
            let mut value = serde_json::to_value(&analyzed).unwrap();
            let calls = value[0]
                .as_object_mut()
                .unwrap()
                .remove("cache_call_provenance")
                .unwrap();
            assert_eq!(calls["resolved_requests"], 1);
            assert_eq!(calls["relations"][0]["result_id"], 1);
            assert_eq!(value, serde_json::to_value(&legacy).unwrap());
        }
        assert!(
            collect_path_with_analysis(
                &file,
                CapturePolicy {
                    max_bytes: bytes.len() - 1,
                    max_points: 7,
                    query_cpu_ownership: false,
                    observed_task_dag: false,
                    execution_flow_dag: false,
                    latch_readiness_dag: false,
                },
                true
            )
            .is_err()
        );
        assert!(command_with_analysis(&directory, true, Some(&file), false, true).is_err());
        std::fs::write(directory.join("b.json"), &bytes).unwrap();
        let duplicates =
            collect_path_with_analysis(&directory, CapturePolicy::strict(), true).unwrap();
        assert_eq!(duplicates.len(), 2);
        for a in duplicates {
            assert!(!a.structural_complete);
            let calls = serde_json::to_value(a.cache_call_provenance.unwrap()).unwrap();
            assert_eq!(calls["resolved_requests"], 0);
            assert_eq!(calls["relations"], json!([]));
            assert_eq!(
                calls["unresolved_reasons"]["admission_authority_incomplete"],
                1
            );
        }
        assert!(
            collect_path_with_analysis(
                &directory,
                CapturePolicy {
                    max_bytes: bytes.len(),
                    max_points: 7,
                    query_cpu_ownership: false,
                    observed_task_dag: false,
                    execution_flow_dag: false,
                    latch_readiness_dag: false,
                },
                true
            )
            .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dispatch_result_provenance_path_is_explicit_and_duplicate_safe() {
        use crate::execution_graph_query::{CapturePolicy, tests::dispatch_fixture};
        let directory = resource_test_directory("dispatch-result-provenance");
        let file = directory.join("a.json");
        let bytes = serde_json::to_vec(&dispatch_fixture()).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        assert!(collect_path(&file).is_err());
        assert!(collect_path_with_analysis(&file, CapturePolicy::large(), true).is_err());
        assert!(collect_query(&bytes, &BTreeMap::new()).is_err());
        for policy in [CapturePolicy::strict(), CapturePolicy::large()] {
            for cache in [false, true] {
                let result = collect_path_with_options(&file, policy, cache, true).unwrap();
                let a = &result[0];
                assert_eq!(a.schema, 6);
                assert!(a.structural_complete);
                assert_eq!(a.cache_call_provenance.is_some(), cache);
                assert_eq!(
                    serde_json::to_value(a).unwrap()["dispatch_result_provenance"]["resolved_requests"],
                    1
                );
                assert!(!a.clock_alignment.as_ref().unwrap().aligned);
                assert_eq!(a.cpu_work_ns, None);
                assert_eq!(a.execution_span_ns, None);
            }
        }
        assert!(command_with_options(&file, true, Some(&file), false, false, true).is_err());
        assert!(
            collect_path_with_options(
                &file,
                CapturePolicy {
                    max_bytes: bytes.len() - 1,
                    max_points: 7,
                    query_cpu_ownership: false,
                    observed_task_dag: false,
                    execution_flow_dag: false,
                    latch_readiness_dag: false,
                },
                false,
                true
            )
            .is_err()
        );
        std::fs::write(directory.join("b.json"), &bytes).unwrap();
        let duplicates =
            collect_path_with_options(&directory, CapturePolicy::strict(), true, true).unwrap();
        assert_eq!(duplicates.len(), 2);
        for a in duplicates {
            assert!(!a.structural_complete);
            let dispatch = serde_json::to_value(a.dispatch_result_provenance.unwrap()).unwrap();
            assert_eq!(dispatch["resolved_requests"], 0);
            assert_eq!(dispatch["relations"], json!([]));
            assert_eq!(
                dispatch["unresolved_reasons"]["admission_authority_incomplete"],
                1
            );
        }
        assert!(
            collect_path_with_options(
                &directory,
                CapturePolicy {
                    max_bytes: bytes.len(),
                    max_points: 7,
                    query_cpu_ownership: false,
                    observed_task_dag: false,
                    execution_flow_dag: false,
                    latch_readiness_dag: false,
                },
                false,
                true
            )
            .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dispatch_result_provenance_graph_does_not_promote_resource_authority() {
        use crate::execution_graph_query::{CapturePolicy, tests::dispatch_fixture};
        let mut documents = crate::execution_graph_resource_attachment::tests::documents();
        let baseline = documents
            .iter()
            .map(|v| serde_json::to_vec(v).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            crate::execution_graph_resource_attachment::collect_documents(&baseline, None)
                .unwrap()
                .state,
            "validated"
        );
        let query_id = documents[0]["capture_id"].clone();
        documents[0] = dispatch_fixture();
        documents[0]["capture_id"] = query_id;
        let directory = resource_test_directory("dispatch-resource-nonpromotion");
        for (i, document) in documents.iter().enumerate() {
            write_json(&directory, &format!("{i}.json"), document);
        }
        let assessments =
            collect_path_with_options(&directory, CapturePolicy::strict(), true, true).unwrap();
        let query = assessments.iter().find(|a| a.schema == 6).unwrap();
        assert!(query.structural_complete);
        assert!(query.task_bindings.as_ref().unwrap().graph_binding_valid);
        assert_eq!(
            serde_json::to_value(query).unwrap()["dispatch_result_provenance"]["resolved_requests"],
            1
        );
        let resources = assessments[0].resource_attachments.as_ref().unwrap();
        assert_ne!(resources.state, "validated");
        assert_eq!(resources.attachments.len(), 1);
        assert!(resources.attachments.iter().all(|a| {
            !a.context_links_valid
                && !a.context_links_complete
                && a.cross_wait_edges.is_empty()
                && a.issues
                    .iter()
                    .any(|s| s == "missing reciprocal query4/pool2 authority")
        }));
        assert!(!resources.full_pipeline_complete && !resources.resource_full_complete);
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn resource_test_directory(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::paths::home().unwrap());
        let directory = root.join(format!(
            "graph-resource-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn write_json(directory: &Path, name: &str, value: &serde_json::Value) {
        std::fs::write(directory.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn query_cpu_documents() -> (serde_json::Value, serde_json::Value) {
        let docs = crate::execution_graph_resource_attachment::tests::documents();
        let mut query = docs[0].clone();
        let mut pool = docs[1].clone();
        let mut timing = crate::execution_graph_timing::tests::fixture();
        timing["schema"] = json!(2);
        timing["clock_bridge"]["pool_capture_id"] = pool["capture_id"].clone();
        for row in timing["segments"].as_array_mut().unwrap() {
            row["query_context"] = serde_json::Value::Null;
            row["query_job"] = serde_json::Value::Null;
            row["query_name"] = serde_json::Value::Null;
        }
        timing["segments"][0]["owner_kind"] = json!("query_execution");
        timing["segments"][0]["query_context"] = json!(1);
        timing["segments"][0]["query_job"] = json!(1);
        timing["segments"][0]["query_name"] = json!("type_of");
        pool["schema"] = json!(4);
        pool["timing"] = timing;
        query["schema"] = json!(5);
        query["execution_clock"] = pool["timing"]["clock_bridge"].clone();
        query["events"][3]["at_ns"] = json!(30);
        (query, pool)
    }

    fn query_cpu_policy() -> crate::execution_graph_query::CapturePolicy {
        crate::execution_graph_query::CapturePolicy {
            query_cpu_ownership: true,
            ..crate::execution_graph_query::CapturePolicy::strict()
        }
    }

    fn task_dag_values(directory: &Path, query_cpu: bool) -> Result<serde_json::Value> {
        let mut policy = crate::execution_graph_query::CapturePolicy::strict();
        policy.observed_task_dag = true;
        policy.query_cpu_ownership = query_cpu;
        Ok(serde_json::to_value(collect_path_with_options(
            directory, policy, false, false,
        )?)?)
    }

    fn flow_documents() -> (serde_json::Value, serde_json::Value) {
        let (query, mut pool) = query_cpu_documents();
        pool["schema"] = json!(5);
        pool["execution_flow"] = crate::execution_graph_flow::tests::fixture();
        (query, pool)
    }
    fn flow_values(directory: &Path) -> Result<serde_json::Value> {
        let policy = crate::execution_graph_query::CapturePolicy {
            execution_flow_dag: true,
            ..crate::execution_graph_query::CapturePolicy::strict()
        };
        Ok(serde_json::to_value(collect_path_with_options(
            directory, policy, false, false,
        )?)?)
    }
    fn readiness_documents() -> (serde_json::Value, serde_json::Value) {
        let (mut query, mut pool) = flow_documents();
        remove_original_fixture_waits(&mut query);
        pool["schema"] = json!(6);
        let flow = &mut pool["execution_flow"];
        flow["schema"] = json!(2);
        flow["scope"] = json!("observed-query-join-and-latch-control-flow");
        for prefix in ["readiness", "wait_attempt"] {
            flow[format!("{prefix}_allocator_baseline")] = json!(1);
            flow[format!("{prefix}_allocator_high_water")] = json!(1);
        }
        for field in [
            "expected_readiness",
            "lost_readiness",
            "active_readiness",
            "expected_wait_attempts",
            "lost_wait_attempts",
            "active_wait_attempts",
        ] {
            flow[field] = json!(0);
        }
        flow["readiness"] = json!([]);
        flow["wait_attempts"] = json!([]);
        (query, pool)
    }
    fn remove_original_fixture_waits(query: &mut serde_json::Value) {
        query["wait_allocator_high_water"] = json!(1);
        query["expected_waits"] = json!(0);
        query["wait_events"] = json!([]);
        let events = query["task_bindings"]["events"].as_array_mut().unwrap();
        events.retain(|event| {
            !matches!(
                event["source"]["kind"].as_str(),
                Some("wait_registered" | "wait_resumed")
            )
        });
        let retained = events.len();
        query["task_bindings"]["expected_points"] = json!(retained);
    }
    fn readiness_values(directory: &Path) -> Result<serde_json::Value> {
        let policy = crate::execution_graph_query::CapturePolicy {
            latch_readiness_dag: true,
            ..crate::execution_graph_query::CapturePolicy::strict()
        };
        Ok(serde_json::to_value(collect_path_with_options(
            directory, policy, false, false,
        )?)?)
    }
    #[test]
    fn latch_readiness_is_independent_explicit_and_preserves_old_modes() {
        let directory = resource_test_directory("readiness-policy");
        let (query, pool) = readiness_documents();
        write_json(&directory, "query.json", &query);
        write_json(&directory, "pool.json", &pool);
        for mut policy in [
            crate::execution_graph_query::CapturePolicy::strict(),
            query_cpu_policy(),
        ] {
            assert!(collect_path_with_options(&directory, policy, false, false).is_err());
            policy.execution_flow_dag = true;
            policy.observed_task_dag = true;
            assert!(collect_path_with_options(&directory, policy, false, false).is_err());
        }
        let values = readiness_values(&directory).unwrap();
        let p = pool_value(&values);
        assert_eq!(
            p["latch_readiness_dag"]["scope"],
            "observed-query-join-and-latch-control-flow"
        );
        assert_eq!(p["latch_readiness_dag"]["records_complete"], true);
        assert_eq!(p["latch_readiness_dag"]["matched_readiness_transfers"], 0);
        for value in values.as_array().unwrap() {
            for field in [
                "execution_flow_dag",
                "query_cpu_ownership",
                "cache_call_provenance",
                "dispatch_result_provenance",
            ] {
                assert!(value.get(field).is_none(), "{field}");
            }
            assert_eq!(value["full_pipeline_complete"], false);
            assert!(value["cpu_work_ns"].is_null() && value["execution_span_ns"].is_null());
        }
        let mut root6 = crate::execution_graph_query::tests::dispatch_fixture();
        root6["capture_id"] = query["capture_id"].clone();
        root6["execution_clock"] = query["execution_clock"].clone();
        root6["events"][3]["at_ns"] = json!(30);
        remove_original_fixture_waits(&mut root6);
        write_json(&directory, "query.json", &root6);
        let root6_values = readiness_values(&directory).unwrap();
        assert_eq!(
            pool_value(&root6_values)["latch_readiness_dag"]["records_complete"],
            true
        );
        assert!(
            root6_values
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v.get("dispatch_result_provenance").is_none()
                    && v.get("query_cpu_ownership").is_none())
        );
        let policy = crate::execution_graph_query::CapturePolicy {
            latch_readiness_dag: true,
            execution_flow_dag: true,
            query_cpu_ownership: true,
            observed_task_dag: true,
            ..crate::execution_graph_query::CapturePolicy::strict()
        };
        let all = serde_json::to_value(
            collect_path_with_options(&directory, policy, true, true).unwrap(),
        )
        .unwrap();
        assert!(pool_value(&all).get("execution_flow_dag").is_none());
        assert_eq!(
            pool_value(&all)["latch_readiness_dag"]["records_complete"],
            true
        );
        assert_eq!(pool_value(&all)["observed_task_dag"]["attached_cpu_ns"], 6);
        let q = all
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["producer"] == "rustc-query-admission")
            .unwrap();
        assert_eq!(q["query_cpu_ownership"]["query_execution_cpu_ns"], 6);
        assert!(
            q.get("dispatch_result_provenance").is_some()
                && q.get("cache_call_provenance").is_some()
        );
        for (query, pool, cpu, flow) in [
            {
                let (q, mut p) = query_cpu_documents();
                p["schema"] = json!(3);
                p["timing"]["schema"] = json!(1);
                p["timing"]["segments"][0]["owner_kind"] = json!("task");
                for row in p["timing"]["segments"].as_array_mut().unwrap() {
                    for field in ["query_context", "query_job", "query_name"] {
                        row.as_object_mut().unwrap().remove(field);
                    }
                }
                (q, p, false, false)
            },
            {
                let (q, p) = query_cpu_documents();
                (q, p, true, false)
            },
            {
                let (q, p) = flow_documents();
                (q, p, false, true)
            },
        ] {
            write_json(&directory, "query.json", &query);
            write_json(&directory, "pool.json", &pool);
            assert_eq!(
                readiness_values(&directory).is_err(),
                pool["schema"].as_u64().unwrap() >= 4
            );
            let mut policy = crate::execution_graph_query::CapturePolicy::strict();
            policy.query_cpu_ownership = cpu;
            policy.execution_flow_dag = flow;
            let before = serde_json::to_value(
                collect_path_with_options(&directory, policy, false, false).unwrap(),
            )
            .unwrap();
            policy.latch_readiness_dag = true;
            let after = serde_json::to_value(
                collect_path_with_options(&directory, policy, false, false).unwrap(),
            )
            .unwrap();
            assert_eq!(before, after);
            assert!(pool_value(&after).get("latch_readiness_dag").is_none());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn latch_readiness_strict_original_envelope_and_global_identity_authority() {
        let directory = resource_test_directory("readiness-authorities");
        let (query, pool) = readiness_documents();
        write_json(&directory, "query.json", &query);
        for field in ["execution_flow", "timing"] {
            for null in [false, true] {
                let mut invalid = pool.clone();
                if null {
                    invalid[field] = serde_json::Value::Null;
                } else {
                    invalid.as_object_mut().unwrap().remove(field);
                }
                write_json(&directory, "pool.json", &invalid);
                assert!(readiness_values(&directory).is_err(), "{field}");
            }
        }
        for (pointer, value) in [
            ("/execution_flow/schema", json!(1)),
            (
                "/execution_flow/scope",
                json!("observed-query-and-join-control-flow"),
            ),
            ("/execution_flow/readiness_allocator_baseline", json!(0)),
            ("/execution_flow/wait_attempt_allocator_baseline", json!(0)),
            ("/execution_flow/expected_readiness", json!(1)),
            ("/execution_flow/scopes/0/query_job", json!(99)),
            ("/timing/clock_bridge/pool_epoch_in_query_ns", json!(1)),
        ] {
            let mut invalid = pool.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            write_json(&directory, "pool.json", &invalid);
            assert!(readiness_values(&directory).is_err(), "{pointer}");
        }
        let text = serde_json::to_string(&pool).unwrap();
        for duplicated in [
            text.replacen("\"schema\":6", "\"schema\":6,\"schema\":6", 1),
            text.replacen("\"readiness\":[]", "\"readiness\":[],\"readiness\":[]", 1),
        ] {
            std::fs::write(directory.join("pool.json"), duplicated).unwrap();
            assert!(readiness_values(&directory).is_err());
        }
        write_json(&directory, "pool.json", &pool);
        for duplicate_query in [true, false] {
            let duplicate = if duplicate_query { &query } else { &pool };
            write_json(&directory, "copy1.json", duplicate);
            write_json(&directory, "copy2.json", duplicate);
            let values = readiness_values(&directory).unwrap();
            for p in values
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["producer"] == "rustc-thread-pool-join")
            {
                let report = &p["latch_readiness_dag"];
                assert_eq!(report["records_complete"], false);
                assert!(report["known_cpu_work_ns"].is_null());
                assert!(report["longest_known_chain_cpu_ns"].is_null());
                assert!(
                    report["longest_known_chain_path"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                assert!(p["observed_cpu"]["query_execution_cpu_ns"].is_null());
            }
            std::fs::remove_file(directory.join("copy1.json")).unwrap();
            std::fs::remove_file(directory.join("copy2.json")).unwrap();
        }
        std::fs::remove_file(directory.join("query.json")).unwrap();
        let values = readiness_values(&directory).unwrap();
        assert_eq!(
            pool_value(&values)["latch_readiness_dag"]["records_complete"],
            false
        );
        assert!(pool_value(&values)["observed_cpu"]["query_execution_cpu_ns"].is_null());
        write_json(&directory, "query.json", &query);
        for identity in [&pool["capture_id"], &query["capture_id"]] {
            let scheduler = json!({"schema":1,"producer":"cargo-scheduler","scope":"cargo-unit-scheduler",
                "capture_id":identity,"declared_nodes":0,"declared_edges":0,"structural_complete":false,
                "full_pipeline_complete":false,"issues":[],"opaque_boundaries":["native"],"events":[],"nodes":[],"edges":[]});
            write_json(&directory, "scheduler.json", &scheduler);
            let values = readiness_values(&directory).unwrap();
            let p = pool_value(&values);
            let report = &p["latch_readiness_dag"];
            assert_eq!(report["records_complete"], false);
            assert!(report["known_cpu_work_ns"].is_null());
            assert!(report["longest_known_chain_cpu_ns"].is_null());
            assert!(
                report["longest_known_chain_path"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            assert!(p["observed_cpu"]["query_execution_cpu_ns"].is_null());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn latch_readiness_missing_original_registered_encounter_cannot_claim_complete() {
        let directory = resource_test_directory("readiness-original-wait-gap");
        let (query, _) = flow_documents();
        let (_, pool) = readiness_documents();
        // Both independent inventories are individually well formed. The
        // retained original registered wait still requires an encounter witness.
        write_json(&directory, "query.json", &query);
        write_json(&directory, "pool.json", &pool);
        let values = readiness_values(&directory).unwrap();
        let report = &pool_value(&values)["latch_readiness_dag"];
        assert_eq!(report["records_complete"], false);
        assert_eq!(report["wait_records_complete"], false);
        assert_eq!(report["matched_readiness_transfers"], 0);
        assert_eq!(report["full_pipeline_complete"], false);
        assert!(report["cpu_work_ns"].is_null() && report["execution_span_ns"].is_null());
        std::fs::remove_dir_all(directory).unwrap();
    }
    fn pool_value(values: &serde_json::Value) -> &serde_json::Value {
        values
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["producer"] == "rustc-thread-pool-join")
            .unwrap()
    }
    #[test]
    fn execution_flow_only_is_explicit_and_internally_qualifies_original_query_cpu() {
        let directory = resource_test_directory("execution-flow-policy");
        let (query, pool) = flow_documents();
        write_json(&directory, "query.json", &query);
        write_json(&directory, "pool.json", &pool);
        assert!(collect_path(&directory).is_err());
        assert!(query_cpu_values(&directory).is_err());
        assert!(task_dag_values(&directory, true).is_err());
        let values = flow_values(&directory).unwrap();
        let p = pool_value(&values);
        let flow = &p["execution_flow_dag"];
        assert_eq!(flow["records_complete"], true);
        assert_eq!(flow["known_cpu_work_ns"], 9);
        assert_eq!(flow["longest_known_chain_cpu_ns"], 6);
        assert_eq!(p["observed_cpu"]["query_execution_cpu_ns"], 6);
        for value in values.as_array().unwrap() {
            assert!(value.get("query_cpu_ownership").is_none());
            assert!(value.get("dispatch_result_provenance").is_none());
            assert!(value.get("cache_call_provenance").is_none());
            assert!(value["cpu_work_ns"].is_null() && value["execution_span_ns"].is_null());
            assert_eq!(value["full_pipeline_complete"], false);
        }
        let mut root6 = crate::execution_graph_query::tests::dispatch_fixture();
        root6["capture_id"] = query["capture_id"].clone();
        root6["execution_clock"] = query["execution_clock"].clone();
        root6["events"][3]["at_ns"] = json!(30);
        write_json(&directory, "query.json", &root6);
        let values = flow_values(&directory).unwrap();
        assert_eq!(
            pool_value(&values)["execution_flow_dag"]["records_complete"],
            true
        );
        assert!(
            values
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v.get("dispatch_result_provenance").is_none()
                    && v.get("query_cpu_ownership").is_none())
        );
        let policy = crate::execution_graph_query::CapturePolicy {
            execution_flow_dag: true,
            query_cpu_ownership: true,
            observed_task_dag: true,
            ..crate::execution_graph_query::CapturePolicy::strict()
        };
        let all = serde_json::to_value(
            collect_path_with_options(&directory, policy, true, true).unwrap(),
        )
        .unwrap();
        assert_eq!(
            pool_value(&all)["execution_flow_dag"]["records_complete"],
            true
        );
        assert_eq!(pool_value(&all)["observed_task_dag"]["attached_cpu_ns"], 6);
        let q = all
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["producer"] == "rustc-query-admission")
            .unwrap();
        assert_eq!(q["query_cpu_ownership"]["query_execution_cpu_ns"], 6);
        assert!(
            q.get("dispatch_result_provenance").is_some()
                && q.get("cache_call_provenance").is_some()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn execution_flow_does_not_launder_legacy_protocol_or_change_legacy_output() {
        let directory = resource_test_directory("execution-flow-legacy");
        let (query, pool4) = query_cpu_documents();
        write_json(&directory, "query.json", &query);
        write_json(&directory, "pool.json", &pool4);
        assert!(
            flow_values(&directory).is_err(),
            "flow option must not admit pool4 implicitly"
        );
        let mut pool3 = pool4.clone();
        pool3["schema"] = json!(3);
        pool3["timing"]["schema"] = json!(1);
        pool3["timing"]["segments"][0]["owner_kind"] = json!("task");
        for row in pool3["timing"]["segments"].as_array_mut().unwrap() {
            for field in ["query_context", "query_job", "query_name"] {
                row.as_object_mut().unwrap().remove(field);
            }
        }
        write_json(&directory, "pool.json", &pool3);
        let baseline = serde_json::to_value(collect_path(&directory).unwrap()).unwrap();
        assert_eq!(flow_values(&directory).unwrap(), baseline);
        let (_, pool5) = flow_documents();
        for value in [serde_json::Value::Null, pool5["execution_flow"].clone()] {
            let mut old = pool3.clone();
            old["execution_flow"] = value;
            write_json(&directory, "pool.json", &old);
            assert!(flow_values(&directory).is_err());
        }
        for remove in [true, false] {
            let mut invalid = pool5.clone();
            if remove {
                invalid.as_object_mut().unwrap().remove("execution_flow");
            } else {
                invalid["execution_flow"] = serde_json::Value::Null;
            }
            write_json(&directory, "pool.json", &invalid);
            assert!(flow_values(&directory).is_err());
        }
        let original = serde_json::to_string(&pool5).unwrap();
        let duplicate = original.replacen(
            "\"execution_flow\":",
            &format!(
                "\"execution_flow\":{},\"execution_flow\":",
                serde_json::to_string(&pool5["execution_flow"]).unwrap()
            ),
            1,
        );
        std::fs::write(directory.join("pool.json"), duplicate).unwrap();
        assert!(
            flow_values(&directory).is_err(),
            "duplicate original envelope keys must not be projected away"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn execution_flow_original_admission_lifecycle_clock_and_duplicate_authorities_are_strict() {
        let directory = resource_test_directory("execution-flow-authority");
        let (query, pool) = flow_documents();
        write_json(&directory, "pool.json", &pool);
        let missing = flow_values(&directory).unwrap();
        assert_eq!(
            pool_value(&missing)["execution_flow_dag"]["records_complete"],
            false
        );
        assert_eq!(
            pool_value(&missing)["observed_cpu"]["query_execution_cpu_ns"],
            serde_json::Value::Null
        );
        write_json(&directory, "query.json", &query);
        for (pointer, value) in [
            ("/execution_flow/scopes/0/query_job", json!(99)),
            ("/execution_flow/nodes/0/at_ns", json!(0)),
            ("/execution_flow/nodes/1/segment_id", json!(99)),
            ("/execution_flow/edges/0/to", json!(99)),
        ] {
            let mut invalid = pool.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            write_json(&directory, "pool.json", &invalid);
            assert!(flow_values(&directory).is_err(), "{pointer}");
        }
        write_json(&directory, "pool.json", &pool);
        let mut foreign_clock = query.clone();
        foreign_clock["execution_clock"]["pool_epoch_in_query_ns"] = json!(1);
        write_json(&directory, "query.json", &foreign_clock);
        let unaligned = flow_values(&directory).unwrap();
        assert_eq!(
            pool_value(&unaligned)["execution_flow_dag"]["records_complete"],
            false
        );
        write_json(&directory, "query.json", &query);
        for n in 1..=2 {
            write_json(&directory, &format!("pool-copy-{n}.json"), &pool);
        }
        for p in flow_values(&directory)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["producer"] == "rustc-thread-pool-join")
        {
            assert_eq!(p["execution_flow_dag"]["records_complete"], false);
            assert!(p["execution_flow_dag"]["known_cpu_work_ns"].is_null());
            assert!(
                p["execution_flow_dag"]["longest_known_chain_path"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        std::fs::remove_file(directory.join("pool-copy-1.json")).unwrap();
        std::fs::remove_file(directory.join("pool-copy-2.json")).unwrap();
        write_json(&directory, "query-copy.json", &query);
        write_json(&directory, "query-copy2.json", &query);
        let duplicated = flow_values(&directory).unwrap();
        let report = &pool_value(&duplicated)["execution_flow_dag"];
        assert_eq!(report["records_complete"], false);
        assert!(report["known_cpu_work_ns"].is_null());
        std::fs::remove_file(directory.join("query-copy.json")).unwrap();
        std::fs::remove_file(directory.join("query-copy2.json")).unwrap();
        let scheduler = json!({"schema":1,"producer":"cargo-scheduler","scope":"cargo-unit-scheduler","capture_id":pool["capture_id"],"declared_nodes":0,"declared_edges":0,"structural_complete":false,"full_pipeline_complete":false,"issues":[],"opaque_boundaries":["native"],"events":[],"nodes":[],"edges":[]});
        write_json(&directory, "scheduler.json", &scheduler);
        let collision = flow_values(&directory).unwrap();
        let report = &pool_value(&collision)["execution_flow_dag"];
        assert_eq!(report["records_complete"], false);
        assert!(report["known_cpu_work_ns"].is_null());
        assert!(
            report["longest_known_chain_path"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn observed_task_dag_requires_explicit_cpu_protocol_and_reciprocal_query_authority() {
        let directory = resource_test_directory("task-dag-authority");
        let (query, pool) = query_cpu_documents();
        write_json(&directory, "pool.json", &pool);
        assert!(task_dag_values(&directory, false).is_err());
        let lone = task_dag_values(&directory, true).unwrap();
        let dag = &lone[0]["observed_task_dag"];
        assert_eq!(dag["topology_complete"], true);
        assert_eq!(dag["records_complete"], false);
        assert!(dag["attached_cpu_ns"].is_null() && dag["known_attached_cpu_ns"].is_null());
        assert_eq!(dag["excluded_opaque_cpu_ns"], 3);
        assert!(dag["nodes"][0]["cpu_ns"].is_null());
        write_json(&directory, "query.json", &query);
        let complete = task_dag_values(&directory, true).unwrap();
        let p = complete
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["producer"] == "rustc-thread-pool-join")
            .unwrap();
        assert_eq!(p["observed_task_dag"]["records_complete"], true);
        assert_eq!(p["observed_task_dag"]["attached_cpu_ns"], 6);
        assert_eq!(
            p["observed_task_dag"]["components"][0]["longest_recorded_fork_join_chain_cpu_ns"],
            6
        );
        let baseline = query_cpu_values(&directory).unwrap();
        let mut without_dag = complete;
        for value in without_dag.as_array_mut().unwrap() {
            value.as_object_mut().unwrap().remove("observed_task_dag");
        }
        assert_eq!(without_dag, baseline);
        write_json(&directory, "query-copy.json", &query);
        let duplicate_query = task_dag_values(&directory, true).unwrap();
        let dag = &duplicate_query
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["producer"] == "rustc-thread-pool-join")
            .unwrap()["observed_task_dag"];
        assert_eq!(dag["topology_complete"], true);
        assert!(dag["known_attached_cpu_ns"].is_null());
        assert_eq!(dag["excluded_opaque_cpu_ns"], 3);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn observed_task_dag_local_legacy_pool_and_root6_only_do_not_enable_unrelated_outputs() {
        let directory = resource_test_directory("task-dag-legacy");
        let mut pool = crate::execution_graph_continuation::tests::schema3();
        write_json(&directory, "pool.json", &pool);
        let lone = task_dag_values(&directory, false).unwrap();
        assert_eq!(lone[0]["observed_task_dag"]["records_complete"], true);
        assert_eq!(lone[0]["observed_task_dag"]["attached_cpu_ns"], 1);
        let mut explicit = lone;
        explicit[0]
            .as_object_mut()
            .unwrap()
            .remove("observed_task_dag");
        assert_eq!(
            explicit,
            serde_json::to_value(collect_path(&directory).unwrap()).unwrap()
        );
        let mut query = crate::execution_graph_query::tests::dispatch_fixture();
        query["capture_id"] = pool["query_capture_id"].clone();
        query["execution_clock"] = pool["timing"]["clock_bridge"].clone();
        pool["capture_id"] = json!("pool-1");
        pool["timing"]["clock_bridge"]["pool_capture_id"] = json!("pool-1");
        query["execution_clock"]["pool_capture_id"] = json!("pool-1");
        write_json(&directory, "pool.json", &pool);
        write_json(&directory, "query.json", &query);
        let values = task_dag_values(&directory, false).unwrap();
        let q = values
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["producer"] == "rustc-query-admission")
            .unwrap();
        assert_eq!(q["schema"], 6);
        for key in [
            "query_cpu_ownership",
            "cache_call_provenance",
            "dispatch_result_provenance",
        ] {
            assert!(q.get(key).is_none(), "{key}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn observed_task_dag_all_duplicate_pool_copies_and_cross_producer_collisions_fail_closed() {
        for copies in [2, 3] {
            let directory = resource_test_directory("task-dag-duplicates");
            let pool = crate::execution_graph_continuation::tests::schema3();
            for n in 0..copies {
                write_json(&directory, &format!("pool-{n}.json"), &pool);
            }
            for p in task_dag_values(&directory, false)
                .unwrap()
                .as_array()
                .unwrap()
            {
                assert_eq!(p["observed_task_dag"]["topology_complete"], false);
                assert!(
                    p["observed_task_dag"]["nodes"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                assert!(p["observed_task_dag"]["known_attached_cpu_ns"].is_null());
            }
            std::fs::remove_dir_all(directory).unwrap();
        }
        let directory = resource_test_directory("task-dag-collision");
        let (_, pool) = query_cpu_documents();
        // A separately valid scheduler receipt with the pool's ID must also
        // invalidate pool authority, regardless of producer or file order.
        let scheduler = json!({"schema":1,"producer":"cargo-scheduler","scope":"cargo-unit-scheduler","capture_id":pool["capture_id"],"declared_nodes":0,"declared_edges":0,"structural_complete":false,"full_pipeline_complete":false,"issues":[],"opaque_boundaries":["native"],"events":[],"nodes":[],"edges":[]});
        write_json(&directory, "pool.json", &pool);
        write_json(&directory, "scheduler.json", &scheduler);
        let values = task_dag_values(&directory, true).unwrap();
        let p = values
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["producer"] == "rustc-thread-pool-join")
            .unwrap();
        assert_eq!(p["observed_task_dag"]["topology_complete"], false);
        assert!(
            p["observed_task_dag"]["nodes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn query_cpu_values(directory: &Path) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(collect_path_with_options(
            directory,
            query_cpu_policy(),
            false,
            false,
        )?)?)
    }

    #[test]
    fn query_cpu_ownership_is_explicit_disjoint_and_admission_scoped() {
        let directory = resource_test_directory("query-cpu");
        let (query, mut pool) = query_cpu_documents();
        write_json(&directory, "query.json", &query);
        for task in [json!(1), serde_json::Value::Null] {
            pool["timing"]["segments"][0]["task"] = task;
            write_json(&directory, "pool.json", &pool);
            assert!(collect_path(&directory).is_err());
            let values = query_cpu_values(&directory).unwrap();
            let q = values
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["producer"] == "rustc-query-admission")
                .unwrap();
            let p = values
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["producer"] == "rustc-thread-pool-join")
                .unwrap();
            let report = &q["query_cpu_ownership"];
            assert_eq!(report["query_execution_cpu_ns"], 6);
            assert_eq!(report["measured_segments"], 1);
            assert_eq!(
                report["queries"],
                json!([{"query_context":1,"query_job":1,"query_name":"type_of","cpu_ns":6}])
            );
            assert_eq!(report["records_complete"], true);
            let cpu = &p["observed_cpu"];
            assert_eq!(cpu["query_execution_cpu_ns"], 6);
            assert_eq!(cpu["task_cpu_ns"], 0);
            assert_eq!(cpu["opaque_cpu_ns"], 3);
            assert_eq!(cpu["observed_cpu_ns"], 9);
            assert_eq!(cpu["query_clock_aligned"], true);
            for value in values.as_array().unwrap() {
                assert!(value["cpu_work_ns"].is_null());
                assert!(value["execution_span_ns"].is_null());
                assert_eq!(value["full_pipeline_complete"], false);
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn query_cpu_ownership_rejects_foreign_pairs_lifetimes_and_schema_laundering() {
        let directory = resource_test_directory("query-cpu-forged");
        let (query, pool) = query_cpu_documents();
        write_json(&directory, "query.json", &query);
        for (pointer, value) in [
            ("/timing/segments/0/query_context", json!(2)),
            ("/timing/segments/0/query_job", json!(3)),
            ("/timing/segments/0/query_job", json!(2)), // job2 exists but lifetime is 10..11
            ("/timing/segments/0/query_job", json!(0)),
            ("/timing/segments/0/query_job", serde_json::Value::Null),
            ("/timing/segments/0/owner_kind", json!("task")),
            ("/timing/segments/0/task", json!(2)),
            ("/timing/schema", json!(1)),
            ("/schema", json!(3)),
            ("/cpu_work_ns", json!(9)),
            ("/full_pipeline_complete", json!(true)),
        ] {
            let mut forged = pool.clone();
            *forged.pointer_mut(pointer).unwrap() = value;
            write_json(&directory, "pool.json", &forged);
            assert!(query_cpu_values(&directory).is_err(), "{pointer}");
        }
        for field in ["query_context", "query_job", "query_name"] {
            for index in [0, 1] {
                let mut forged = pool.clone();
                forged["timing"]["segments"][index]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                write_json(&directory, "pool.json", &forged);
                assert!(
                    query_cpu_values(&directory).is_err(),
                    "missing {field} row {index}"
                );
            }
        }
        let raw = serde_json::to_string(&pool).unwrap();
        for (from, to) in [
            ("\"query_job\":1", "\"query_job\":1,\"query_job\":1"),
            (
                "\"query_context\":null",
                "\"query_context\":null,\"query_context\":null",
            ),
            ("\"schema\":4", "\"schema\":4,\"schema\":4"),
        ] {
            std::fs::write(directory.join("pool.json"), raw.replacen(from, to, 1)).unwrap();
            assert!(query_cpu_values(&directory).is_err(), "duplicate {from}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn query_cpu_ownership_never_borrows_missing_incomplete_or_duplicate_authority() {
        let directory = resource_test_directory("query-cpu-authority");
        let (query, pool) = query_cpu_documents();
        write_json(&directory, "pool.json", &pool);
        let lone = query_cpu_values(&directory).unwrap();
        assert!(lone[0]["observed_cpu"]["query_execution_cpu_ns"].is_null());
        assert_eq!(lone[0]["observed_cpu"]["records_complete"], false);
        for (pointer, value) in [
            ("/capture_complete", json!(false)),
            ("/lost_events", json!(1)),
            ("/interface_outcome", json!("interrupted")),
            ("/execution_clock", serde_json::Value::Null),
            ("/execution_clock/pool_observer", json!(2)),
            ("/events/3/at_ns", json!(0)),
            ("/events/3/job_id", json!(2)),
        ] {
            let mut forged = query.clone();
            *forged.pointer_mut(pointer).unwrap() = value;
            write_json(&directory, "query.json", &forged);
            let values = query_cpu_values(&directory).unwrap();
            for value in values.as_array().unwrap() {
                if value["producer"] == "rustc-query-admission" {
                    assert!(
                        value["query_cpu_ownership"]["query_execution_cpu_ns"].is_null(),
                        "{pointer}"
                    );
                    assert_eq!(value["query_cpu_ownership"]["records_complete"], false);
                } else {
                    assert!(
                        value["observed_cpu"]["query_execution_cpu_ns"].is_null(),
                        "{pointer}"
                    );
                }
            }
        }
        write_json(&directory, "query.json", &query);
        for (name, duplicate) in [
            ("duplicate-query.json", &query),
            ("duplicate-pool.json", &pool),
        ] {
            write_json(&directory, name, duplicate);
            let values = query_cpu_values(&directory).unwrap();
            for value in values.as_array().unwrap() {
                assert!(value["observed_cpu"]["query_execution_cpu_ns"].is_null());
                assert!(value["query_cpu_ownership"]["query_execution_cpu_ns"].is_null());
            }
            std::fs::remove_file(directory.join(name)).unwrap();
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn query_cpu_ownership_honest_loss_is_partial_and_root6_does_not_enable_dispatch_output() {
        let directory = resource_test_directory("query-cpu-loss");
        let (query, mut pool) = query_cpu_documents();
        write_json(&directory, "query.json", &query);
        pool["timing"]["records_complete"] = json!(false);
        pool["timing"]["lost_segments"] = json!(1);
        pool["timing"]["segment_allocator_high_water"] = json!(4);
        pool["timing"]["expected_segments"] = json!(3);
        write_json(&directory, "pool.json", &pool);
        let values = query_cpu_values(&directory).unwrap();
        let q = values
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["producer"] == "rustc-query-admission")
            .unwrap();
        assert_eq!(q["query_cpu_ownership"]["query_execution_cpu_ns"], 6);
        assert_eq!(q["query_cpu_ownership"]["records_complete"], false);
        let mut root6 = crate::execution_graph_query::tests::dispatch_fixture();
        root6["capture_id"] = query["capture_id"].clone();
        root6["execution_clock"] = query["execution_clock"].clone();
        root6["events"][3]["at_ns"] = json!(30);
        write_json(&directory, "query.json", &root6);
        let values = query_cpu_values(&directory).unwrap();
        let q = values
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["producer"] == "rustc-query-admission")
            .unwrap();
        assert_eq!(q["schema"], 6);
        assert!(q.get("dispatch_result_provenance").is_none());
        assert_eq!(q["query_cpu_ownership"]["query_execution_cpu_ns"], 6);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn query_cpu_ownership_opt_in_preserves_every_legacy_measurement() {
        let directory = resource_test_directory("query-cpu-legacy");
        let (query, mut pool) = query_cpu_documents();
        pool["schema"] = json!(3);
        pool["timing"]["schema"] = json!(1);
        pool["timing"]["segments"][0]["owner_kind"] = json!("task");
        for row in pool["timing"]["segments"].as_array_mut().unwrap() {
            for field in ["query_context", "query_job", "query_name"] {
                row.as_object_mut().unwrap().remove(field);
            }
        }
        write_json(&directory, "query.json", &query);
        write_json(&directory, "pool.json", &pool);
        let legacy = serde_json::to_value(collect_path(&directory).unwrap()).unwrap();
        let mut explicit = query_cpu_values(&directory).unwrap();
        for value in explicit.as_array_mut().unwrap() {
            if value["producer"] == "rustc-query-admission" {
                let report = value
                    .as_object_mut()
                    .unwrap()
                    .remove("query_cpu_ownership")
                    .unwrap();
                assert!(report["query_execution_cpu_ns"].is_null());
                assert_eq!(report["records_complete"], false);
            }
        }
        assert_eq!(
            serde_json::to_vec(&explicit).unwrap(),
            serde_json::to_vec(&legacy).unwrap()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn query_cpu_ownership_all_copies_fail_closed_on_triples_and_cross_producer_collisions() {
        for scenario in [
            "triple-query",
            "triple-pool",
            "triple-both",
            "resource-query",
            "resource-pool",
        ] {
            let directory = resource_test_directory(scenario);
            let (query, pool) = query_cpu_documents();
            write_json(&directory, "query.json", &query);
            write_json(&directory, "pool.json", &pool);
            if matches!(scenario, "triple-query" | "triple-both") {
                write_json(&directory, "query-copy-2.json", &query);
                write_json(&directory, "query-copy-3.json", &query);
            }
            if matches!(scenario, "triple-pool" | "triple-both") {
                write_json(&directory, "pool-copy-2.json", &pool);
                write_json(&directory, "pool-copy-3.json", &pool);
            }
            if matches!(scenario, "resource-query" | "resource-pool") {
                let mut resource = crate::execution_graph_resource::tests::fixture();
                resource["capture_id"] = if scenario == "resource-query" {
                    query["capture_id"].clone()
                } else {
                    pool["capture_id"].clone()
                };
                write_json(&directory, "resource-collision.json", &resource);
            }
            let values = query_cpu_values(&directory).unwrap();
            let queries = values
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["producer"] == "rustc-query-admission")
                .collect::<Vec<_>>();
            let pools = values
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["producer"] == "rustc-thread-pool-join")
                .collect::<Vec<_>>();
            assert_eq!(
                queries.len(),
                if matches!(scenario, "triple-query" | "triple-both") {
                    3
                } else {
                    1
                }
            );
            assert_eq!(
                pools.len(),
                if matches!(scenario, "triple-pool" | "triple-both") {
                    3
                } else {
                    1
                }
            );
            for query in queries {
                let report = &query["query_cpu_ownership"];
                assert!(report["query_execution_cpu_ns"].is_null(), "{scenario}");
                assert_eq!(report["queries"], json!([]), "{scenario}");
                assert_eq!(report["records_complete"], false, "{scenario}");
            }
            for pool in pools {
                let cpu = &pool["observed_cpu"];
                assert!(cpu["query_execution_cpu_ns"].is_null(), "{scenario}");
                assert_eq!(cpu["records_complete"], false, "{scenario}");
                assert_eq!(cpu["query_clock_aligned"], false, "{scenario}");
            }
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn pool3_query5_original_bytes_reconcile_clock_without_full_work_claims() {
        let directory = resource_test_directory("timing-clock");
        let docs = crate::execution_graph_resource_attachment::tests::documents();
        let mut query = docs[0].clone();
        let mut pool = docs[1].clone();
        let mut timing = crate::execution_graph_timing::tests::fixture();
        timing["clock_bridge"]["pool_capture_id"] = pool["capture_id"].clone();
        pool["schema"] = json!(3);
        pool["timing"] = timing;
        query["schema"] = json!(5);
        query["execution_clock"] = pool["timing"]["clock_bridge"].clone();
        write_json(&directory, "query.json", &query);
        write_json(&directory, "pool.json", &pool);
        let a = collect_path(&directory).unwrap();
        let q = a.iter().find(|r| r.schema == 5).unwrap();
        assert!(q.clock_alignment.as_ref().unwrap().aligned);
        assert!(q.task_bindings.as_ref().unwrap().graph_binding_complete);
        let p = a.iter().find(|r| r.schema == 3).unwrap();
        let cpu = p.observed_cpu.as_ref().unwrap();
        assert_eq!(cpu.observed_cpu_ns, Some(9));
        assert!(cpu.records_complete);
        assert!(!cpu.query_clock_aligned);
        assert!(a.iter().all(|r| r.cpu_work_ns.is_none()
            && r.execution_span_ns.is_none()
            && !r.full_pipeline_complete));
        // A unilateral descriptor edit cannot borrow an otherwise valid pool CPU observation.
        query["execution_clock"]["pool_observer"] = json!(2);
        write_json(&directory, "query.json", &query);
        let a = collect_path(&directory).unwrap();
        assert!(
            !a.iter()
                .find(|r| r.schema == 5)
                .unwrap()
                .clock_alignment
                .as_ref()
                .unwrap()
                .aligned
        );
        assert_eq!(
            a.iter()
                .find(|r| r.schema == 3)
                .unwrap()
                .observed_cpu
                .as_ref()
                .unwrap()
                .observed_cpu_ns,
            Some(9)
        );
        query["execution_clock"] = pool["timing"]["clock_bridge"].clone();
        // Honest timing loss preserves lifecycle/clock authority, never CPU completeness.
        pool["timing"]["records_complete"] = json!(false);
        pool["timing"]["lost_segments"] = json!(1);
        pool["timing"]["segment_allocator_high_water"] = json!(4);
        pool["timing"]["expected_segments"] = json!(3);
        write_json(&directory, "query.json", &query);
        write_json(&directory, "pool.json", &pool);
        let a = collect_path(&directory).unwrap();
        assert!(
            a.iter()
                .find(|r| r.schema == 5)
                .unwrap()
                .clock_alignment
                .as_ref()
                .unwrap()
                .aligned
        );
        assert!(
            !a.iter()
                .find(|r| r.schema == 3)
                .unwrap()
                .observed_cpu
                .as_ref()
                .unwrap()
                .records_complete
        );
        // A schema4 query has no reciprocal execution clock, even beside a valid pool3.
        query["schema"] = json!(4);
        query.as_object_mut().unwrap().remove("execution_clock");
        write_json(&directory, "query.json", &query);
        let a = collect_path(&directory).unwrap();
        assert!(
            a.iter()
                .find(|r| r.schema == 4)
                .unwrap()
                .clock_alignment
                .is_none()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pool3_duplicate_bytes_and_duplicate_capture_authority_fail_closed() {
        let directory = resource_test_directory("timing-duplicates");
        let docs = crate::execution_graph_resource_attachment::tests::documents();
        let mut query = docs[0].clone();
        let mut pool = docs[1].clone();
        let mut timing = crate::execution_graph_timing::tests::fixture();
        timing["clock_bridge"]["pool_capture_id"] = pool["capture_id"].clone();
        query["schema"] = json!(5);
        query["execution_clock"] = timing["clock_bridge"].clone();
        pool["schema"] = json!(3);
        pool["timing"] = timing;
        write_json(&directory, "query.json", &query);
        write_json(&directory, "pool.json", &pool);
        write_json(&directory, "duplicate.json", &query);
        let a = collect_path(&directory).unwrap();
        assert!(
            a.iter()
                .filter(|r| r.schema == 5)
                .all(|r| !r.structural_complete && !r.clock_alignment.as_ref().unwrap().aligned)
        );
        std::fs::remove_file(directory.join("duplicate.json")).unwrap();
        let raw = serde_json::to_string(&pool).unwrap();
        let duplicate = raw.replacen("\"cpu_ns\":6", "\"cpu_ns\":6,\"cpu_ns\":6", 1);
        assert_ne!(raw, duplicate);
        std::fs::write(directory.join("pool.json"), duplicate).unwrap();
        assert!(collect_path(&directory).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn large_capture_path_preserves_total_bytes_identity_and_symlink_bounds() {
        use crate::execution_graph_query::{CapturePolicy, tests::task_binding_fixture};
        let directory = resource_test_directory("large-policy");
        let fixture = task_binding_fixture();
        let bytes = serde_json::to_vec(&fixture).unwrap();
        let file = directory.join("a.json");
        std::fs::write(&file, &bytes).unwrap();
        let policy = CapturePolicy {
            max_bytes: bytes.len(),
            max_points: 7,
            query_cpu_ownership: false,
            observed_task_dag: false,
            execution_flow_dag: false,
            latch_readiness_dag: false,
        };
        assert!(collect_path_with_policy(&file, policy).unwrap()[0].structural_complete);
        assert!(collect_path_with_policy(&directory, policy).unwrap()[0].structural_complete);
        assert!(
            collect_path_with_policy(
                &file,
                CapturePolicy {
                    max_bytes: bytes.len() - 1,
                    ..policy
                }
            )
            .is_err()
        );
        std::fs::write(directory.join("b.json"), &bytes).unwrap();
        assert!(collect_path_with_policy(&directory, policy).is_err());
        let duplicates = collect_path_with_policy(&directory, CapturePolicy::large()).unwrap();
        assert_eq!(duplicates.len(), 2);
        assert!(duplicates.iter().all(|a| !a.structural_complete
            && a.issues.iter().any(|s| s == "duplicate capture identity")));
        assert!(command_with_policy(&directory, true, Some(&file), true).is_err());
        #[cfg(unix)]
        {
            let link = directory.join("link.json");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert!(collect_path_with_policy(&link, CapturePolicy::large()).is_err());
            assert!(collect_path_with_policy(&directory, CapturePolicy::large()).is_err());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn resource_summary(assessments: &[Assessment]) -> serde_json::Value {
        serde_json::to_value(assessments[0].resource_attachments.as_ref().unwrap()).unwrap()
    }

    #[test]
    fn public_resource_absence_is_unknown_and_malformed_authority_is_independent() {
        let directory = resource_test_directory("independent");
        let query = crate::execution_graph_query::tests::task_binding_fixture();
        write_json(&directory, "a.json", &query);
        let assessments = collect_path(&directory).unwrap();
        assert_eq!(resource_summary(&assessments)["state"], "unknown");
        assert!(assessments[0].structural_complete);
        for (name, document) in [
            (
                "resource.manifest.json",
                json!({
                    "schema":1,"producer":"rustc-frontend-allowance",
                    "capture_id":"invalid-resource","scope":"selected-ticketed-grant-decisions",
                    "profile":42
                }),
            ),
            (
                "endorsement.envelope.json",
                json!({
                    "schema":1,"producer":"rustc-frontend-admission-attachment",
                    "capture_id":"invalid-endorsement","scope":"selected-resource-query-pool-attachment"
                }),
            ),
        ] {
            write_json(&directory, name, &document);
        }
        let assessments = collect_path(&directory).unwrap();
        assert_eq!(
            assessments.len(),
            3,
            "content, not owner-like filenames, routes resources"
        );
        let query = assessments
            .iter()
            .find(|a| a.producer == "rustc-query-admission")
            .unwrap();
        assert!(query.structural_complete);
        assert_eq!(query.wait_structural_complete, Some(true));
        assert!(query.query_reads.as_ref().unwrap().records_complete);
        assert!(
            assessments
                .iter()
                .filter(|a| a.resource.is_some())
                .all(|a| !a.structural_complete)
        );
        assert_ne!(resource_summary(&assessments)["state"], "validated");
        command(&directory, true, None).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn write_resource_documents(directory: &Path, documents: &[serde_json::Value]) {
        for (index, document) in documents.iter().enumerate() {
            // No participant's authority comes from its path or ordering.
            let name = if document["producer"] == "rustc-frontend-allowance" {
                "resource.manifest.json".to_owned()
            } else {
                format!("copy-{index}.json")
            };
            write_json(directory, &name, document);
        }
    }

    #[test]
    fn public_resource_file_copies_missing_duplicate_foreign_tuple_and_endpoint_matrix() {
        let root = resource_test_directory("mutation-matrix");
        let original = crate::execution_graph_resource_attachment::tests::documents();
        let mut cases = vec![("valid".to_owned(), original.clone(), true)];
        for role in 3..6 {
            let mut documents = original.clone();
            documents.remove(role);
            cases.push((format!("missing-role-{role}"), documents, false));
        }
        let mut missing_all = original.clone();
        missing_all.truncate(3);
        cases.push(("missing-all-roles".into(), missing_all, false));
        for index in 1..6 {
            let mut documents = original.clone();
            documents.push(documents[index].clone());
            cases.push((format!("duplicate-{index}"), documents, false));
        }
        for field in [
            "attachment_capture_id",
            "attachment_id",
            "controller_id",
            "resource_capture_id",
            "query_capture_id",
            "query_root_namespace",
            "pool_capture_id",
            "pool_observer",
        ] {
            let mut documents = original.clone();
            documents[4]["attachment"][field] = if documents[4]["attachment"][field].is_string() {
                json!("foreign-authority")
            } else {
                json!(2)
            };
            cases.push((format!("foreign-{field}"), documents, false));
        }
        for (transaction, event, label) in [
            (1, 0, "request"),
            (1, 1, "grant"),
            (1, 2, "ack"),
            (2, 0, "returned"),
        ] {
            let mut documents = original.clone();
            documents[2]["transactions"][transaction]["events"]
                .as_array_mut()
                .unwrap()
                .remove(event);
            cases.push((format!("missing-{label}"), documents, false));
        }
        for kind in ["release", "helper"] {
            let mut documents = original.clone();
            documents[2]["transactions"][1]["events"][1]["source"] = if kind == "release" {
                json!({"kind":"release","release_id":99})
            } else {
                json!({"kind":"helper","callback_id":99})
            };
            cases.push((format!("foreign-source-{kind}"), documents, false));
        }
        for (field, value) in [
            ("wait_id", json!(2)),
            ("phase", json!("query_wait_release")),
            ("query_root_namespace", json!(2)),
        ] {
            let mut documents = original.clone();
            documents[2]["transactions"][1]["events"][0]["context"][field] = value;
            cases.push((format!("foreign-context-{field}"), documents, false));
        }
        for field in [
            "identity_exhausted",
            "counter_overflowed",
            "loss_overflowed",
        ] {
            let mut documents = original.clone();
            documents[2][field] = json!(true);
            documents[2]["records_complete"] = json!(false);
            cases.push((format!("capped-{field}"), documents, false));
        }
        for (label, documents, valid) in cases {
            let directory = root.join(&label);
            std::fs::create_dir(&directory).unwrap();
            write_resource_documents(&directory, &documents);
            // A duplicate prefix must be a second real file, not overwritten by
            // the fixture writer's descriptive resource filename.
            if label == "duplicate-2" {
                write_json(&directory, "second-prefix-copy.json", &documents[2]);
            }
            let assessments = collect_path(&directory).unwrap();
            let summary = resource_summary(&assessments);
            if label == "duplicate-2" {
                for assessment in assessments
                    .iter()
                    .filter(|a| a.producer == "rustc-frontend-allowance")
                {
                    let resource = assessment.resource.as_ref().unwrap();
                    assert_eq!(resource["prefix_records_complete"], false);
                    assert!(
                        resource["local_resource_edges"]
                            .as_array()
                            .unwrap()
                            .is_empty()
                    );
                }
                assert!(
                    summary["resources"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|resource| resource["local_resource_edges"]
                            .as_array()
                            .unwrap()
                            .is_empty())
                );
            }
            let links = summary["attachments"].as_array().unwrap();
            let cross: usize = links
                .iter()
                .map(|a| a["cross_wait_edges"].as_array().unwrap().len())
                .sum();
            assert_eq!(cross, usize::from(valid), "{label}: {summary}");
            if valid {
                assert_eq!(summary["state"], "validated", "{label}: {summary}");
            } else {
                assert!(
                    links.iter().all(|a| a["context_links_complete"] == false),
                    "{label}: {summary}"
                );
            }
            let query = assessments
                .iter()
                .find(|a| a.producer == "rustc-query-admission")
                .unwrap();
            assert!(
                query.structural_complete,
                "{label}: independent query validity changed"
            );
            assert_eq!(query.wait_structural_complete, Some(true), "{label}");
            assert!(
                query.query_reads.as_ref().unwrap().records_complete,
                "{label}"
            );
            assert_eq!(summary["full_pipeline_complete"], false);
            assert_eq!(summary["resource_full_complete"], false);
            assert!(summary["cpu_work_ns"].is_null() && summary["execution_span_ns"].is_null());
            command(&directory, true, None).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_resource_query_read_partial_retains_validated_wait_link() {
        let directory = resource_test_directory("read-partial");
        let mut documents = crate::execution_graph_resource_attachment::tests::documents();
        documents[0]["query_reads"]["records_complete"] = json!(false);
        documents[0]["query_reads"]["lost_events"] = json!(1);
        write_resource_documents(&directory, &documents);
        let assessments = collect_path(&directory).unwrap();
        let query = assessments
            .iter()
            .find(|a| a.producer == "rustc-query-admission")
            .unwrap();
        assert!(!query.structural_complete);
        assert_eq!(query.wait_structural_complete, Some(true));
        assert!(!query.query_reads.as_ref().unwrap().records_complete);
        let summary = resource_summary(&assessments);
        assert_eq!(
            summary["attachments"][0]["cross_wait_edges"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "{summary}"
        );
        command(&directory, true, None).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn public_resource_same_controller_distinct_prefixes_are_not_merged() {
        let directory = resource_test_directory("controller-prefixes");
        let documents = crate::execution_graph_resource_attachment::tests::documents();
        write_resource_documents(&directory, &documents);
        let mut later = documents[2].clone();
        later["capture_id"] = json!("later-prefix");
        later["attachment_capture_id"] = json!("later-attachment");
        write_json(&directory, "later-prefix.json", &later);
        let assessments = collect_path(&directory).unwrap();
        let summary = resource_summary(&assessments);
        let resources = summary["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 2);
        assert!(
            resources
                .iter()
                .all(|r| r["prefix_records_complete"] == true)
        );
        assert!(
            resources
                .iter()
                .all(|r| r["controller_id"] == "controller-1")
        );
        let links = summary["attachments"].as_array().unwrap();
        assert_eq!(
            links
                .iter()
                .flat_map(|a| a["cross_wait_edges"].as_array().unwrap())
                .count(),
            1
        );
        let later = links
            .iter()
            .find(|a| a["attachment_capture_id"] == "later-attachment")
            .unwrap();
        assert_eq!(later["state"], "incomplete");
        assert!(later["cross_wait_edges"].as_array().unwrap().is_empty());
        command(&directory, true, None).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn public_resource_lossfree_unreturned_tail_is_not_complete_context_mapping() {
        let directory = resource_test_directory("unreturned-tail");
        let mut documents = crate::execution_graph_resource_attachment::tests::documents();
        let prefix = &mut documents[2];
        prefix["transactions"].as_array_mut().unwrap().remove(2);
        prefix["transactions"][2]["transaction_id"] = json!(2);
        prefix["transactions"][2]["first_seq"] = json!(4);
        prefix["transactions"][2]["next_seq"] = json!(5);
        prefix["transactions"][2]["events"][0]["seq"] = json!(4);
        prefix["end"]["counters"]["acquire_returns"] = json!(0);
        prefix["end"]["counters"]["transactions_attempted"] = json!(3);
        prefix["end"]["counters"]["events_attempted"] = json!(5);
        prefix["end"]["allocators"]["acquire_return"]["high_water"] = json!(0);
        prefix["end"]["allocators"]["transaction"]["high_water"] = json!(3);
        prefix["end"]["allocators"]["event"]["high_water"] = json!(5);
        prefix["end"]["registered_inflight"]["acquire_cores_not_returned"] = json!(1);
        write_resource_documents(&directory, &documents);
        let assessments = collect_path(&directory).unwrap();
        let summary = resource_summary(&assessments);
        assert_eq!(
            summary["resources"][0]["prefix_records_complete"], true,
            "{summary}"
        );
        assert_eq!(summary["resources"][0]["closed_lifecycle"], false);
        assert_eq!(
            summary["attachments"][0]["context_links_complete"], false,
            "{summary}"
        );
        assert!(
            summary["attachments"][0]["cross_wait_edges"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        command(&directory, true, None).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn public_resource_original_bytes_duplicates_caps_and_symlink_safety() {
        let root = resource_test_directory("strict-bytes");
        let original = crate::execution_graph_resource_attachment::tests::documents();
        for (index, label) in [
            (2, "resource-duplicate-field"),
            (3, "endorsement-duplicate-field"),
            (2, "fact-cap"),
            (2, "byte-cap"),
        ] {
            let directory = root.join(label);
            std::fs::create_dir(&directory).unwrap();
            write_resource_documents(&directory, &original);
            let file = if index == 2 {
                directory.join("resource.manifest.json")
            } else {
                directory.join("copy-3.json")
            };
            let bytes = if label == "fact-cap" {
                let mut document = original[index].clone();
                let event = document["transactions"][0]["events"][0].clone();
                document["transactions"][0]["events"] =
                    serde_json::Value::Array(vec![event; 100_001]);
                serde_json::to_vec(&document).unwrap()
            } else if label == "byte-cap" {
                let mut bytes = serde_json::to_vec(&original[index]).unwrap();
                bytes.resize(32 * 1024 * 1024 + 1, b' ');
                bytes
            } else {
                serde_json::to_string(&original[index])
                    .unwrap()
                    .replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1)
                    .into_bytes()
            };
            std::fs::write(file, bytes).unwrap();
            let assessments = collect_path(&directory).unwrap();
            let summary = resource_summary(&assessments);
            assert_ne!(summary["state"], "validated", "{label}: {summary}");
            assert!(
                summary["attachments"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|a| a["cross_wait_edges"].as_array().unwrap().is_empty())
            );
            let query = assessments
                .iter()
                .find(|a| a.producer == "rustc-query-admission")
                .unwrap();
            assert!(query.structural_complete, "{label}");
            assert_eq!(query.wait_structural_complete, Some(true), "{label}");
            command(&directory, true, None).unwrap();
        }
        let aggregate = root.join("aggregate-cap");
        std::fs::create_dir(&aggregate).unwrap();
        for name in ["a.json", "b.json"] {
            let file = std::fs::File::create(aggregate.join(name)).unwrap();
            file.set_len(33 * 1024 * 1024).unwrap();
        }
        assert!(
            collect_path(&aggregate)
                .unwrap_err()
                .to_string()
                .contains("total receipt byte budget")
        );
        #[cfg(unix)]
        {
            let directory = root.join("symlink");
            std::fs::create_dir(&directory).unwrap();
            write_resource_documents(&directory, &original);
            std::os::unix::fs::symlink(
                directory.join("resource.manifest.json"),
                directory.join("copy.json"),
            )
            .unwrap();
            assert!(collect_path(&directory).is_err());
            let link = root.join("directory-link");
            std::os::unix::fs::symlink(&directory, &link).unwrap();
            assert!(collect_path(&link).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    fn receipt() -> Receipt {
        serde_json::from_value(serde_json::json!({
            "schema":1,"producer":"cargo-scheduler","scope":"cargo-unit-scheduler",
            "capture_id":"test","structural_complete":true,"full_pipeline_complete":false,
            "issues":[],"opaque_boundaries":["compiler internals"],
            "declared_nodes":2,"declared_edges":1,
            "events":[
                {"unit":0,"event":"start","at_ns":0},
                {"unit":0,"event":"artifact-ready","at_ns":4,"artifact":"rmeta"},
                {"unit":1,"event":"start","at_ns":5},
                {"unit":0,"event":"terminal","at_ns":10,"outcome":"success"},
                {"unit":1,"event":"terminal","at_ns":15,"outcome":"success"}
            ],
            "nodes":[
                {"id":0,"kind":"cargo-unit","start_ns":0,"end_ns":10,"outcome":"success"},
                {"id":1,"kind":"cargo-unit","start_ns":5,"end_ns":15,"outcome":"success"}
            ],"edges":[{"from":1,"to":0,"artifact":"rmeta"}]
        }))
        .unwrap()
    }

    #[test]
    fn valid_pipelined_scope_is_not_full_pipeline_or_cpu_accounting() {
        let a = assess(receipt()).unwrap();
        assert!(a.structural_complete);
        assert!(!a.full_pipeline_complete);
        assert_eq!(a.cpu_work_ns, None);
        assert_eq!(a.execution_span_ns, None);
    }

    #[test]
    fn missing_duplicate_and_invalid_lifecycles_are_incomplete() {
        for change in 0..5 {
            let mut r = receipt();
            match change {
                0 => r.nodes[0].outcome = None,
                1 => r.nodes[1].id = 0,
                2 => r.nodes[0].end_ns = Some(0),
                3 => r.nodes[0].outcome = Some("invented".into()),
                _ => r.edges[0].from = 99,
            }
            if change == 2 {
                r.nodes[0].start_ns = Some(1);
            }
            assert!(!assess(r).unwrap().structural_complete);
        }
    }

    #[test]
    fn cycle_and_unsupported_artifacts_are_incomplete() {
        let mut r = receipt();
        r.edges.push(Edge {
            from: 0,
            to: 1,
            artifact: "all".into(),
        });
        assert!(!assess(r).unwrap().structural_complete);
        let mut r = receipt();
        r.edges[0].artifact = "unknown".into();
        assert!(!assess(r).unwrap().structural_complete);
    }

    #[test]
    fn unknown_schema_and_false_full_claim_are_rejected() {
        let mut r = receipt();
        r.schema = 2;
        assert!(assess(r).is_err());
        let mut r = receipt();
        r.full_pipeline_complete = true;
        assert!(!assess(r).unwrap().structural_complete);
        let mut r = receipt();
        r.opaque_boundaries.clear();
        assert!(!assess(r).unwrap().structural_complete);
    }

    #[test]
    fn failed_and_not_started_nodes_can_be_accounted() {
        let mut r = receipt();
        r.nodes[0].outcome = Some("failure".into());
        r.events[3].outcome = Some("failure".into());
        r.events[4].outcome = Some("not_started".into());
        r.events.remove(2);
        r.nodes[1].outcome = Some("not_started".into());
        r.nodes[1].start_ns = None;
        r.nodes[1].end_ns = None;
        assert!(assess(r).unwrap().structural_complete);
        let mut r = receipt();
        r.issues.push("event loss".into());
        assert!(!assess(r).unwrap().structural_complete);
    }

    #[test]
    fn lost_events_and_unready_handoffs_are_incomplete() {
        let mut r = receipt();
        r.events.remove(0);
        assert!(!assess(r).unwrap().structural_complete);
        let mut r = receipt();
        r.events[1].at_ns = 6;
        assert!(!assess(r).unwrap().structural_complete);
        let mut r = receipt();
        r.declared_edges += 1;
        assert!(!assess(r).unwrap().structural_complete);
        let mut r = receipt();
        r.events[0].unit = 99;
        assert!(!assess(r).unwrap().structural_complete);
        let mut r = receipt();
        r.events.push(Event {
            unit: 0,
            event: "terminal".into(),
            at_ns: 20,
            artifact: None,
            outcome: Some("success".into()),
        });
        assert!(!assess(r).unwrap().structural_complete);
    }

    #[test]
    fn equal_timestamps_do_not_hide_reversed_lifecycles_or_handoffs() {
        for swap in [(0, 3), (1, 2)] {
            let mut r = receipt();
            for node in &mut r.nodes {
                node.start_ns = Some(0);
                node.end_ns = Some(0);
            }
            for event in &mut r.events {
                event.at_ns = 0;
            }
            r.events.swap(swap.0, swap.1);
            assert!(!assess(r).unwrap().structural_complete);
        }
        let mut r = receipt();
        for node in &mut r.nodes {
            node.start_ns = Some(0);
            node.end_ns = Some(0);
        }
        for event in &mut r.events {
            event.at_ns = 0;
        }
        assert!(assess(r).unwrap().structural_complete);
    }

    #[test]
    fn failed_or_cancelled_units_cannot_supply_all_artifacts() {
        for outcome in ["failure", "cancelled"] {
            let mut r = receipt();
            r.events[1].artifact = Some("all".into());
            r.edges[0].artifact = "all".into();
            r.nodes[0].outcome = Some(outcome.into());
            r.events[3].outcome = Some(outcome.into());
            assert!(!assess(r).unwrap().structural_complete);
        }
        // Successful metadata can legitimately unblock a consumer before the
        // producing compiler subsequently fails during code generation.
        let mut r = receipt();
        r.nodes[0].outcome = Some("failure".into());
        r.events[3].outcome = Some("failure".into());
        assert!(assess(r).unwrap().structural_complete);
    }

    #[test]
    fn malformed_and_symlink_receipts_are_rejected() {
        let dir = crate::paths::home()
            .unwrap()
            .join(format!("graph-file-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("malformed.json");
        std::fs::write(&file, b"{}").unwrap();
        assert!(read_receipt(&file).is_err());
        #[cfg(unix)]
        {
            let link = dir.join("link.json");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert!(read_receipt(&link).is_err());
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn strict_query_v2_routes_with_explicit_edge_classes() {
        let path = crate::paths::home()
            .unwrap()
            .join(format!("graph-query-routing-{}.json", std::process::id()));
        let mut v = json!({
            "schema":2,"producer":"rustc-query-admission","scope":"query-job-admission-lifecycle",
            "capture_id":"empty-waits","full_pipeline_complete":false,"cpu_work_ns":null,
            "execution_span_ns":null,"interface_outcome":"returned","capture_complete":true,
            "lost_events":0,"expected_contexts":0,"contexts":[],"events":[],
            "opaque_boundaries":["fast-cache-hits","race-cache-hits","green-ensure-no-job",
                "feeds","direct-paths","nonquery","resource-internals","driver-and-process-outcome",
                "cargo-parent-binding","cache-read-dependencies","wait-and-resource-dependencies"],
            "wait_allocator_baseline":1,"wait_allocator_high_water":1,"expected_waits":0,
            "wait_capture_complete":true,"wait_events":[]
        });
        std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        let a = read_receipt(&path).unwrap();
        assert!(a.structural_complete);
        let output = serde_json::to_value(a).unwrap();
        assert_eq!(output["wait_structural_complete"], true);
        assert_eq!(output["descriptive_parent_edges"], 0);
        assert_eq!(output["control_causal_edges"], 0);
        assert_eq!(output["waits"], 0);
        assert_eq!(output["cpu_work_ns"], serde_json::Value::Null);
        assert_eq!(output["execution_span_ns"], serde_json::Value::Null);
        assert_eq!(output["full_pipeline_complete"], false);
        v["schema"] = json!(1);
        std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(read_receipt(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        let cargo = serde_json::to_value(assess(receipt()).unwrap()).unwrap();
        assert!(cargo.get("waits").is_none());
        assert!(cargo.get("control_causal_edges").is_none());
    }

    #[test]
    fn absent_receipts_never_attest_completeness() {
        let dir = crate::paths::home()
            .unwrap()
            .join(format!("graph-empty-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(command(&dir, true, None).is_err());
        std::fs::remove_dir(&dir).unwrap();
    }

    fn join_fixture() -> serde_json::Value {
        json!({"schema":1,"producer":"rustc-thread-pool-join","scope":"native-join-continuations",
            "capture_id":"join-routing","observer":1,"full_pipeline_complete":false,
            "cpu_work_ns":null,"execution_span_ns":null,"scope_coverage":false,
            "scheduler_coverage":false,"pre_attach_coverage":false,"sealed":true,
            "interface_outcome":"returned","task_allocator_baseline":1,
            "task_allocator_high_water":2,"expected_tasks":1,"dropped_events":0,
            "events":[{"kind":"root","task":1,"at_ns":0},
                {"kind":"start","task":1,"at_ns":1},
                {"kind":"finish","task":1,"outcome":"normal","at_ns":2}]})
    }

    #[test]
    fn native_join_receipt_routes_without_query_slice_claims() {
        let path = crate::paths::home()
            .unwrap()
            .join(format!("graph-join-routing-{}.json", std::process::id()));
        std::fs::write(&path, serde_json::to_vec(&join_fixture()).unwrap()).unwrap();
        let a = read_receipt(&path).unwrap();
        assert!(a.structural_complete);
        assert_eq!((a.schema, a.nodes, a.edges), (1, 1, 0));
        assert_eq!(a.producer, "rustc-thread-pool-join");
        assert_eq!(a.scope, "native-join-continuations");
        assert!(!a.full_pipeline_complete);
        assert_eq!((a.cpu_work_ns, a.execution_span_ns), (None, None));
        let output = serde_json::to_value(a).unwrap();
        for field in [
            "descriptive_parent_edges",
            "control_causal_edges",
            "wait_structural_complete",
            "waits",
            "query_reads",
        ] {
            assert!(output.get(field).is_none(), "{field}");
        }
        let mut v = join_fixture();
        v["sealed"] = json!(false);
        std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(!read_receipt(&path).unwrap().structural_complete);
        for (field, value) in [
            ("schema", json!(2)),
            ("full_pipeline_complete", json!(true)),
            ("cpu_work_ns", json!(1)),
            ("unexpected", json!(true)),
        ] {
            let mut v = join_fixture();
            v[field] = value;
            std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
            assert!(read_receipt(&path).is_err(), "{field}");
        }
        let raw = serde_json::to_string(&join_fixture()).unwrap();
        std::fs::write(
            &path,
            raw.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1),
        )
        .unwrap();
        assert!(read_receipt(&path).is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn public_task_binding_file_copies_require_exact_reciprocal_pool_authority() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::paths::home().unwrap());
        let directory = root.join(format!(
            "graph-task-binding-copies-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let query = crate::execution_graph_query::tests::task_binding_fixture();
        let mut pool = join_fixture();
        pool["schema"] = json!(2);
        pool["capture_id"] = json!("pool-1");
        pool["query_capture_id"] = query["capture_id"].clone();
        pool["events"][2]["at_ns"] = json!(20);
        for case in [
            "valid",
            "missing",
            "duplicate",
            "foreign-capture",
            "stale-observer",
            "nonreciprocal",
            "outside-interval",
        ] {
            let case_dir = directory.join(case);
            std::fs::create_dir(&case_dir).unwrap();
            let mut candidate_pool = pool.clone();
            match case {
                "foreign-capture" => candidate_pool["capture_id"] = json!("another-pool"),
                "stale-observer" => candidate_pool["observer"] = json!(2),
                "nonreciprocal" => candidate_pool["query_capture_id"] = json!("another-query"),
                "outside-interval" => candidate_pool["events"][2]["at_ns"] = json!(9),
                _ => {}
            }
            // Names intentionally provide neither capture identity nor a -pool
            // suffix. Query sorts before its independently collected authority.
            std::fs::write(case_dir.join("a.json"), serde_json::to_vec(&query).unwrap()).unwrap();
            if case != "missing" {
                let bytes = serde_json::to_vec(&candidate_pool).unwrap();
                std::fs::write(case_dir.join("z.json"), &bytes).unwrap();
                if case == "duplicate" {
                    std::fs::write(case_dir.join("y.json"), &bytes).unwrap();
                }
            }
            let assessments = collect_path(&case_dir).unwrap();
            let result = assessments
                .iter()
                .find(|a| a.producer == "rustc-query-admission")
                .unwrap();
            assert!(
                result.structural_complete,
                "{case}: admission validity was erased"
            );
            assert_eq!(result.wait_structural_complete, Some(true), "{case}");
            assert!(
                result.query_reads.as_ref().unwrap().records_complete,
                "{case}"
            );
            let bindings = result.task_bindings.as_ref().unwrap();
            assert!(bindings.records_complete, "{case}");
            assert!(bindings.bound_points > 0, "{case}");
            assert_eq!(
                bindings.graph_binding_complete,
                case == "valid",
                "{case}: {bindings:?}"
            );
            assert!(!result.full_pipeline_complete);
            command(&case_dir, true, None).unwrap();
        }
        let duplicate_dir = directory.join("duplicate-query-authority");
        std::fs::create_dir(&duplicate_dir).unwrap();
        for name in ["a.json", "b.json"] {
            std::fs::write(
                duplicate_dir.join(name),
                serde_json::to_vec(&query).unwrap(),
            )
            .unwrap();
        }
        std::fs::write(
            duplicate_dir.join("z.json"),
            serde_json::to_vec(&pool).unwrap(),
        )
        .unwrap();
        let duplicates = collect_path(&duplicate_dir).unwrap();
        for result in duplicates
            .iter()
            .filter(|a| a.producer == "rustc-query-admission")
        {
            assert!(!result.structural_complete);
            assert!(
                !result
                    .task_bindings
                    .as_ref()
                    .unwrap()
                    .graph_binding_complete
            );
            assert!(result.query_reads.as_ref().unwrap().records_complete);
            assert_eq!(result.wait_structural_complete, Some(true));
        }
        command(&duplicate_dir, true, None).unwrap();
        for schema in [1, 2, 3] {
            let case_dir = directory.join(format!("legacy-{schema}"));
            std::fs::create_dir(&case_dir).unwrap();
            let mut legacy = query.clone();
            legacy["schema"] = json!(schema);
            let fields = legacy.as_object_mut().unwrap();
            fields.remove("task_bindings");
            if schema < 3 {
                fields.remove("query_reads");
            }
            if schema < 2 {
                for field in [
                    "wait_allocator_baseline",
                    "wait_allocator_high_water",
                    "expected_waits",
                    "wait_capture_complete",
                    "wait_events",
                ] {
                    fields.remove(field);
                }
            }
            let mut legacy_pool = pool.clone();
            legacy_pool["schema"] = json!(1);
            legacy_pool
                .as_object_mut()
                .unwrap()
                .remove("query_capture_id");
            std::fs::write(
                case_dir.join("a.json"),
                serde_json::to_vec(&legacy).unwrap(),
            )
            .unwrap();
            std::fs::write(
                case_dir.join("z.json"),
                serde_json::to_vec(&legacy_pool).unwrap(),
            )
            .unwrap();
            let assessments = collect_path(&case_dir).unwrap();
            assert_eq!(resource_summary(&assessments)["state"], "unknown");
            let result = assessments
                .iter()
                .find(|a| a.producer == "rustc-query-admission")
                .unwrap();
            assert!(result.structural_complete);
            assert!(
                result.task_bindings.is_none(),
                "legacy lineage is unknown, not zero"
            );
            command(&case_dir, true, None).unwrap();
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
