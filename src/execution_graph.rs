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
        issues,
        opaque_boundaries: r.opaque_boundaries,
        cpu_work_ns: None,
        execution_span_ns: None,
    })
}

fn read_receipt(path: &Path) -> Result<Assessment> {
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
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES {
        bail!("receipt is not a regular file within the byte budget");
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("receipt exceeds byte budget");
    }
    assess(serde_json::from_slice(&bytes).context("invalid execution graph receipt")?)
}

pub fn command(path: &Path, json: bool) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("capture path must not be a symlink");
    }
    let mut files = if metadata.is_dir() {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(path)? {
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
    for file in files {
        total_bytes = total_bytes.saturating_add(std::fs::symlink_metadata(&file)?.len());
        if total_bytes > MAX_BYTES {
            bail!("capture directory exceeds total receipt byte budget");
        }
        let mut assessment = read_receipt(&file)?;
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
    fn absent_receipts_never_attest_completeness() {
        let dir = crate::paths::home()
            .unwrap()
            .join(format!("graph-empty-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(command(&dir, true).is_err());
        std::fs::remove_dir(&dir).unwrap();
    }
}
