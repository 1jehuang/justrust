//! Independent validation of native execution graph receipts.
//! A structurally valid Cargo-unit receipt is not a complete compiler graph.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;
use std::path::Path;

const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 256;
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
    task_bindings: Option<crate::execution_graph_task_bindings::BindingAssessment>,
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
        task_bindings: None,
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
        let pool = crate::execution_graph_continuation::collect_bytes(bytes)?;
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
    let mut query = crate::execution_graph_query::collect_bytes(bytes)?;
    if let Some(bindings) = &query.bindings {
        query.assessment.task_bindings =
            Some(crate::execution_graph_task_bindings::reconcile_bindings(
                &query.assessment.capture_id,
                bindings,
                pools,
            ));
    }
    Ok(query.assessment)
}

fn assess_receipt_bytes(
    bytes: &[u8],
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
) -> Result<Assessment> {
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
                task_bindings: None,
                issues: vec!["requires explicit --compiler-manifest reconciliation".into()],
                opaque_boundaries: vec!["compiler-launch-authority".into()],
                cpu_work_ns: None,
                execution_span_ns: None,
            })
        }
        "rustc-query-admission" => {
            let query = collect_query(bytes, pools)?;
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
                task_bindings: query.task_bindings,
                issues: query.issues,
                opaque_boundaries: query.opaque_boundaries,
                cpu_work_ns: query.cpu_work_ns,
                execution_span_ns: query.execution_span_ns,
            })
        }
        "rustc-thread-pool-join" => {
            let join = crate::execution_graph_continuation::assess_bytes(bytes)?;
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
                task_bindings: None,
                issues: join.issues,
                opaque_boundaries: join.opaque_boundaries,
                cpu_work_ns: join.cpu_work_ns,
                execution_span_ns: join.execution_span_ns,
            })
        }
        _ => bail!("unsupported execution graph producer"),
    }
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

pub fn command(path: &Path, json: bool, manifest: Option<&Path>) -> Result<()> {
    if let Some(manifest) = manifest {
        return command_manifest(path, manifest, json);
    }
    let assessments = collect_path(path)?;
    render_assessments(assessments, json)
}

fn collect_path(path: &Path) -> Result<Vec<Assessment>> {
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
            if path.extension().is_some_and(|s| s == "json")
                && !is_owner_document(&path)
                && !path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.ends_with(".envelope.json"))
            {
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
        total_bytes = total_bytes.saturating_add(std::fs::symlink_metadata(&file)?.len());
        if total_bytes > MAX_BYTES {
            bail!("capture directory exceeds total receipt byte budget");
        }
        documents.push(read_bytes(&file, MAX_BYTES)?);
    }
    let pools = collect_pool_inventories(&documents)?;
    for bytes in &documents {
        let mut assessment = assess_receipt_bytes(bytes, &pools)?;
        if let Some(&previous) = identities.get(&assessment.capture_id) {
            let previous: &mut Assessment = &mut assessments[previous];
            previous.structural_complete = false;
            previous.issues.push("duplicate capture identity".into());
            assessment.structural_complete = false;
            assessment.issues.push("duplicate capture identity".into());
        } else {
            identities.insert(assessment.capture_id.clone(), assessments.len());
        }
        assessments.push(assessment);
    }
    Ok(assessments)
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
            if let Some(bindings) = &a.task_bindings {
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
