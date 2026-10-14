//! Independent validation of admitted query jobs, not query requests or a dependency graph.
//! Only synthetic fixtures exercise this module until a rebuilt native compiler is accepted.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_CONTEXTS: usize = 1024;
const MAX_EVENTS: usize = 100_000;
const MAX_JOBS: u64 = 100_000;
const REQUIRED_BOUNDARIES: &[&str] = &[
    "fast-cache-hits",
    "race-cache-hits",
    "green-ensure-no-job",
    "feeds",
    "direct-paths",
    "nonquery",
    "resource-internals",
    "driver-and-process-outcome",
    "cargo-parent-binding",
    "cache-read-dependencies",
    "wait-and-resource-dependencies",
];

// deserialize_with makes nullable fields mandatory rather than silently defaulting missing keys.
fn nullable<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(d)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    producer: String,
    scope: String,
    capture_id: String,
    full_pipeline_complete: bool,
    #[serde(deserialize_with = "nullable")]
    cpu_work_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    execution_span_ns: Option<u64>,
    interface_outcome: InterfaceOutcome,
    capture_complete: bool,
    lost_events: u64,
    opaque_boundaries: Vec<String>,
    expected_contexts: u64,
    contexts: Vec<QueryContext>,
    events: Vec<Event>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptV2 {
    schema: u32,
    producer: String,
    scope: String,
    capture_id: String,
    full_pipeline_complete: bool,
    #[serde(deserialize_with = "nullable")]
    cpu_work_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    execution_span_ns: Option<u64>,
    interface_outcome: InterfaceOutcome,
    capture_complete: bool,
    lost_events: u64,
    opaque_boundaries: Vec<String>,
    expected_contexts: u64,
    contexts: Vec<QueryContext>,
    events: Vec<Event>,
    wait_allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    wait_allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_waits: Option<u64>,
    wait_capture_complete: bool,
    wait_events: Vec<WaitEvent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptV3 {
    schema: u32,
    producer: String,
    scope: String,
    capture_id: String,
    full_pipeline_complete: bool,
    #[serde(deserialize_with = "nullable")]
    cpu_work_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    execution_span_ns: Option<u64>,
    interface_outcome: InterfaceOutcome,
    capture_complete: bool,
    lost_events: u64,
    opaque_boundaries: Vec<String>,
    expected_contexts: u64,
    contexts: Vec<QueryContext>,
    events: Vec<Event>,
    wait_allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    wait_allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_waits: Option<u64>,
    wait_capture_complete: bool,
    wait_events: Vec<WaitEvent>,
    query_reads: crate::execution_graph_reads::ReadCapture,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum WaitReason {
    Completed,
    Poisoned,
    Cycle,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WaitEvent {
    Registered {
        wait_id: u64,
        context_id: u64,
        waitee_job_id: u64,
        #[serde(deserialize_with = "nullable")]
        consumer_job_id: Option<u64>,
        at_ns: u64,
    },
    Notified {
        wait_id: u64,
        context_id: u64,
        waitee_job_id: u64,
        #[serde(deserialize_with = "nullable")]
        consumer_job_id: Option<u64>,
        at_ns: u64,
        reason: WaitReason,
    },
    CondvarReturned {
        wait_id: u64,
        context_id: u64,
        waitee_job_id: u64,
        #[serde(deserialize_with = "nullable")]
        consumer_job_id: Option<u64>,
        at_ns: u64,
    },
    Resumed {
        wait_id: u64,
        context_id: u64,
        waitee_job_id: u64,
        #[serde(deserialize_with = "nullable")]
        consumer_job_id: Option<u64>,
        at_ns: u64,
    },
}

type WaitEndpoints = (u64, u64, Option<u64>);
type WaitParts = (u64, WaitEndpoints, u64, usize, Option<WaitReason>);

impl WaitEvent {
    fn parts(&self) -> WaitParts {
        match *self {
            Self::Registered {
                wait_id,
                context_id,
                waitee_job_id,
                consumer_job_id,
                at_ns,
            } => (
                wait_id,
                (context_id, waitee_job_id, consumer_job_id),
                at_ns,
                0,
                None,
            ),
            Self::Notified {
                wait_id,
                context_id,
                waitee_job_id,
                consumer_job_id,
                at_ns,
                reason,
            } => (
                wait_id,
                (context_id, waitee_job_id, consumer_job_id),
                at_ns,
                1,
                Some(reason),
            ),
            Self::CondvarReturned {
                wait_id,
                context_id,
                waitee_job_id,
                consumer_job_id,
                at_ns,
            } => (
                wait_id,
                (context_id, waitee_job_id, consumer_job_id),
                at_ns,
                2,
                None,
            ),
            Self::Resumed {
                wait_id,
                context_id,
                waitee_job_id,
                consumer_job_id,
                at_ns,
            } => (
                wait_id,
                (context_id, waitee_job_id, consumer_job_id),
                at_ns,
                3,
                None,
            ),
        }
    }
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum InterfaceOutcome {
    Returned,
    Interrupted,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryContext {
    context_id: u64,
    allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_jobs: Option<u64>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Event {
    Admitted {
        context_id: u64,
        job_id: u64,
        #[serde(deserialize_with = "nullable")]
        parent_job_id: Option<u64>,
        at_ns: u64,
    },
    Completed {
        context_id: u64,
        job_id: u64,
        at_ns: u64,
    },
    Poisoned {
        context_id: u64,
        job_id: u64,
        at_ns: u64,
    },
}

#[derive(Debug, Serialize)]
pub struct QueryAssessment {
    pub schema: u32,
    pub producer: String,
    pub scope: String,
    pub capture_id: String,
    pub structural_complete: bool,
    pub full_pipeline_complete: bool,
    /// Unique admitted jobs, namespaced by context.
    pub nodes: usize,
    /// Legacy count of descriptive parents only, never waits or semantic reads.
    pub edges: usize,
    pub descriptive_parent_edges: usize,
    /// Observed notification -> post-token-acquisition resume, never semantic reads.
    pub control_causal_edges: usize,
    /// None for v1: absence of a wait inventory is not evidence of zero waits.
    pub wait_structural_complete: Option<bool>,
    pub waits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) query_reads: Option<crate::execution_graph_reads::ReadAssessment>,
    pub issues: Vec<String>,
    pub opaque_boundaries: Vec<String>,
    pub cpu_work_ns: Option<u64>,
    pub execution_span_ns: Option<u64>,
}

/// Parse the original bytes directly so duplicate fields cannot disappear through a Value.
/// Malformed/unsupported receipts fail parsing. Lifecycle discrepancies return issues.
/// Capture identity uniqueness across receipts must be enforced by the caller.
pub fn assess_bytes(bytes: &[u8]) -> Result<QueryAssessment> {
    if bytes.len() > MAX_BYTES {
        bail!("query receipt exceeds 64 MiB byte budget");
    }
    #[derive(Deserialize)]
    struct Version {
        schema: u32,
    }
    let version: Version =
        serde_json::from_slice(bytes).context("invalid query receipt version")?;
    match version.schema {
        1 => assess_admissions(
            serde_json::from_slice(bytes).context("invalid query admission receipt")?,
        ),
        2 => assess_waits(serde_json::from_slice(bytes).context("invalid query wait receipt")?),
        3 => assess_reads(serde_json::from_slice(bytes).context("invalid query read receipt")?),
        _ => bail!("unsupported query receipt schema"),
    }
}

fn assess_admissions(r: Receipt) -> Result<QueryAssessment> {
    if r.schema != 1
        || r.producer != "rustc-query-admission"
        || r.scope != "query-job-admission-lifecycle"
    {
        bail!("unsupported query receipt schema, producer or scope");
    }
    if r.capture_id.is_empty() || r.capture_id.len() > 128 {
        bail!("query capture identity must contain 1..=128 bytes");
    }
    if r.expected_contexts > MAX_CONTEXTS as u64
        || r.contexts.len() > MAX_CONTEXTS
        || r.events.len() > MAX_EVENTS
        || r.opaque_boundaries.len() > 1024
        || r.opaque_boundaries
            .iter()
            .any(|s| s.is_empty() || s.len() > 128)
    {
        bail!("query receipt exceeds context, event or boundary budget");
    }
    let mut issues = BTreeSet::new();
    if r.full_pipeline_complete {
        issues.insert("query admissions cannot attest full pipeline completeness".into());
    }
    if r.cpu_work_ns.is_some() || r.execution_span_ns.is_some() {
        issues.insert("query admissions cannot attest CPU work or execution span".into());
    }
    for label in REQUIRED_BOUNDARIES {
        if !r.opaque_boundaries.iter().any(|s| s == label) {
            issues.insert(format!("missing required opaque boundary: {label}"));
        }
    }
    if !r.capture_complete {
        issues.insert("capture is incomplete".into());
    }
    if r.interface_outcome != InterfaceOutcome::Returned {
        issues.insert("compiler interface was interrupted".into());
    }
    if r.lost_events != 0 {
        issues.insert("capture lost events".into());
    }
    let mut contexts = BTreeMap::new();
    let mut total_expected = 0u64;
    for c in &r.contexts {
        if contexts.insert(c.context_id, c).is_some() {
            issues.insert(format!("duplicate context {}", c.context_id));
        }
        if c.allocator_baseline != 1 {
            issues.insert(format!("context {} baseline is not 1", c.context_id));
        }
        if c.allocator_high_water
            .is_some_and(|high| high > MAX_JOBS + 1)
            || c.expected_jobs.is_some_and(|expected| expected > MAX_JOBS)
        {
            bail!("allocator counts exceed job budget");
        }
        match (c.allocator_high_water, c.expected_jobs) {
            (Some(high), Some(expected)) => {
                if expected > MAX_JOBS || high > MAX_JOBS + 1 {
                    bail!("allocator counts exceed job budget");
                }
                total_expected = total_expected.saturating_add(expected);
                if high.checked_sub(c.allocator_baseline) != Some(expected) {
                    issues.insert(format!("context {} allocator count mismatch", c.context_id));
                }
            }
            _ => {
                issues.insert(format!(
                    "context {} missing allocator accounting",
                    c.context_id
                ));
            }
        }
    }
    if contexts.len() as u64 != r.expected_contexts {
        issues.insert("independent context allocation count mismatch".into());
    }
    if contexts.keys().copied().ne(1..=r.expected_contexts) {
        issues.insert("context IDs do not exactly cover independent allocator range".into());
    }
    if total_expected > MAX_JOBS {
        bail!("aggregate allocator count exceeds job budget");
    }
    if contexts.is_empty()
        && (!r.events.is_empty() || r.interface_outcome != InterfaceOutcome::Returned)
    {
        issues
            .insert("empty contexts require an explicitly returned interface and no events".into());
    }
    // Index and timestamp order are checked only within each job, never globally.
    let mut admitted = BTreeMap::new();
    let mut terminals = BTreeMap::new();
    for (index, event) in r.events.iter().enumerate() {
        let (context_id, job_id, at_ns) = match event {
            Event::Admitted {
                context_id,
                job_id,
                at_ns,
                ..
            }
            | Event::Completed {
                context_id,
                job_id,
                at_ns,
            }
            | Event::Poisoned {
                context_id,
                job_id,
                at_ns,
            } => (*context_id, *job_id, *at_ns),
        };
        let key = (context_id, job_id);
        match contexts.get(&context_id) {
            None => {
                issues.insert(format!("event references unknown context {context_id}"));
            }
            Some(c) => {
                if job_id < c.allocator_baseline
                    || !matches!(c.allocator_high_water, Some(high) if job_id < high)
                {
                    issues.insert(format!("job {key:?} outside allocator range"));
                }
            }
        }
        match event {
            Event::Admitted { parent_job_id, .. } => match admitted.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((index, at_ns, *parent_job_id));
                }
                std::collections::btree_map::Entry::Occupied(_) => {
                    issues.insert(format!("duplicate admission {key:?}"));
                }
            },
            _ => match terminals.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((index, at_ns));
                }
                std::collections::btree_map::Entry::Occupied(_) => {
                    issues.insert(format!("duplicate terminal {key:?}"));
                }
            },
        }
    }
    let mut edges = 0;
    for (&key, &(index, at_ns, parent)) in &admitted {
        match terminals.get(&key) {
            Some(&(terminal_index, terminal_ns))
                if terminal_index > index && terminal_ns >= at_ns => {}
            Some(_) => {
                issues.insert(format!(
                    "invalid per-job terminal order or timestamp {key:?}"
                ));
            }
            None => {
                issues.insert(format!("missing terminal {key:?}"));
            }
        }
        if let Some(parent) = parent {
            edges += 1;
            // Parent IDs are descriptive. Cross-thread event append order is not global order.
            if parent >= key.1
                || !matches!(admitted.get(&(key.0, parent)), Some(&(_, parent_ns, _)) if parent_ns <= at_ns)
            {
                issues.insert(format!("invalid same-context parent for {key:?}"));
            }
        }
    }
    for key in terminals.keys() {
        if !admitted.contains_key(key) {
            issues.insert(format!("terminal references unadmitted job {key:?}"));
        }
    }
    for c in contexts.values() {
        if let (Some(high), Some(expected)) = (c.allocator_high_water, c.expected_jobs) {
            let count = admitted
                .range((c.context_id, 0)..=(c.context_id, u64::MAX))
                .count() as u64;
            if count != expected {
                issues.insert(format!("context {} admission count mismatch", c.context_id));
            }
            // Range membership plus exact cardinality proves contiguous coverage independently.
            if (c.allocator_baseline..high).any(|job| !admitted.contains_key(&(c.context_id, job)))
            {
                issues.insert(format!(
                    "context {} has missing contiguous admissions",
                    c.context_id
                ));
            }
        }
    }
    Ok(QueryAssessment {
        schema: r.schema,
        producer: r.producer,
        scope: r.scope,
        capture_id: r.capture_id,
        structural_complete: r.capture_complete
            && r.interface_outcome == InterfaceOutcome::Returned
            && r.lost_events == 0
            && issues.is_empty(),
        full_pipeline_complete: false,
        nodes: admitted.len(),
        edges,
        descriptive_parent_edges: edges,
        control_causal_edges: 0,
        wait_structural_complete: None,
        waits: None,
        query_reads: None,
        issues: issues.into_iter().collect(),
        opaque_boundaries: r.opaque_boundaries,
        cpu_work_ns: None,
        execution_span_ns: None,
    })
}

#[derive(Default)]
struct WaitLifecycle {
    endpoints: Option<WaitEndpoints>,
    stages: [Option<(usize, u64)>; 4],
    reason: Option<WaitReason>,
    invalid: bool,
}

fn assess_reads(r: ReceiptV3) -> Result<QueryAssessment> {
    if r.schema != 3 {
        bail!("unsupported query read receipt schema");
    }
    let admitted = r
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Admitted {
                context_id, job_id, ..
            } => Some((*context_id, *job_id)),
            _ => None,
        })
        .collect();
    let expected_contexts = r.expected_contexts;
    let mut assessment = assess_waits(ReceiptV2 {
        schema: 2,
        producer: r.producer,
        scope: r.scope,
        capture_id: r.capture_id,
        full_pipeline_complete: r.full_pipeline_complete,
        cpu_work_ns: r.cpu_work_ns,
        execution_span_ns: r.execution_span_ns,
        interface_outcome: r.interface_outcome,
        capture_complete: r.capture_complete,
        lost_events: r.lost_events,
        opaque_boundaries: r.opaque_boundaries,
        expected_contexts: r.expected_contexts,
        contexts: r.contexts,
        events: r.events,
        wait_allocator_baseline: r.wait_allocator_baseline,
        wait_allocator_high_water: r.wait_allocator_high_water,
        expected_waits: r.expected_waits,
        wait_capture_complete: r.wait_capture_complete,
        wait_events: r.wait_events,
    })?;
    assessment.schema = 3;
    let reads = crate::execution_graph_reads::assess(&r.query_reads, &admitted, expected_contexts)?;
    if !reads.records_complete {
        assessment.structural_complete = false;
        assessment
            .issues
            .push("query read records are incomplete or invalid".into());
    }
    assessment.query_reads = Some(reads);
    Ok(assessment)
}

