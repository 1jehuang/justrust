//! Independent validation of admitted query jobs, not query requests or a dependency graph.
//! Native admission/read acceptance is tracked by the query-read benchmark provenance.
//! Schema4 task lineage additionally requires reciprocal continuation-pool authority.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 64 * 1024 * 1024;
/// Explicit opt-in policy. Schema and all non-read inventories remain unchanged.
#[derive(Clone, Copy)]
pub(crate) struct CapturePolicy {
    pub(crate) max_bytes: usize,
    pub(crate) max_points: usize,
}

impl CapturePolicy {
    pub(crate) const fn strict() -> Self {
        Self {
            max_bytes: MAX_BYTES,
            max_points: 100_000,
        }
    }

    pub(crate) const fn large() -> Self {
        Self {
            max_bytes: 512 * 1024 * 1024,
            max_points: 1_000_000,
        }
    }
}

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptV4 {
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
    task_bindings: crate::execution_graph_task_bindings::BindingCapture,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptV5 {
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
    task_bindings: crate::execution_graph_task_bindings::BindingCapture,
    #[serde(deserialize_with = "nullable_clock")]
    execution_clock: Option<crate::execution_graph_timing::ClockBridge>,
}

fn nullable_clock<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<crate::execution_graph_timing::ClockBridge>, D::Error> {
    Option::deserialize(d)
}

impl ReceiptV5 {
    /// Typed conversion only, after the original-byte v5 parser rejects unknown
    /// and duplicate fields. No serialization can erase malformed input.
    fn into_base(
        self,
    ) -> (
        ReceiptV4,
        Option<crate::execution_graph_timing::ClockBridge>,
    ) {
        let clock = self.execution_clock;
        let base = ReceiptV4 {
            schema: 4,
            producer: self.producer,
            scope: self.scope,
            capture_id: self.capture_id,
            full_pipeline_complete: self.full_pipeline_complete,
            cpu_work_ns: self.cpu_work_ns,
            execution_span_ns: self.execution_span_ns,
            interface_outcome: self.interface_outcome,
            capture_complete: self.capture_complete,
            lost_events: self.lost_events,
            opaque_boundaries: self.opaque_boundaries,
            expected_contexts: self.expected_contexts,
            contexts: self.contexts,
            events: self.events,
            wait_allocator_baseline: self.wait_allocator_baseline,
            wait_allocator_high_water: self.wait_allocator_high_water,
            expected_waits: self.expected_waits,
            wait_capture_complete: self.wait_capture_complete,
            wait_events: self.wait_events,
            query_reads: self.query_reads,
            task_bindings: self.task_bindings,
        };
        (base, clock)
    }
}

/// Typed captures are retained until the caller's second, reciprocal-authority pass.
pub struct CollectedQuery {
    pub assessment: QueryAssessment,
    pub bindings: Option<crate::execution_graph_task_bindings::ValidatedBindingCapture>,
    pub(crate) wait_inventory: Option<ValidatedQueryWaitInventory>,
    execution_clock: Option<crate::execution_graph_timing::ClockBridge>,
    clock_binding: Option<ClockBinding>,
}

/// Clock authority only. Valid alignment does not attest CPU coverage, query
/// semantic completeness, full-pipeline work, or an execution span.
#[derive(Clone, Debug, Serialize)]
pub struct ClockAlignmentAssessment {
    pub aligned: bool,
    pub issues: Vec<String>,
}

#[derive(Deserialize)]
struct ClockBinding {
    capture_id: String,
    observer: u64,
}

impl CollectedQuery {
    /// Run after the independent task-binding reconciliation. Only a reciprocal
    /// validated pool3 descriptor can authorize crossing query/pool clocks.
    pub(crate) fn reconcile_clock_alignment(
        &mut self,
        pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
    ) {
        if self.assessment.schema != 5 {
            return;
        }
        let mut issues = Vec::new();
        match (&self.execution_clock, &self.clock_binding) {
            (None, _) => issues.push("query execution clock unavailable".into()),
            (Some(_), None) => issues.push("query clock has no task-binding pool identity".into()),
            (Some(clock), Some(binding)) => {
                if clock.query_capture_id != self.assessment.capture_id
                    || clock.pool_capture_id == self.assessment.capture_id
                    || clock.pool_capture_id != binding.capture_id
                    || clock.pool_observer != binding.observer
                    || clock.pool_observer == 0
                    || clock.pool_capture_id.is_empty()
                    || clock.pool_capture_id.len() > 128
                    || clock.pool_epoch_in_query_ns != 0
                {
                    issues.push("query execution clock identity or origin mismatch".into());
                }
                match pools.get(&clock.pool_capture_id) {
                    Some(pool)
                        if pool.schema == 3
                            && pool.structural_complete
                            && pool.capture_id == clock.pool_capture_id
                            && pool.observer == clock.pool_observer
                            && pool.query_capture_id.as_deref()
                                == Some(self.assessment.capture_id.as_str())
                            && pool.timing_bridge.as_ref() == Some(clock) => {}
                    _ => issues
                        .push("reciprocal pool execution clock unavailable or mismatched".into()),
                }
                if !self
                    .assessment
                    .task_bindings
                    .as_ref()
                    .is_some_and(|binding| binding.graph_binding_valid)
                {
                    issues.push("query clock task-binding authority is not valid".into());
                }
            }
        }
        self.assessment.clock_alignment = Some(ClockAlignmentAssessment {
            aligned: issues.is_empty(),
            issues,
        });
    }
}

/// Schema4/5 facts retained from the original strict receipt. Cross-clock
/// authorization, when present in schema5, is reconciled independently.
pub(crate) struct ValidatedQueryWaitInventory {
    capture_id: String,
    complete: bool,
    waits: BTreeMap<u64, WaitLifecycle>,
}

impl ValidatedQueryWaitInventory {
    pub(crate) fn contains_resumed(&self, capture_id: &str, wait_id: u64) -> bool {
        self.complete
            && self.capture_id == capture_id
            && self.waits.get(&wait_id).is_some_and(|w| {
                !w.invalid && w.endpoints.is_some() && w.stages.iter().all(Option::is_some)
            })
    }
}

fn retain_wait_inventory(r: &ReceiptV4) -> ValidatedQueryWaitInventory {
    let mut waits: BTreeMap<u64, WaitLifecycle> = BTreeMap::new();
    for (index, event) in r.wait_events.iter().enumerate() {
        let (id, endpoints, ns, stage, reason) = event.parts();
        let wait = waits.entry(id).or_default();
        if wait.endpoints.is_some_and(|old| old != endpoints) || wait.stages[stage].is_some() {
            wait.invalid = true;
        }
        wait.endpoints = Some(endpoints);
        wait.stages[stage] = Some((index, ns));
        if reason.is_some() {
            wait.reason = reason;
        }
    }
    ValidatedQueryWaitInventory {
        capture_id: r.capture_id.clone(),
        complete: false,
        waits,
    }
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
    /// Explicitly requested first-cache availability observations, never execution DAG edges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_call_provenance: Option<crate::execution_graph_reads::CacheCallAssessment>,
    /// None for schemas 1/2/3: legacy absence is unknown, never zero lineage.
    pub task_bindings: Option<crate::execution_graph_task_bindings::BindingAssessment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock_alignment: Option<ClockAlignmentAssessment>,
    pub issues: Vec<String>,
    pub opaque_boundaries: Vec<String>,
    pub cpu_work_ns: Option<u64>,
    pub execution_span_ns: Option<u64>,
}

/// Parse the original bytes directly so duplicate fields cannot disappear through a Value.
/// Malformed/unsupported receipts fail parsing. Lifecycle discrepancies return issues.
/// Capture identity uniqueness across receipts must be enforced by the caller.
#[allow(dead_code)] // Public assessment-only entry point retained alongside collection.
pub fn assess_bytes(bytes: &[u8]) -> Result<QueryAssessment> {
    Ok(collect_bytes(bytes)?.assessment)
}

pub fn collect_bytes(bytes: &[u8]) -> Result<CollectedQuery> {
    collect_bytes_with_policy(bytes, CapturePolicy::strict())
}

pub(crate) fn collect_bytes_with_policy(
    bytes: &[u8],
    policy: CapturePolicy,
) -> Result<CollectedQuery> {
    collect_bytes_with_analysis(bytes, policy, false)
}

/// Analysis is explicit and never changes capture limits or legacy collectors.
pub(crate) fn collect_bytes_with_analysis(
    bytes: &[u8],
    policy: CapturePolicy,
    cache_call_provenance: bool,
) -> Result<CollectedQuery> {
    if policy.max_bytes > 512 * 1024 * 1024 || policy.max_points > 1_000_000 {
        bail!("query capture policy exceeds hard bounds");
    }
    if bytes.len() > policy.max_bytes {
        bail!("query receipt exceeds byte budget");
    }
    #[derive(Deserialize)]
    struct Version {
        schema: u32,
    }
    let version: Version =
        serde_json::from_slice(bytes).context("invalid query receipt version")?;
    if cache_call_provenance && !matches!(version.schema, 4 | 5) {
        bail!(
            "--cache-call-provenance requires query schema4 or schema5 with independent endpoint inventory"
        );
    }
    let assessment = match version.schema {
        1 => assess_admissions(
            serde_json::from_slice(bytes).context("invalid query admission receipt")?,
        ),
        2 => assess_waits(serde_json::from_slice(bytes).context("invalid query wait receipt")?),
        3 => assess_reads(serde_json::from_slice(bytes).context("invalid query read receipt")?),
        4 => return collect_bindings(bytes, policy, cache_call_provenance),
        5 => return collect_clock_bindings(bytes, policy, cache_call_provenance),
        _ => bail!("unsupported query receipt schema"),
    }?;
    Ok(CollectedQuery {
        assessment,
        bindings: None,
        wait_inventory: None,
        execution_clock: None,
        clock_binding: None,
    })
}

// This projection is read only AFTER the strict, duplicate-preserving ReceiptV4
// or ReceiptV5 parser accepted the original bytes. It does not replace either
// receipt parser or the nested read schema parser.
#[derive(Deserialize)]
struct ReadInventoryProjection {
    query_reads: ReadPointInventory,
    task_bindings: BindingIdentityProjection,
}
#[derive(Deserialize)]
struct BindingIdentityProjection {
    pool: Option<ClockBinding>,
}
#[derive(Deserialize)]
struct ReadPointInventory {
    request_high_water: u64,
    read_high_water: u64,
    result_high_water: u64,
    events: Vec<ReadPointEvent>,
}
#[derive(Deserialize)]
struct ReadPointEvent {
    kind: String,
    request_id: Option<u64>,
    read_id: Option<u64>,
    result_id: Option<u64>,
}

fn collect_bindings(
    bytes: &[u8],
    policy: CapturePolicy,
    cache_call_provenance: bool,
) -> Result<CollectedQuery> {
    let r: ReceiptV4 =
        serde_json::from_slice(bytes).context("invalid query task-binding receipt")?;
    collect_bindings_base(r, bytes, policy, cache_call_provenance)
}

fn collect_clock_bindings(
    bytes: &[u8],
    policy: CapturePolicy,
    cache_call_provenance: bool,
) -> Result<CollectedQuery> {
    let r: ReceiptV5 = serde_json::from_slice(bytes).context("invalid query clock receipt")?;
    if r.schema != 5 {
        bail!("unsupported query clock receipt schema");
    }
    let (base, clock) = r.into_base();
    let mut query = collect_bindings_base(base, bytes, policy, cache_call_provenance)?;
    query.assessment.schema = 5;
    query.execution_clock = clock;
    // Standalone collection has no validated reciprocal pool authority. Never
    // upgrade alignment based on a one-sided descriptor.
    query.reconcile_clock_alignment(&BTreeMap::new());
    Ok(query)
}

fn collect_bindings_base(
    r: ReceiptV4,
    bytes: &[u8],
    policy: CapturePolicy,
    cache_call_provenance: bool,
) -> Result<CollectedQuery> {
    use crate::execution_graph_task_bindings::{QuerySourceInventory, Source};
    if r.schema != 4 {
        bail!("unsupported query task-binding receipt schema");
    }
    let reads: ReadInventoryProjection = serde_json::from_slice(bytes)?;
    let clock_binding = reads.task_bindings.pool;
    let reads = reads.query_reads;
    if r.contexts.len() > MAX_CONTEXTS {
        bail!("task-binding context budget exceeded");
    }
    let mut inventory = QuerySourceInventory {
        expected_points: Some(0),
        ..Default::default()
    };
    let mut add_count = |count: Option<u64>| {
        inventory.expected_points = inventory
            .expected_points
            .and_then(|sum| sum.checked_add(count?));
    };
    for context in &r.contexts {
        add_count(
            context
                .allocator_high_water
                .and_then(|high| high.checked_sub(1)),
        );
    }
    for high in [
        reads.request_high_water,
        reads.read_high_water,
        reads.result_high_water,
    ] {
        add_count(high.checked_sub(1));
    }
    add_count(
        r.wait_allocator_high_water
            .and_then(|high| high.checked_sub(1))
            .and_then(|n| n.checked_mul(2)),
    );
    let context_ids: BTreeSet<_> = r.contexts.iter().map(|c| c.context_id).collect();
    if context_ids.len() != r.contexts.len()
        || context_ids.iter().copied().ne(1..=r.expected_contexts)
    {
        inventory.expected_points = None;
        inventory
            .issues
            .push("task-binding independent context inventory mismatch".into());
    }
    inventory.expected_contexts = r.expected_contexts;
    inventory.context_high_water = r
        .contexts
        .iter()
        .map(|c| (c.context_id, c.allocator_high_water))
        .collect();
    inventory.read_high_water = [
        reads.request_high_water,
        reads.read_high_water,
        reads.result_high_water,
    ];
    inventory.wait_high_water = r.wait_allocator_high_water;
    for context in &r.contexts {
        if context.allocator_high_water.is_none() {
            inventory.unknown_contexts.insert(context.context_id);
        }
    }
    inventory.unknown_waits = r.wait_allocator_high_water.is_none();
    for event in &r.events {
        if let Event::Admitted {
            context_id, job_id, ..
        } = event
        {
            inventory.observed.insert(Source::Admitted {
                context_id: *context_id,
                job_id: *job_id,
            });
        }
    }
    for event in &r.wait_events {
        match event {
            WaitEvent::Registered { wait_id, .. } => {
                inventory
                    .observed
                    .insert(Source::WaitRegistered { wait_id: *wait_id });
            }
            WaitEvent::Resumed { wait_id, .. } => {
                inventory
                    .observed
                    .insert(Source::WaitResumed { wait_id: *wait_id });
            }
            _ => {}
        }
    }
    for event in reads.events {
        let source = match event.kind.as_str() {
            "request_started" => event
                .request_id
                .map(|request_id| Source::RequestStarted { request_id }),
            "dep_read" => event.read_id.map(|read_id| Source::DepRead { read_id }),
            "result_produced" => event
                .result_id
                .map(|result_id| Source::ResultProduced { result_id }),
            _ => None,
        };
        if let Some(source) = source {
            inventory.observed.insert(source);
        }
    }
    // Assess admission, wait and read contracts independently. Binding partiality
    // must not downgrade any of these existing completeness assessments.
    let wait_inventory = retain_wait_inventory(&r);
    let mut assessment = assess_reads_policy(
        ReceiptV3 {
            schema: 3,
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
            query_reads: r.query_reads,
        },
        true,
        policy,
        cache_call_provenance,
    )?;
    let bindings = crate::execution_graph_task_bindings::assess_with_limit(
        r.task_bindings,
        inventory,
        policy.max_points,
    )?;
    assessment.schema = 4;
    assessment.task_bindings = Some(bindings.assessment.clone());
    let assessment_wait_complete = assessment.wait_structural_complete == Some(true);
    Ok(CollectedQuery {
        assessment,
        bindings: Some(bindings),
        wait_inventory: Some(ValidatedQueryWaitInventory {
            complete: assessment_wait_complete,
            ..wait_inventory
        }),
        execution_clock: None,
        clock_binding,
    })
}

fn assess_admissions(r: Receipt) -> Result<QueryAssessment> {
    assess_admissions_policy(r, false)
}

fn assess_admissions_policy(r: Receipt, large_inventory: bool) -> Result<QueryAssessment> {
    if r.schema != 1
        || r.producer != "rustc-query-admission"
        || r.scope != "query-job-admission-lifecycle"
    {
        bail!("unsupported query receipt schema, producer or scope");
    }
    if r.capture_id.is_empty() || r.capture_id.len() > 128 {
        bail!("query capture identity must contain 1..=128 bytes");
    }
    if (!large_inventory && r.expected_contexts > MAX_CONTEXTS as u64)
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
        if !large_inventory
            && (c
                .allocator_high_water
                .is_some_and(|high| high > MAX_JOBS + 1)
                || c.expected_jobs.is_some_and(|expected| expected > MAX_JOBS))
        {
            bail!("allocator counts exceed job budget");
        }
        match (c.allocator_high_water, c.expected_jobs) {
            (Some(high), Some(expected)) => {
                if !large_inventory && (expected > MAX_JOBS || high > MAX_JOBS + 1) {
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
    if !large_inventory && total_expected > MAX_JOBS {
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
        task_bindings: None,
        clock_alignment: None,
        edges,
        descriptive_parent_edges: edges,
        control_causal_edges: 0,
        wait_structural_complete: None,
        waits: None,
        query_reads: None,
        cache_call_provenance: None,
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
    assess_reads_policy(r, false, CapturePolicy::strict(), false)
}

fn assess_reads_policy(
    r: ReceiptV3,
    large_inventory: bool,
    policy: CapturePolicy,
    cache_call_provenance: bool,
) -> Result<QueryAssessment> {
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
    let context_high_water = r
        .contexts
        .iter()
        .map(|context| (context.context_id, context.allocator_high_water))
        .collect();
    let mut assessment = assess_waits_policy(
        ReceiptV2 {
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
        },
        large_inventory,
    )?;
    assessment.schema = 3;
    let reads = if large_inventory {
        if policy.max_points == CapturePolicy::strict().max_points {
            crate::execution_graph_reads::assess_with_inventory(
                &r.query_reads,
                &admitted,
                expected_contexts,
                &context_high_water,
            )?
        } else {
            crate::execution_graph_reads::assess_with_inventory_limit(
                &r.query_reads,
                &admitted,
                expected_contexts,
                &context_high_water,
                policy.max_points,
            )?
        }
    } else {
        crate::execution_graph_reads::assess(&r.query_reads, &admitted, expected_contexts)?
    };
    let admission_authority_complete = assessment.structural_complete;
    if !reads.records_complete {
        assessment.structural_complete = false;
        assessment
            .issues
            .push("query read records are incomplete or invalid".into());
    }
    if cache_call_provenance {
        let mut calls = crate::execution_graph_reads::assess_cache_calls_with_inventory_limit(
            &r.query_reads,
            &admitted,
            expected_contexts,
            &context_high_water,
            policy.max_points,
        )?;
        if !admission_authority_complete {
            calls.suppress_incomplete_admission_authority();
        }
        assessment.cache_call_provenance = Some(calls);
    }
    assessment.query_reads = Some(reads);
    Ok(assessment)
}

fn assess_waits(r: ReceiptV2) -> Result<QueryAssessment> {
    assess_waits_policy(r, false)
}

fn assess_waits_policy(r: ReceiptV2, large_inventory: bool) -> Result<QueryAssessment> {
    if r.schema != 2 {
        bail!("unsupported query wait receipt schema");
    }
    if r.wait_events.len() > MAX_EVENTS
        || (!large_inventory
            && (r.expected_waits.is_some_and(|n| n > MAX_JOBS)
                || r.wait_allocator_high_water
                    .is_some_and(|n| n > MAX_JOBS + 1)))
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
    let mut assessment = assess_admissions_policy(
        Receipt {
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
        },
        large_inventory,
    )?;
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
pub(crate) mod tests {
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

    pub(crate) fn task_binding_fixture() -> Value {
        let mut v = read_fixture();
        v["schema"] = json!(4);
        let start = json!({"kind":"request_started","request_id":1,"read_id":null,"result_id":null,
            "context_id":1,"consumer_job_id":2,"producer_job_id":null,"dep_node_index":null,"mode":"get","outcome":null});
        let mut finish = start.clone();
        finish["kind"] = json!("request_finished");
        finish["outcome"] = json!("value");
        let mut produced = start.clone();
        produced["kind"] = json!("result_produced");
        produced["request_id"] = Value::Null;
        produced["consumer_job_id"] = Value::Null;
        produced["producer_job_id"] = json!(1);
        produced["result_id"] = json!(1);
        produced["dep_node_index"] = json!(42);
        produced["mode"] = Value::Null;
        let mut read = start.clone();
        read["kind"] = json!("dep_read");
        read["read_id"] = json!(1);
        read["dep_node_index"] = json!(42);
        read["mode"] = Value::Null;
        read["outcome"] = json!("tracked_new");
        for name in ["request", "read", "result"] {
            v["query_reads"][format!("{name}_high_water")] = json!(2);
            v["query_reads"][format!("expected_{name}s")] = json!(1);
        }
        v["query_reads"]["events"] = json!([start, produced, read, finish]);
        let sources = [
            json!({"kind":"admitted","context_id":1,"job_id":1}),
            json!({"kind":"admitted","context_id":1,"job_id":2}),
            json!({"kind":"request_started","request_id":1}),
            json!({"kind":"dep_read","read_id":1}),
            json!({"kind":"result_produced","result_id":1}),
            json!({"kind":"wait_registered","wait_id":1}),
            json!({"kind":"wait_resumed","wait_id":1}),
        ];
        v["task_bindings"] = json!({"schema":1,"records_complete":true,"all_targets_bound":true,
            "expected_points":7,"lost_events":0,"loss_overflowed":false,
            "pool":{"capture_id":"pool-1","observer":1},"unavailable_reason":null,
            "events":sources.into_iter().map(|source| json!({"source":source,"target":{"task":1,"at_pool_ns":10},"unbound_reason":null})).collect::<Vec<_>>()});
        v
    }

    pub(crate) fn cache_call_fixture() -> Value {
        let mut v = task_binding_fixture();
        let mut hit = v["query_reads"]["events"][0].clone();
        hit["kind"] = json!("cache_hit");
        hit["dep_node_index"] = json!(42);
        v["query_reads"]["events"]
            .as_array_mut()
            .unwrap()
            .insert(2, hit);
        v
    }

    #[test]
    fn cache_call_provenance_is_explicit_and_preserves_legacy_assessments() {
        for schema in [4, 5] {
            let mut v = cache_call_fixture();
            if schema == 5 {
                v["schema"] = json!(5);
                v["execution_clock"] = Value::Null;
            }
            let bytes = serde_json::to_vec(&v).unwrap();
            let legacy = collect_bytes(&bytes).unwrap().assessment;
            let explicit_off = collect_bytes_with_analysis(&bytes, CapturePolicy::strict(), false)
                .unwrap()
                .assessment;
            assert_eq!(
                serde_json::to_vec(&legacy).unwrap(),
                serde_json::to_vec(&explicit_off).unwrap()
            );
            let legacy_json = serde_json::to_value(&legacy).unwrap();
            assert!(legacy_json.get("cache_call_provenance").is_none());
            for policy in [CapturePolicy::strict(), CapturePolicy::large()] {
                let mut analyzed = serde_json::to_value(
                    collect_bytes_with_analysis(&bytes, policy, true)
                        .unwrap()
                        .assessment,
                )
                .unwrap();
                let calls = analyzed
                    .as_object_mut()
                    .unwrap()
                    .remove("cache_call_provenance")
                    .unwrap();
                assert_eq!(calls["resolved_requests"], 1);
                assert_eq!(calls["relations"][0]["publication_event_index"], 1);
                assert_eq!(calls["relations"][0]["cache_hit_event_index"], 2);
                assert_eq!(calls["relations"][0]["request_terminal_event_index"], 4);
                assert_eq!(analyzed, legacy_json);
                assert_eq!(analyzed["cpu_work_ns"], Value::Null);
                assert_eq!(analyzed["execution_span_ns"], Value::Null);
            }
        }
        for v in [fixture(), wait_fixture(), read_fixture()] {
            let bytes = serde_json::to_vec(&v).unwrap();
            assert!(collect_bytes(&bytes).is_ok());
            assert!(
                collect_bytes_with_analysis(&bytes, CapturePolicy::strict(), true)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("requires query schema4 or schema5")
            );
        }
    }

    #[test]
    fn cache_call_provenance_reuses_original_bytes_and_independent_authority() {
        let v = cache_call_fixture();
        let text = serde_json::to_string(&v).unwrap();
        let bounded = CapturePolicy {
            max_bytes: text.len(),
            max_points: 7,
        };
        assert!(collect_bytes_with_analysis(text.as_bytes(), bounded, true).is_ok());
        for policy in [
            CapturePolicy {
                max_bytes: text.len() - 1,
                ..bounded
            },
            CapturePolicy {
                max_points: 4,
                ..bounded
            },
            CapturePolicy {
                max_points: 1_000_001,
                ..bounded
            },
        ] {
            assert!(collect_bytes_with_analysis(text.as_bytes(), policy, true).is_err());
        }
        for duplicate in [
            text.replacen("\"schema\":4", "\"schema\":4,\"schema\":4", 1),
            text.replacen(
                "\"kind\":\"cache_hit\"",
                "\"kind\":\"cache_hit\",\"kind\":\"cache_hit\"",
                1,
            ),
        ] {
            assert_ne!(duplicate, text);
            assert!(
                collect_bytes_with_analysis(duplicate.as_bytes(), CapturePolicy::large(), true)
                    .is_err()
            );
        }
        let mut incomplete = v.clone();
        incomplete["capture_complete"] = json!(false);
        incomplete["lost_events"] = json!(1);
        let a = collect_bytes_with_analysis(
            &serde_json::to_vec(&incomplete).unwrap(),
            CapturePolicy::strict(),
            true,
        )
        .unwrap()
        .assessment;
        let calls = serde_json::to_value(a.cache_call_provenance.unwrap()).unwrap();
        assert_eq!(calls["resolved_requests"], 0);
        assert_eq!(calls["relations"], json!([]));
        assert_eq!(
            calls["unresolved_reasons"]["admission_authority_incomplete"],
            1
        );
        let mut invalid = v;
        invalid["query_reads"]["events"][2]["consumer_job_id"] = json!(99);
        assert!(
            collect_bytes_with_analysis(
                &serde_json::to_vec(&invalid).unwrap(),
                CapturePolicy::large(),
                true
            )
            .is_err()
        );
    }

    fn clock_fixture() -> Value {
        let mut v = task_binding_fixture();
        v["schema"] = json!(5);
        v["execution_clock"] = json!({
            "query_capture_id":v["capture_id"],"pool_capture_id":"pool-1",
            "pool_observer":1,"pool_epoch_in_query_ns":0
        });
        v
    }

    fn clock_pools(
        v: &Value,
    ) -> BTreeMap<String, crate::execution_graph_continuation::PoolInventory> {
        use crate::execution_graph_continuation::{PoolInventory, TaskInterval};
        let clock: crate::execution_graph_timing::ClockBridge =
            serde_json::from_value(clock_fixture()["execution_clock"].clone()).unwrap();
        BTreeMap::from([(
            "pool-1".into(),
            PoolInventory {
                schema: 3,
                capture_id: "pool-1".into(),
                observer: 1,
                query_capture_id: Some(v["capture_id"].as_str().unwrap().into()),
                structural_complete: true,
                task_intervals: BTreeMap::from([(
                    1,
                    TaskInterval {
                        start_ns: 0,
                        finish_ns: 20,
                    },
                )]),
                timing_bridge: Some(clock),
            },
        )])
    }

    fn reconcile_clock(
        v: &Value,
        pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
    ) -> QueryAssessment {
        let mut query = collect_bytes(&serde_json::to_vec(v).unwrap()).unwrap();
        if let Some(bindings) = &query.bindings {
            query.assessment.task_bindings =
                Some(crate::execution_graph_task_bindings::reconcile_bindings(
                    &query.assessment.capture_id,
                    bindings,
                    pools,
                ));
        }
        query.reconcile_clock_alignment(pools);
        query.assessment
    }

    #[test]
    fn schema5_requires_reciprocal_clock_without_promoting_global_metrics() {
        let v = clock_fixture();
        let query = collect_bytes(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(query.assessment.schema, 5);
        assert!(query.assessment.structural_complete);
        assert!(!query.assessment.clock_alignment.unwrap().aligned);
        let a = reconcile_clock(&v, &clock_pools(&v));
        assert!(a.clock_alignment.as_ref().unwrap().aligned, "{a:?}");
        assert!(a.task_bindings.as_ref().unwrap().graph_binding_valid);
        assert!(!a.full_pipeline_complete);
        assert_eq!((a.cpu_work_ns, a.execution_span_ns), (None, None));
        let legacy = task_binding_fixture();
        let a = reconcile_clock(&legacy, &clock_pools(&legacy));
        assert!(a.clock_alignment.is_none());
        assert!(
            serde_json::to_value(a)
                .unwrap()
                .get("clock_alignment")
                .is_none()
        );
    }

    #[test]
    fn schema5_null_missing_and_mismatched_clocks_never_align() {
        let mut v = clock_fixture();
        v["execution_clock"] = Value::Null;
        let a = reconcile_clock(&v, &clock_pools(&v));
        assert!(a.structural_complete);
        assert!(!a.clock_alignment.unwrap().aligned);
        v.as_object_mut().unwrap().remove("execution_clock");
        assert!(collect_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        for (field, value) in [
            ("query_capture_id", json!("foreign-query")),
            ("pool_capture_id", json!("foreign-pool")),
            ("pool_capture_id", json!("synthetic-1")),
            ("pool_observer", json!(2)),
            ("pool_observer", json!(0)),
            ("pool_epoch_in_query_ns", json!(1)),
        ] {
            let mut v = clock_fixture();
            v["execution_clock"][field] = value;
            let a = reconcile_clock(&v, &clock_pools(&v));
            assert!(!a.clock_alignment.unwrap().aligned, "{field}");
        }
        for mutation in 0..7 {
            let v = clock_fixture();
            let mut pools = clock_pools(&v);
            let pool = pools.get_mut("pool-1").unwrap();
            match mutation {
                0 => pool.schema = 2,
                1 => pool.structural_complete = false,
                2 => pool.query_capture_id = Some("foreign-query".into()),
                3 => pool.observer = 2,
                4 => pool.timing_bridge = None,
                5 => pool.timing_bridge.as_mut().unwrap().pool_epoch_in_query_ns = 1,
                6 => pool.capture_id = "foreign-pool".into(),
                _ => unreachable!(),
            }
            assert!(!reconcile_clock(&v, &pools).clock_alignment.unwrap().aligned);
        }
    }

    #[test]
    fn schema5_clock_requires_binding_identity_and_valid_recorded_targets() {
        for mutation in 0..3 {
            let mut v = clock_fixture();
            match mutation {
                0 => v["task_bindings"]["pool"]["observer"] = json!(2),
                1 => v["task_bindings"]["pool"]["capture_id"] = json!("other-pool"),
                2 => v["task_bindings"]["events"][0]["target"]["at_pool_ns"] = json!(21),
                _ => unreachable!(),
            }
            assert!(
                !reconcile_clock(&v, &clock_pools(&v))
                    .clock_alignment
                    .unwrap()
                    .aligned
            );
        }
        // Incomplete target coverage is not a different clock epoch. Valid
        // remaining lineage may align, while completeness stays false.
        let mut v = clock_fixture();
        v["task_bindings"]["all_targets_bound"] = json!(false);
        v["task_bindings"]["events"][0]["target"] = Value::Null;
        v["task_bindings"]["events"][0]["unbound_reason"] = json!("no_active");
        let a = reconcile_clock(&v, &clock_pools(&v));
        assert!(a.clock_alignment.unwrap().aligned);
        assert!(!a.task_bindings.unwrap().graph_binding_complete);
    }

    #[test]
    fn schema5_original_bytes_reject_duplicate_unknown_and_missing_fields() {
        let raw = serde_json::to_string(&clock_fixture()).unwrap();
        for (old, new) in [
            ("\"schema\":5", "\"schema\":5,\"schema\":5"),
            (
                "\"pool_observer\":1",
                "\"pool_observer\":1,\"pool_observer\":1",
            ),
            (
                "\"execution_clock\":{",
                "\"execution_clock\":null,\"execution_clock\":{",
            ),
            ("\"read_id\":1", "\"read_id\":1,\"read_id\":1"),
        ] {
            let duplicate = raw.replacen(old, new, 1);
            assert_ne!(duplicate, raw);
            assert!(
                collect_bytes_with_policy(duplicate.as_bytes(), CapturePolicy::large()).is_err()
            );
        }
        for field in [
            "query_capture_id",
            "pool_capture_id",
            "pool_observer",
            "pool_epoch_in_query_ns",
        ] {
            let mut v = clock_fixture();
            v["execution_clock"].as_object_mut().unwrap().remove(field);
            assert!(collect_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        for path in [
            vec![],
            vec!["execution_clock"],
            vec!["query_reads"],
            vec!["task_bindings"],
        ] {
            let mut v = clock_fixture();
            let mut object = &mut v;
            for field in path {
                object = &mut object[field];
            }
            object["unknown"] = json!(true);
            assert!(
                collect_bytes_with_policy(&serde_json::to_vec(&v).unwrap(), CapturePolicy::large())
                    .is_err()
            );
        }
        let mut legacy = task_binding_fixture();
        legacy["execution_clock"] = Value::Null;
        assert!(collect_bytes(&serde_json::to_vec(&legacy).unwrap()).is_err());
    }

    #[test]
    fn schema5_typed_conversion_preserves_base_policy_and_inventory_gates() {
        let v = clock_fixture();
        let bytes = serde_json::to_vec(&v).unwrap();
        let policy = CapturePolicy {
            max_bytes: bytes.len(),
            max_points: 7,
        };
        let a = collect_bytes_with_policy(&bytes, policy)
            .unwrap()
            .assessment;
        assert_eq!(a.schema, 5);
        assert_eq!(a.query_reads.unwrap().resolved_reads, 1);
        assert_eq!(a.task_bindings.unwrap().expected_points, Some(7));
        for policy in [
            CapturePolicy {
                max_bytes: bytes.len() - 1,
                ..policy
            },
            CapturePolicy {
                max_points: 6,
                ..policy
            },
        ] {
            assert!(collect_bytes_with_policy(&bytes, policy).is_err());
        }
        let mut invalid = v;
        invalid["query_reads"]["events"][1]["producer_job_id"] = json!(99);
        assert!(
            collect_bytes_with_policy(
                &serde_json::to_vec(&invalid).unwrap(),
                CapturePolicy::large()
            )
            .is_err()
        );
    }

    #[test]
    fn large_capture_policy_is_explicit_and_bounded() {
        assert_eq!(CapturePolicy::strict().max_bytes, 64 * 1024 * 1024);
        assert_eq!(CapturePolicy::strict().max_points, 100_000);
        assert_eq!(CapturePolicy::large().max_bytes, 512 * 1024 * 1024);
        assert_eq!(CapturePolicy::large().max_points, 1_000_000);
        let bytes = serde_json::to_vec(&task_binding_fixture()).unwrap();
        let small = CapturePolicy {
            max_bytes: bytes.len(),
            max_points: 7,
        };
        let collected = collect_bytes_with_policy(&bytes, small).unwrap();
        assert_eq!(collected.assessment.schema, 4);
        assert!(collected.assessment.structural_complete);
        assert!(collected.assessment.task_bindings.unwrap().records_complete);
        for policy in [
            CapturePolicy {
                max_bytes: bytes.len() - 1,
                ..small
            },
            CapturePolicy {
                max_points: 6,
                ..small
            },
            CapturePolicy {
                max_points: 1_000_001,
                ..small
            },
            CapturePolicy {
                max_bytes: 512 * 1024 * 1024 + 1,
                ..small
            },
        ] {
            assert!(collect_bytes_with_policy(&bytes, policy).is_err());
        }
    }

    #[test]
    fn large_capture_keeps_strict_original_bytes_and_independent_bounds() {
        let v = task_binding_fixture();
        let raw = serde_json::to_string(&v).unwrap();
        let duplicate = raw.replacen("\"schema\":4", "\"schema\":4,\"schema\":4", 1);
        assert_ne!(duplicate, raw);
        assert!(collect_bytes_with_policy(duplicate.as_bytes(), CapturePolicy::large()).is_err());
        let mut invalid = v.clone();
        invalid["task_bindings"]["events"][0]["target"]["task"] = json!(0);
        assert!(
            collect_bytes_with_policy(
                &serde_json::to_vec(&invalid).unwrap(),
                CapturePolicy::large()
            )
            .is_err()
        );
        for field in ["events", "wait_events"] {
            let mut invalid = v.clone();
            let row = invalid[field][0].clone();
            invalid[field] = Value::Array(vec![row; MAX_EVENTS + 1]);
            assert!(
                collect_bytes_with_policy(
                    &serde_json::to_vec(&invalid).unwrap(),
                    CapturePolicy::large()
                )
                .is_err()
            );
        }
        let mut partial = v;
        partial["query_reads"]["events"]
            .as_array_mut()
            .unwrap()
            .remove(1);
        partial["query_reads"]["records_complete"] = json!(false);
        partial["query_reads"]["lost_events"] = json!(1);
        let a = collect_bytes_with_policy(
            &serde_json::to_vec(&partial).unwrap(),
            CapturePolicy::large(),
        )
        .unwrap()
        .assessment;
        assert!(!a.query_reads.unwrap().records_complete);
        assert!(!a.task_bindings.unwrap().graph_binding_complete);
    }

    #[test]
    fn schema4_separates_records_targets_and_pool_authority() {
        let v = task_binding_fixture();
        let collected = collect_bytes(&serde_json::to_vec(&v).unwrap()).unwrap();
        let a = collected.assessment;
        assert!(a.structural_complete, "{a:?}");
        assert_eq!(a.wait_structural_complete, Some(true));
        assert!(a.query_reads.as_ref().unwrap().records_complete);
        let binding = a.task_bindings.unwrap();
        assert!(binding.records_complete && binding.all_targets_bound);
        assert!(!binding.graph_binding_complete);
        assert_eq!(
            (
                binding.expected_points,
                binding.bound_points,
                binding.unbound_points
            ),
            (Some(7), 7, 0)
        );
        assert!(!a.full_pipeline_complete);
        assert_eq!((a.cpu_work_ns, a.execution_span_ns), (None, None));
        for v in [fixture(), wait_fixture(), read_fixture()] {
            assert!(assess(&v).task_bindings.is_none());
        }
    }

    #[test]
    fn schema4_missing_duplicate_foreign_source_category_matrix() {
        for index in 0..7 {
            let mut v = task_binding_fixture();
            v["task_bindings"]["events"]
                .as_array_mut()
                .unwrap()
                .remove(index);
            malformed(&v);
            v["task_bindings"]["records_complete"] = json!(false);
            v["task_bindings"]["all_targets_bound"] = json!(false);
            let a = assess(&v);
            assert!(a.structural_complete);
            assert!(!a.task_bindings.unwrap().records_complete);
            let mut v = task_binding_fixture();
            let row = v["task_bindings"]["events"][index].clone();
            v["task_bindings"]["events"]
                .as_array_mut()
                .unwrap()
                .push(row);
            malformed(&v);
            let mut v = task_binding_fixture();
            let source = v["task_bindings"]["events"][index]["source"]
                .as_object_mut()
                .unwrap();
            let id = source
                .keys()
                .find(|key| key.ends_with("_id") && key.as_str() != "context_id")
                .unwrap()
                .clone();
            source.insert(id, json!(99));
            malformed(&v);
        }
        let mut v = task_binding_fixture();
        v["task_bindings"]["events"][0]["source"]["context_id"] = json!(2);
        malformed(&v);
    }

    #[test]
    fn schema4_strict_nested_nullable_and_unknown_matrix() {
        let paths: &[&[&str]] = &[
            &["task_bindings"],
            &["task_bindings", "pool"],
            &["task_bindings", "events", "0"],
            &["task_bindings", "events", "0", "source"],
            &["task_bindings", "events", "0", "target"],
        ];
        for path in paths {
            let mut v = task_binding_fixture();
            let mut object = &mut v;
            for key in *path {
                object = if *key == "0" {
                    &mut object[0]
                } else {
                    &mut object[*key]
                };
            }
            object["unknown"] = json!(0);
            malformed(&v);
        }
        for field in ["expected_points", "pool", "unavailable_reason"] {
            let mut v = task_binding_fixture();
            v["task_bindings"].as_object_mut().unwrap().remove(field);
            malformed(&v);
        }
        for field in ["target", "unbound_reason"] {
            let mut v = task_binding_fixture();
            v["task_bindings"]["events"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            malformed(&v);
        }
        for schema in [1, 2, 3] {
            let mut v = task_binding_fixture();
            v["schema"] = json!(schema);
            malformed(&v);
        }
        let raw = serde_json::to_string(&task_binding_fixture()).unwrap();
        for (key, value) in [
            ("schema", "1"),
            ("observer", "1"),
            ("at_pool_ns", "10"),
            ("job_id", "1"),
            ("unbound_reason", "null"),
            ("expected_points", "7"),
        ] {
            let needle = format!("\"{key}\":{value}");
            let duplicate = raw.replacen(&needle, &format!("{needle},{needle}"), 1);
            assert_ne!(duplicate, raw);
            assert!(assess_bytes(duplicate.as_bytes()).is_err(), "{key}");
        }
    }

    #[test]
    fn schema4_loss_counters_nulls_and_honest_partial_matrix() {
        for (field, value) in [
            ("lost_events", json!(1)),
            ("loss_overflowed", json!(true)),
            ("expected_points", json!(8)),
            ("expected_points", Value::Null),
            ("expected_points", json!(u64::MAX)),
        ] {
            let mut v = task_binding_fixture();
            v["task_bindings"][field] = value;
            malformed(&v);
        }
        for field in ["lost_events", "loss_overflowed"] {
            let mut v = task_binding_fixture();
            v["task_bindings"][field] = if field == "lost_events" {
                json!(1)
            } else {
                json!(true)
            };
            v["task_bindings"]["records_complete"] = json!(false);
            v["task_bindings"]["all_targets_bound"] = json!(false);
            let a = assess(&v);
            assert!(a.structural_complete);
            assert_eq!(a.wait_structural_complete, Some(true));
            assert!(a.query_reads.unwrap().records_complete);
            assert!(!a.task_bindings.unwrap().records_complete);
        }
        for reason in [
            "no_active",
            "suspended",
            "foreign_observer",
            "clock_overflow",
            "pool_unavailable",
        ] {
            let mut v = task_binding_fixture();
            v["task_bindings"]["events"][0]["target"] = Value::Null;
            v["task_bindings"]["events"][0]["unbound_reason"] = json!(reason);
            malformed(&v);
            v["task_bindings"]["all_targets_bound"] = json!(false);
            let a = assess(&v);
            assert!(a.structural_complete);
            let b = a.task_bindings.unwrap();
            assert!(b.records_complete && !b.all_targets_bound);
            assert_eq!((b.bound_points, b.unbound_points), (6, 1));
        }
        let mut v = task_binding_fixture();
        v["task_bindings"]["pool"] = Value::Null;
        v["task_bindings"]["unavailable_reason"] = json!("pool_unavailable");
        v["task_bindings"]["all_targets_bound"] = json!(false);
        for row in v["task_bindings"]["events"].as_array_mut().unwrap() {
            row["target"] = Value::Null;
            row["unbound_reason"] = json!("pool_unavailable");
        }
        let b = assess(&v).task_bindings.unwrap();
        assert!(b.records_complete);
        assert_eq!(b.unbound_points, 7);
        let mut v = task_binding_fixture();
        v["contexts"][0]["allocator_high_water"] = Value::Null;
        v["contexts"][0]["expected_jobs"] = Value::Null;
        v["task_bindings"]["expected_points"] = Value::Null;
        v["task_bindings"]["records_complete"] = json!(false);
        v["task_bindings"]["all_targets_bound"] = json!(false);
        assert_eq!(assess(&v).task_bindings.unwrap().expected_points, None);
    }

    #[test]
    fn schema4_unknown_total_missing_context_and_sticky_overflow() {
        for missing_context in [false, true] {
            let mut v = task_binding_fixture();
            if missing_context {
                v["expected_contexts"] = json!(2);
            }
            v["task_bindings"]["expected_points"] = Value::Null;
            v["task_bindings"]["records_complete"] = json!(false);
            v["task_bindings"]["all_targets_bound"] = json!(false);
            // A sticky overflow latch remains evidence of loss after modulo wrap.
            v["task_bindings"]["loss_overflowed"] = json!(true);
            v["task_bindings"]["lost_events"] = json!(1);
            let a = assess(&v);
            assert_eq!(a.structural_complete, !missing_context);
            let b = a.task_bindings.unwrap();
            assert_eq!(b.expected_points, None);
            assert!(!b.records_complete && !b.all_targets_bound && !b.graph_binding_complete);
            assert_eq!(b.bound_points, 7);
            assert!(!b.issues.is_empty());
        }
    }

    #[test]
    fn schema4_huge_independent_counts_do_not_expand_inventory() {
        for category in ["context", "read", "wait"] {
            let mut v = task_binding_fixture();
            let extra: u64 = match category {
                "context" => {
                    v["contexts"][0]["allocator_high_water"] = json!(1_000_001);
                    v["contexts"][0]["expected_jobs"] = json!(1_000_000);
                    v["capture_complete"] = json!(false);
                    999_998
                }
                "read" => {
                    v["query_reads"]["read_high_water"] = json!(1_000_001);
                    v["query_reads"]["expected_reads"] = json!(1_000_000);
                    v["query_reads"]["records_complete"] = json!(false);
                    v["query_reads"]["lost_events"] = json!(1);
                    999_999
                }
                _ => {
                    v["wait_allocator_high_water"] = json!(1_000_001);
                    v["expected_waits"] = json!(1_000_000);
                    v["wait_capture_complete"] = json!(false);
                    1_999_998
                }
            };
            v["task_bindings"]["expected_points"] = json!(7 + extra);
            v["task_bindings"]["records_complete"] = json!(false);
            v["task_bindings"]["all_targets_bound"] = json!(false);
            v["task_bindings"]["lost_events"] = json!(extra);
            let b = assess(&v).task_bindings.unwrap();
            assert_eq!(b.expected_points, Some(7 + extra));
            assert_eq!(b.bound_points, 7);
            assert!(!b.records_complete);
        }
    }

    #[test]
    fn schema4_admission_loss_preserves_complete_read_records() {
        let mut v = task_binding_fixture();
        v["events"].as_array_mut().unwrap().remove(0);
        v["capture_complete"] = json!(false);
        v["lost_events"] = json!(1);
        let a = assess(&v);
        assert!(!a.structural_complete);
        let reads = a.query_reads.unwrap();
        assert!(reads.records_complete);
        assert!(!reads.endpoints_complete && !reads.semantic_complete);
        assert_eq!(reads.resolved_job_edges, 0);
        let bindings = a.task_bindings.unwrap();
        assert!(bindings.records_complete && bindings.all_targets_bound);
        assert!(!bindings.source_endpoints_complete && !bindings.graph_binding_valid);
        v["query_reads"]["events"][1]["producer_job_id"] = json!(99);
        malformed(&v);
    }

    #[test]
    fn schema4_underlying_read_loss_does_not_falsify_binding_record_inventory() {
        let mut v = task_binding_fixture();
        v["query_reads"]["events"].as_array_mut().unwrap().remove(1);
        v["query_reads"]["records_complete"] = json!(false);
        v["query_reads"]["lost_events"] = json!(1);
        let a = assess(&v);
        assert!(!a.query_reads.unwrap().records_complete);
        let b = a.task_bindings.unwrap();
        assert!(b.records_complete && b.all_targets_bound);
        assert!(!b.source_endpoints_complete && !b.graph_binding_complete);
        assert!(!b.issues.is_empty());
    }

    #[test]
    fn schema4_budgets_and_incoherent_target_reasons() {
        let mut v = task_binding_fixture();
        let row = v["task_bindings"]["events"][0].clone();
        v["task_bindings"]["events"] = Value::Array(vec![row; MAX_EVENTS + 1]);
        malformed(&v);
        for high in [json!(u64::MAX), json!(MAX_JOBS + 2), json!(0)] {
            let mut v = task_binding_fixture();
            v["contexts"][0]["allocator_high_water"] = high;
            malformed(&v);
        }
        for (field, value) in [
            ("task", json!(0)),
            ("task", json!(-1)),
            ("at_pool_ns", Value::Null),
        ] {
            let mut v = task_binding_fixture();
            v["task_bindings"]["events"][0]["target"][field] = value;
            malformed(&v);
        }
        {
            let reason = "foreign_kind";
            let mut v = task_binding_fixture();
            v["task_bindings"]["events"][0]["target"] = Value::Null;
            v["task_bindings"]["events"][0]["unbound_reason"] = json!(reason);
            v["task_bindings"]["all_targets_bound"] = json!(false);
            malformed(&v);
        }
        let mut v = task_binding_fixture();
        v["task_bindings"]["records_complete"] = json!(false);
        malformed(&v); // all_targets_bound cannot bypass declared record incompleteness.
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