fn assess_waits(r: ReceiptV2) -> Result<QueryAssessment> {
    if r.schema != 2 {
        bail!("unsupported query wait receipt schema");
    }
    if r.wait_events.len() > MAX_EVENTS
        || r.expected_waits.is_some_and(|n| n > MAX_JOBS)
        || r.wait_allocator_high_water
            .is_some_and(|n| n > MAX_JOBS + 1)
    {
        bail!("query wait receipt exceeds wait budget");
    }
    let mut issues = BTreeSet::new();
    if !r.wait_capture_complete {
        issues.insert("wait capture is incomplete".into());
    }
    if r.wait_allocator_baseline != 1 {
        issues.insert("wait allocator baseline is not 1".into());
    }
    match (r.wait_allocator_high_water, r.expected_waits) {
        (Some(high), Some(expected)) => {
            if high.checked_sub(r.wait_allocator_baseline) != Some(expected) {
                issues.insert("independent wait allocator count mismatch".into());
            }
        }
        _ => {
            issues.insert("missing independent wait allocator accounting".into());
        }
    }
    let admissions: BTreeMap<_, _> = r
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Admitted {
                context_id,
                job_id,
                at_ns,
                ..
            } => Some(((*context_id, *job_id), *at_ns)),
            _ => None,
        })
        .collect();
    let terminals: BTreeMap<_, _> = r
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Completed {
                context_id,
                job_id,
                at_ns,
            } => Some(((*context_id, *job_id), (*at_ns, WaitReason::Completed))),
            Event::Poisoned {
                context_id,
                job_id,
                at_ns,
            } => Some(((*context_id, *job_id), (*at_ns, WaitReason::Poisoned))),
            _ => None,
        })
        .collect();
    let contexts: BTreeSet<_> = r.contexts.iter().map(|c| c.context_id).collect();
    let mut waits: BTreeMap<u64, WaitLifecycle> = BTreeMap::new();
    for (index, event) in r.wait_events.iter().enumerate() {
        let (id, endpoints, at_ns, stage, reason) = event.parts();
        let wait = waits.entry(id).or_default();
        if id < r.wait_allocator_baseline
            || !r.wait_allocator_high_water.is_some_and(|high| id < high)
        {
            issues.insert(format!("wait {id} outside allocator range"));
            wait.invalid = true;
        }
        if wait.endpoints.is_some_and(|previous| previous != endpoints) {
            issues.insert(format!("wait {id} has inconsistent endpoints"));
            wait.invalid = true;
        } else {
            wait.endpoints = Some(endpoints);
        }
        let (context, waitee, consumer) = endpoints;
        if !contexts.contains(&context)
            || !admissions.contains_key(&(context, waitee))
            || consumer.is_some_and(|job| !admissions.contains_key(&(context, job)))
        {
            issues.insert(format!("wait {id} has foreign or unadmitted endpoint"));
            wait.invalid = true;
        }
        if wait.stages[stage].is_some() {
            issues.insert(format!("wait {id} has duplicate lifecycle stage {stage}"));
            wait.invalid = true;
        } else {
            wait.stages[stage] = Some((index, at_ns));
        }
        if reason.is_some() {
            wait.reason = reason;
        }
    }
    if let (Some(high), Some(expected)) = (r.wait_allocator_high_water, r.expected_waits) {
        if waits.len() as u64 != expected {
            issues.insert("independent wait inventory count mismatch".into());
        }
        if (r.wait_allocator_baseline..high).any(|id| !waits.contains_key(&id)) {
            issues.insert("missing contiguous wait lifecycle records".into());
        }
    }
    let mut control_causal_edges = 0;
    for (&id, wait) in &mut waits {
        if wait.stages.iter().any(Option::is_none) {
            issues.insert(format!("wait {id} has missing lifecycle records"));
            wait.invalid = true;
        }
        for pair in wait.stages.windows(2) {
            if let [Some((left_index, left_ns)), Some((right_index, right_ns))] = pair
                && (left_index >= right_index || left_ns > right_ns)
            {
                issues.insert(format!(
                    "wait {id} has invalid lifecycle order or timestamp"
                ));
                wait.invalid = true;
            }
        }
        if let (Some((context, waitee, consumer)), Some((_, registered_ns))) =
            (wait.endpoints, wait.stages[0])
        {
            if admissions
                .get(&(context, waitee))
                .is_some_and(|&ns| ns > registered_ns)
                || consumer.is_some_and(|job| {
                    admissions
                        .get(&(context, job))
                        .is_some_and(|&ns| ns > registered_ns)
                })
            {
                issues.insert(format!(
                    "wait {id} registration precedes endpoint admission"
                ));
                wait.invalid = true;
            }
            if let Some(reason @ (WaitReason::Completed | WaitReason::Poisoned)) = wait.reason
                && !terminals
                    .get(&(context, waitee))
                    .is_some_and(|&(_, terminal_reason)| terminal_reason == reason)
            {
                issues.insert(format!(
                    "wait {id} notification reason disagrees with waitee terminal"
                ));
                wait.invalid = true;
            }
        }
        if !wait.invalid {
            control_causal_edges += 1;
        }
    }
    let mut assessment = assess_admissions(Receipt {
        schema: 1,
        producer: r.producer,
        scope: r.scope,
        capture_id: r.capture_id,
        full_pipeline_complete: r.full_pipeline_complete,
        cpu_work_ns: r.cpu_work_ns,
        execution_span_ns: r.execution_span_ns,
        interface_outcome: r.interface_outcome,
        capture_complete: r.capture_complete,
        lost_events: r.lost_events,
        opaque_boundaries: r.opaque_boundaries,
        expected_contexts: r.expected_contexts,
        contexts: r.contexts,
        events: r.events,
    })?;
    assessment.schema = 2;
    assessment.wait_structural_complete = Some(assessment.structural_complete && issues.is_empty());
    assessment.structural_complete &= issues.is_empty();
    assessment.waits = Some(waits.len());
    assessment.control_causal_edges = control_causal_edges;
    assessment.issues.extend(issues);
    assessment.issues.sort();
    assessment.issues.dedup();
    Ok(assessment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture() -> Value {
        json!({"schema":1,"producer":"rustc-query-admission","scope":"query-job-admission-lifecycle",
            "capture_id":"synthetic-1","full_pipeline_complete":false,"cpu_work_ns":null,
            "execution_span_ns":null,"interface_outcome":"returned","capture_complete":true,
            "lost_events":0,"expected_contexts":1,"opaque_boundaries":REQUIRED_BOUNDARIES,
            "contexts":[{"context_id":1,"allocator_baseline":1,"allocator_high_water":3,"expected_jobs":2}],
            "events":[{"kind":"admitted","context_id":1,"job_id":1,"parent_job_id":null,"at_ns":1},
                {"kind":"admitted","context_id":1,"job_id":2,"parent_job_id":1,"at_ns":10},
                {"kind":"completed","context_id":1,"job_id":2,"at_ns":11},
                {"kind":"poisoned","context_id":1,"job_id":1,"at_ns":2}]})
    }
    fn assess(v: &Value) -> QueryAssessment {
        assess_bytes(&serde_json::to_vec(v).unwrap()).unwrap()
    }
    fn incomplete(v: &Value) {
        let a = assess(v);
        assert!(!a.structural_complete, "{a:?}");
        assert!(!a.issues.is_empty());
    }

    #[test]
    fn nonmonotonic_cross_thread_time_is_valid() {
        let a = assess(&fixture());
        assert!(a.structural_complete, "{a:?}");
        assert_eq!((a.nodes, a.edges), (2, 1));
        assert!(!a.full_pipeline_complete);
        assert_eq!((a.cpu_work_ns, a.execution_span_ns), (None, None));
    }
    #[test]
    fn missing_duplicate_and_unknown_lifecycles() {
        let mut v = fixture();
        v["events"].as_array_mut().unwrap().pop();
        incomplete(&v);
        let mut v = fixture();
        let duplicate = v["events"][0].clone();
        v["events"].as_array_mut().unwrap().push(duplicate);
        incomplete(&v);
        let mut v = fixture();
        let duplicate = v["events"][2].clone();
        v["events"].as_array_mut().unwrap().push(duplicate);
        incomplete(&v);
        let mut v = fixture();
        v["events"][3]["job_id"] = json!(3);
        incomplete(&v);
        let mut v = fixture();
        v["events"][0]["kind"] = json!("requested");
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn context_namespaces_allow_id_reuse_but_not_duplicate_contexts() {
        let mut v = fixture();
        v["expected_contexts"] = json!(2);
        let mut c = v["contexts"][0].clone();
        c["context_id"] = json!(2);
        v["contexts"].as_array_mut().unwrap().push(c);
        let extra: Vec<_> = v["events"]
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .map(|mut e| {
                e["context_id"] = json!(2);
                e
            })
            .collect();
        v["events"].as_array_mut().unwrap().extend(extra);
        assert!(assess(&v).structural_complete);
        v["contexts"][1]["context_id"] = json!(1);
        incomplete(&v);
        let mut v = fixture();
        v["events"][0]["context_id"] = json!(99);
        incomplete(&v);
    }
    #[test]
    fn allocator_and_parent_mismatches() {
        for (field, value) in [
            ("allocator_baseline", json!(0)),
            ("expected_jobs", json!(1)),
            ("allocator_high_water", json!(4)),
            ("allocator_high_water", Value::Null),
        ] {
            let mut v = fixture();
            v["contexts"][0][field] = value;
            incomplete(&v);
        }
        for parent in [0, 2, 9] {
            let mut v = fixture();
            v["events"][1]["parent_job_id"] = json!(parent);
            incomplete(&v);
        }
        let mut v = fixture();
        v["events"].as_array_mut().unwrap().swap(0, 1);
        assert!(assess(&v).structural_complete);
    }
    #[test]
    fn per_job_order_and_time_are_required() {
        let mut v = fixture();
        v["events"][2]["at_ns"] = json!(1);
        incomplete(&v);
        let mut v = fixture();
        v["events"].as_array_mut().unwrap().swap(1, 2);
        incomplete(&v);
    }
    #[test]
    fn incomplete_capture_and_false_claims_never_pass() {
        for (field, value) in [
            ("interface_outcome", json!("interrupted")),
            ("capture_complete", json!(false)),
            ("lost_events", json!(1)),
            ("full_pipeline_complete", json!(true)),
            ("cpu_work_ns", json!(1)),
            ("execution_span_ns", json!(1)),
        ] {
            let mut v = fixture();
            v[field] = value;
            incomplete(&v);
        }
    }
    #[test]
    fn opaque_privacy_scopes_are_required_individually() {
        for index in 0..REQUIRED_BOUNDARIES.len() {
            let mut v = fixture();
            v["opaque_boundaries"].as_array_mut().unwrap().remove(index);
            incomplete(&v);
        }
        let mut v = fixture();
        v["query_key"] = json!("private key");
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn typed_parse_rejects_duplicates_unknown_fields_and_missing_nulls() {
        let bytes = serde_json::to_string(&fixture()).unwrap();
        let duplicate = bytes.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1);
        assert!(assess_bytes(duplicate.as_bytes()).is_err());
        for field in ["cpu_work_ns", "execution_span_ns"] {
            let mut v = fixture();
            v.as_object_mut().unwrap().remove(field);
            assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        let mut v = fixture();
        v["events"][2]["parent_job_id"] = Value::Null;
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = fixture();
        v["events"][0]
            .as_object_mut()
            .unwrap()
            .remove("parent_job_id");
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        for field in ["schema", "producer", "scope"] {
            let mut v = fixture();
            v[field] = if field == "schema" {
                json!(2)
            } else {
                json!("unknown")
            };
            assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        }
    }
    #[test]
    fn empty_returned_capture_is_not_full_work() {
        let mut v = fixture();
        v["expected_contexts"] = json!(0);
        v["contexts"] = json!([]);
        v["events"] = json!([]);
        let a = assess(&v);
        assert!(a.structural_complete);
        assert_eq!(a.nodes, 0);
        assert!(!a.full_pipeline_complete);
        v["interface_outcome"] = json!("interrupted");
        incomplete(&v);
    }
    #[test]
    fn independent_inventory_detects_whole_context_deletion() {
        let mut v = fixture();
        v["contexts"] = json!([]);
        v["events"] = json!([]);
        incomplete(&v);

        // Delete a trailing context and every one of its events. Remaining jobs
        // are perfectly accounted for, but the independent root allocator is not.
        let mut v = fixture();
        v["expected_contexts"] = json!(2);
        incomplete(&v);

        // Delete the first context instead, leaving the same number of valid jobs.
        v["contexts"][0]["context_id"] = json!(2);
        for e in v["events"].as_array_mut().unwrap() {
            e["context_id"] = json!(2);
        }
        incomplete(&v);
    }
    #[test]
    fn context_ids_must_exactly_cover_independent_inventory() {
        for id in [0, 2, u64::MAX] {
            let mut v = fixture();
            v["contexts"][0]["context_id"] = json!(id);
            for e in v["events"].as_array_mut().unwrap() {
                e["context_id"] = json!(id);
            }
            incomplete(&v);
        }
        let mut v = fixture();
        v["expected_contexts"] = json!(0);
        incomplete(&v);
        let mut v = fixture();
        let duplicate = v["contexts"][0].clone();
        v["contexts"].as_array_mut().unwrap().push(duplicate);
        incomplete(&v);
    }
    #[test]
    fn independent_inventory_field_is_required_and_bounded() {
        let mut v = fixture();
        v.as_object_mut().unwrap().remove("expected_contexts");
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        for value in [Value::Null, json!("1"), json!(1025), json!(u64::MAX)] {
            let mut v = fixture();
            v["expected_contexts"] = value;
            assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        let raw = serde_json::to_string(&fixture()).unwrap();
        let duplicate = raw.replacen(
            "\"expected_contexts\":1",
            "\"expected_contexts\":1,\"expected_contexts\":1",
            1,
        );
        assert!(assess_bytes(duplicate.as_bytes()).is_err());
    }
    #[test]
    fn parent_admission_time_cannot_follow_child() {
        let mut v = fixture();
        v["events"][0]["at_ns"] = json!(11);
        v["events"][3]["at_ns"] = json!(12);
        incomplete(&v);
        let mut v = fixture();
        v["events"][1]["at_ns"] = json!(1);
        assert!(assess(&v).structural_complete);
    }
    #[test]
    fn budgets_reject_unreasonable_input() {
        let mut v = fixture();
        v["capture_id"] = json!("x".repeat(129));
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = fixture();
        v["contexts"][0]["expected_jobs"] = json!(u64::MAX);
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = fixture();
        v["opaque_boundaries"][0] = json!("x".repeat(129));
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        assert!(assess_bytes(&vec![b' '; MAX_BYTES + 1]).is_err());
    }

    fn wait_fixture() -> Value {
        let mut v = fixture();
        v["schema"] = json!(2);
        v["wait_allocator_baseline"] = json!(1);
        v["wait_allocator_high_water"] = json!(2);
        v["expected_waits"] = json!(1);
        v["wait_capture_complete"] = json!(true);
        v["events"][3]["at_ns"] = json!(30);
        v["wait_events"] = json!([
            {"kind":"registered","wait_id":1,"context_id":1,"waitee_job_id":2,"consumer_job_id":1,"at_ns":10},
            {"kind":"notified","wait_id":1,"context_id":1,"waitee_job_id":2,"consumer_job_id":1,"at_ns":11,"reason":"completed"},
            {"kind":"condvar_returned","wait_id":1,"context_id":1,"waitee_job_id":2,"consumer_job_id":1,"at_ns":12},
            {"kind":"resumed","wait_id":1,"context_id":1,"waitee_job_id":2,"consumer_job_id":1,"at_ns":13}
        ]);
        v
    }

    fn malformed(v: &Value) {
        assert!(
            assess_bytes(&serde_json::to_vec(v).unwrap()).is_err(),
            "{v}"
        );
    }

    #[test]
    fn v2_wait_control_is_separate_from_descriptive_admission() {
        let a = assess(&wait_fixture());
        assert!(a.structural_complete, "{a:?}");
        assert_eq!(a.schema, 2);
        assert_eq!(a.wait_structural_complete, Some(true));
        assert_eq!(a.waits, Some(1));
        assert_eq!(
            (a.edges, a.descriptive_parent_edges, a.control_causal_edges),
            (1, 1, 1)
        );
        assert!(!a.full_pipeline_complete);
        assert_eq!((a.cpu_work_ns, a.execution_span_ns), (None, None));
        let legacy = assess(&fixture());
        assert_eq!(
            (legacy.waits, legacy.wait_structural_complete),
            (None, None)
        );
        assert_eq!(legacy.control_causal_edges, 0);
    }

    #[test]
    fn v1_v2_fields_are_strict_and_not_optional_extensions() {
        let mut v = wait_fixture();
        v["schema"] = json!(1);
        malformed(&v);
        for field in [
            "wait_allocator_baseline",
            "wait_allocator_high_water",
            "expected_waits",
            "wait_capture_complete",
            "wait_events",
        ] {
            let mut v = wait_fixture();
            v.as_object_mut().unwrap().remove(field);
            malformed(&v);
        }
        let mut v = wait_fixture();
        v["query_reads"] = json!({"semantic_complete":true});
        malformed(&v);
    }

    #[test]
    fn independent_wait_inventory_detects_whole_deletion_and_gaps() {
        let mut v = wait_fixture();
        v["wait_events"] = json!([]);
        incomplete(&v);
        for id in [0, 2, u64::MAX] {
            let mut v = wait_fixture();
            for e in v["wait_events"].as_array_mut().unwrap() {
                e["wait_id"] = json!(id);
            }
            incomplete(&v);
            assert_eq!(assess(&v).control_causal_edges, 0);
        }
        let mut v = wait_fixture();
        v["wait_allocator_high_water"] = json!(3);
        v["expected_waits"] = json!(2);
        incomplete(&v);
    }

    #[test]
    fn every_wait_stage_is_required_once() {
        for index in 0..4 {
            let mut v = wait_fixture();
            v["wait_events"].as_array_mut().unwrap().remove(index);
            incomplete(&v);
            assert_eq!(assess(&v).control_causal_edges, 0);
            let mut v = wait_fixture();
            let duplicate = v["wait_events"][index].clone();
            v["wait_events"].as_array_mut().unwrap().push(duplicate);
            incomplete(&v);
            assert_eq!(assess(&v).control_causal_edges, 0);
        }
    }

    #[test]
    fn wait_stage_order_and_timestamps_are_per_token() {
        for index in 0..3 {
            let mut v = wait_fixture();
            v["wait_events"]
                .as_array_mut()
                .unwrap()
                .swap(index, index + 1);
            incomplete(&v);
            let mut v = wait_fixture();
            v["wait_events"][index + 1]["at_ns"] = json!(0);
            incomplete(&v);
        }
        let mut v = wait_fixture();
        for e in v["wait_events"].as_array_mut().unwrap() {
            e["at_ns"] = json!(10);
        }
        assert!(assess(&v).structural_complete);
    }

    #[test]
    fn wait_ids_are_global_and_interleaving_is_not_global_chronology() {
        let mut v = wait_fixture();
        v["wait_allocator_high_water"] = json!(3);
        v["expected_waits"] = json!(2);
        let extra: Vec<_> = v["wait_events"]
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .map(|mut e| {
                e["wait_id"] = json!(2);
                e["consumer_job_id"] = Value::Null;
                e["at_ns"] = json!(10);
                e
            })
            .collect();
        v["wait_events"].as_array_mut().unwrap().extend(extra);
        let a = assess(&v);
        assert!(a.structural_complete, "{a:?}");
        assert_eq!((a.waits, a.control_causal_edges), (Some(2), 2));
    }

    #[test]
    fn wait_endpoints_must_exist_and_remain_identical() {
        for field in ["context_id", "waitee_job_id", "consumer_job_id"] {
            let mut v = wait_fixture();
            for e in v["wait_events"].as_array_mut().unwrap() {
                e[field] = json!(99);
            }
            incomplete(&v);
            assert_eq!(assess(&v).control_causal_edges, 0);
            let mut v = wait_fixture();
            v["wait_events"][2][field] = json!(1);
            if field == "context_id" {
                v["wait_events"][2][field] = json!(2);
            }
            if field == "consumer_job_id" {
                v["wait_events"][2][field] = Value::Null;
            }
            incomplete(&v);
        }
        let mut v = wait_fixture();
        for e in v["wait_events"].as_array_mut().unwrap() {
            e["consumer_job_id"] = Value::Null;
        }
        assert!(assess(&v).structural_complete);
        let mut v = wait_fixture();
        v["wait_events"][0]["at_ns"] = json!(9);
        incomplete(&v);
    }

    #[test]
    fn notification_reasons_are_closed_and_terminal_kinds_must_align() {
        for reason in ["completed", "poisoned", "cycle"] {
            let mut v = wait_fixture();
            v["wait_events"][1]["reason"] = json!(reason);
            if reason == "poisoned" {
                v["events"][2]["kind"] = json!("poisoned");
            }
            assert!(assess(&v).structural_complete, "{v}");
        }
        let mut v = wait_fixture();
        v["wait_events"][1]["reason"] = json!("poisoned");
        incomplete(&v);
        for reason in [Value::Null, json!("unknown"), json!(1)] {
            let mut v = wait_fixture();
            v["wait_events"][1]["reason"] = reason;
            malformed(&v);
        }
        // Terminal publication may follow resume, so no cross-vector append order is imposed.
        let mut v = wait_fixture();
        v["events"][2]["at_ns"] = json!(50);
        assert!(assess(&v).structural_complete);
    }

    #[test]
    fn wait_loss_accounting_and_unjustified_measurements_never_pass() {
        for (field, value) in [
            ("wait_capture_complete", json!(false)),
            ("lost_events", json!(1)),
            ("capture_complete", json!(false)),
            ("interface_outcome", json!("interrupted")),
            ("wait_allocator_baseline", json!(0)),
            ("wait_allocator_high_water", Value::Null),
            ("expected_waits", Value::Null),
            ("expected_waits", json!(0)),
            ("wait_allocator_high_water", json!(0)),
            ("full_pipeline_complete", json!(true)),
            ("cpu_work_ns", json!(1)),
            ("execution_span_ns", json!(1)),
        ] {
            let mut v = wait_fixture();
            v[field] = value;
            incomplete(&v);
            let a = assess(&v);
            assert_eq!(a.wait_structural_complete, Some(false));
            assert!(!a.full_pipeline_complete);
            assert_eq!((a.cpu_work_ns, a.execution_span_ns), (None, None));
        }
    }

    #[test]
    fn strict_wait_records_reject_unknown_missing_and_duplicate_fields() {
        for stage in 0..4 {
            for field in [
                "wait_id",
                "context_id",
                "waitee_job_id",
                "consumer_job_id",
                "at_ns",
                "kind",
            ] {
                let mut v = wait_fixture();
                v["wait_events"][stage]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                malformed(&v);
            }
            let mut v = wait_fixture();
            v["wait_events"][stage]["raw_pointer"] = json!(42);
            malformed(&v);
            if stage != 1 {
                let mut v = wait_fixture();
                v["wait_events"][stage]["reason"] = Value::Null;
                malformed(&v);
            }
        }
        let mut v = wait_fixture();
        v["wait_events"][1]
            .as_object_mut()
            .unwrap()
            .remove("reason");
        malformed(&v);
        let raw = serde_json::to_string(&wait_fixture()).unwrap();
        for (needle, replacement) in [
            ("\"schema\":2", "\"schema\":2,\"schema\":2"),
            (
                "\"expected_waits\":1",
                "\"expected_waits\":1,\"expected_waits\":1",
            ),
            ("\"wait_id\":1", "\"wait_id\":1,\"wait_id\":1"),
            (
                "\"kind\":\"registered\"",
                "\"kind\":\"registered\",\"kind\":\"registered\"",
            ),
            (
                "\"consumer_job_id\":1",
                "\"consumer_job_id\":1,\"consumer_job_id\":1",
            ),
            (
                "\"reason\":\"completed\"",
                "\"reason\":\"completed\",\"reason\":\"completed\"",
            ),
        ] {
            assert!(assess_bytes(raw.replacen(needle, replacement, 1).as_bytes()).is_err());
        }
    }

    #[test]
    fn v2_empty_inventory_is_explicit_zero_not_missing() {
        let mut v = wait_fixture();
        v["wait_events"] = json!([]);
        v["wait_allocator_high_water"] = json!(1);
        v["expected_waits"] = json!(0);
        let a = assess(&v);
        assert!(a.structural_complete);
        assert_eq!(
            (a.waits, a.wait_structural_complete, a.control_causal_edges),
            (Some(0), Some(true), 0)
        );
    }

    #[test]
    fn wait_count_budgets_and_types_fail_parsing() {
        for field in ["expected_waits", "wait_allocator_high_water"] {
            for value in [json!(u64::MAX), json!("1"), json!(-1)] {
                let mut v = wait_fixture();
                v[field] = value;
                malformed(&v);
            }
        }
        let mut v = wait_fixture();
        let e = v["wait_events"][0].clone();
        v["wait_events"] = Value::Array(vec![e; MAX_EVENTS + 1]);
        malformed(&v);
    }

    fn read_fixture() -> Value {
        let mut v = wait_fixture();
        v["schema"] = json!(3);
        v["query_reads"] = json!({
            "schema":1,"records_complete":true,"semantic_complete":false,
            "full_pipeline_complete":false,"dep_node_namespace":"current-context",
            "request_high_water":1,"read_high_water":1,"result_high_water":1,
            "expected_requests":0,"expected_reads":0,"expected_results":0,"lost_events":0,
            "opaque_boundaries":["execution-request-read-binding","previous-session-promotion",
                "feed-producer-provenance","missing-consumer-context","no-implicit-context-reads","nonquery"],
            "events":[]
        });
        v
    }

    #[test]
    fn schema3_requires_typed_reads_and_preserves_exact_older_versions() {
        let a = assess(&read_fixture());
        assert!(a.structural_complete, "{a:?}");
        assert_eq!(a.schema, 3);
        assert_eq!(a.wait_structural_complete, Some(true));
        let output = serde_json::to_value(a).unwrap();
        assert_eq!(output["query_reads"]["records_complete"], true);
        assert_eq!(output["query_reads"]["semantic_complete"], false);
        let mut v = read_fixture();
        v.as_object_mut().unwrap().remove("query_reads");
        malformed(&v);
        for schema in [1, 2] {
            let mut v = read_fixture();
            v["schema"] = json!(schema);
            malformed(&v);
        }
        let mut v = read_fixture();
        v["query_reads"]["unvalidated_extra"] = json!({"semantic_complete":true});
        malformed(&v);
        let raw = serde_json::to_string(&read_fixture()).unwrap();
        let duplicated = raw.replacen(
            "\"records_complete\":true",
            "\"records_complete\":true,\"records_complete\":true",
            1,
        );
        assert!(assess_bytes(duplicated.as_bytes()).is_err());
    }

    #[test]
    fn read_capture_loss_does_not_erase_independent_wait_assessment() {
        for (field, value) in [
            ("records_complete", json!(false)),
            ("lost_events", json!(1)),
        ] {
            let mut v = read_fixture();
            v["query_reads"][field] = value;
            v["query_reads"]["records_complete"] = json!(false);
            let a = assess(&v);
            assert!(!a.structural_complete, "{a:?}");
            assert_eq!(a.wait_structural_complete, Some(true));
            assert_eq!(a.control_causal_edges, 1);
            assert!(!a.full_pipeline_complete);
            assert_eq!((a.cpu_work_ns, a.execution_span_ns), (None, None));
            assert!(!a.query_reads.unwrap().records_complete);
        }
        let mut v = read_fixture();
        v["query_reads"]["semantic_complete"] = json!(true);
        malformed(&v);
    }
}
