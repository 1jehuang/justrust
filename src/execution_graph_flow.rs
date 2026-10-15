//! Strict explicit execution-flow facts. Known-chain diagnostics are never full span.
use anyhow::{Result, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_FACTS: usize = 4_000_000;
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FlowCapture {
    schema: u32,
    scope: String,
    full_pipeline_complete: bool,
    flow_complete: bool,
    sealed: bool,
    records_complete: bool,
    identity_exhausted: bool,
    tls_failed: bool,
    active_scopes: u64,
    node_allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    node_allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_nodes: Option<u64>,
    edge_allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    edge_allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_edges: Option<u64>,
    scope_allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    scope_allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_scopes: Option<u64>,
    lost_nodes: u64,
    lost_edges: u64,
    lost_scopes: u64,
    loss_overflowed: bool,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    scopes: Vec<Scope>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum NodeKind {
    CpuSegment,
    ScopeEnter,
    ScopeExit,
    Fork,
    TaskEnter,
    TaskExit,
    Join,
    Cut,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Returned,
    Unwound,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GapReason {
    Dispatch,
    UnobservedQuery,
    ForeignRecorder,
    TaskSwitch,
    MissingFlow,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    id: u64,
    at_ns: u64,
    kind: NodeKind,
    #[serde(deserialize_with = "nullable")]
    segment_id: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    scope_id: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    task: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    outcome: Option<Outcome>,
    #[serde(deserialize_with = "nullable")]
    gap_reason: Option<GapReason>,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EdgeKind {
    Strand,
    SyncCall,
    SyncReturn,
    ForkBranch,
    JoinRequired,
    Continuation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edge {
    id: u64,
    from: u64,
    to: u64,
    kind: EdgeKind,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ScopeOutcome {
    Active,
    Returned,
    Unwound,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scope {
    id: u64,
    query_context: u64,
    query_job: u64,
    query_name: String,
    entry: u64,
    #[serde(deserialize_with = "nullable")]
    exit: Option<u64>,
    outcome: ScopeOutcome,
}

pub(crate) struct FlowContext<'a> {
    pub pool_capture_id: &'a str,
    pub query_capture_id: &'a str,
    pub observer: u64,
    pub pool_sealed: bool,
    pub structural_complete: bool,
    pub timing: &'a crate::execution_graph_timing::ObservedCpuAssessment,
    pub query_inventory: Option<&'a crate::execution_graph_query::QueryExecutionInventory>,
    pub clock_aligned: bool,
    pub task_high_water: u64,
    pub task_intervals: &'a BTreeMap<u64, crate::execution_graph_continuation::TaskInterval>,
    pub task_edges: &'a [crate::execution_graph_continuation::TaskEdge],
    pub task_outcomes: &'a BTreeMap<u64, &'static str>,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct FlowAssessment {
    pub scope: &'static str,
    pub pool_capture_id: String,
    pub pool_observer: u64,
    pub records_complete: bool,
    pub full_pipeline_complete: bool,
    pub flow_complete: bool,
    pub nodes: usize,
    pub edges: usize,
    pub scopes: usize,
    pub active_scopes: u64,
    pub lost_nodes: u64,
    pub lost_edges: u64,
    pub lost_scopes: u64,
    pub assigned_cpu_segments: usize,
    pub unassigned_cpu_segments: usize,
    pub unassigned_known_cpu_ns: Option<u64>,
    pub missing_measurement_references: usize,
    pub unavailable_cpu_segments: usize,
    pub cut_nodes: usize,
    /// Retained, individually authorized measurements only, not total work.
    pub known_cpu_work_ns: Option<u64>,
    pub known_query_cpu_ns: Option<u64>,
    pub known_task_cpu_ns: Option<u64>,
    pub known_opaque_cpu_ns: Option<u64>,
    /// A path through known costs only. Unknown nodes break this diagnostic.
    pub longest_known_chain_cpu_ns: Option<u64>,
    pub longest_known_chain_path: Vec<u64>,
    pub longest_known_chain_steps: Vec<FlowChainStep>,
    pub cpu_work_ns: Option<u64>,
    pub execution_span_ns: Option<u64>,
    pub parallelism: Option<f64>,
    pub issues: Vec<String>,
    pub limitations: Vec<String>,
}

impl FlowAssessment {
    pub(crate) fn unavailable(
        pool_capture_id: &str,
        observer: u64,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            scope: "observed-query-and-join-control-flow",
            pool_capture_id: pool_capture_id.into(),
            pool_observer: observer,
            issues: vec![reason.into()],
            limitations: vec!["no uniquely qualified original flow source was assessed".into()],
            ..Self::default()
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct FlowChainStep {
    pub node_id: u64,
    pub kind: &'static str,
    pub at_ns: u64,
    pub segment_id: Option<u64>,
    pub scope_id: Option<u64>,
    pub task: Option<u64>,
    pub query_context: Option<u64>,
    pub query_job: Option<u64>,
    pub query_name: Option<String>,
    pub cpu_ns: Option<u64>,
}

impl NodeKind {
    fn label(self) -> &'static str {
        match self {
            Self::CpuSegment => "cpu_segment",
            Self::ScopeEnter => "scope_enter",
            Self::ScopeExit => "scope_exit",
            Self::Fork => "fork",
            Self::TaskEnter => "task_enter",
            Self::TaskExit => "task_exit",
            Self::Join => "join",
            Self::Cut => "cut",
        }
    }
}

fn inventory(
    baseline: u64,
    high: Option<u64>,
    expected: Option<u64>,
    ids: impl Iterator<Item = u64>,
    lost: u64,
    exhausted: bool,
    overflowed: bool,
) -> Result<(BTreeSet<u64>, bool)> {
    ensure!(baseline == 1, "invalid flow allocator baseline");
    match (high, expected) {
        (Some(high), Some(expected)) => {
            ensure!(
                high.checked_sub(1) == Some(expected),
                "flow allocator count mismatch"
            );
        }
        (None, None) => ensure!(exhausted, "missing unexhausted flow allocator"),
        _ => anyhow::bail!("inconsistent nullable flow allocator inventory"),
    }
    let mut seen = BTreeSet::new();
    for id in ids {
        ensure!(
            id != 0 && high.is_none_or(|h| id < h),
            "flow ID outside allocator range"
        );
        ensure!(seen.insert(id), "duplicate original flow identity");
    }
    if let Some(expected) = expected {
        ensure!(
            seen.len() as u64 <= expected,
            "flow inventory exceeds allocation"
        );
        if !overflowed {
            ensure!(
                (seen.len() as u64).checked_add(lost) == Some(expected),
                "flow loss accounting mismatch"
            );
        }
    }
    let complete = high.is_some() && lost == 0 && !overflowed && !exhausted;
    Ok((seen, complete))
}

fn valid_name(s: &str) -> bool {
    let mut bytes = s.bytes();
    !s.is_empty()
        && s.len() <= 128
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn reference(id: u64, high: Option<u64>, present: bool, can_be_lost: bool) -> Result<bool> {
    ensure!(
        id != 0 && high.is_none_or(|h| id < h),
        "flow reference outside allocator range"
    );
    ensure!(
        present || can_be_lost,
        "flow references an absent original identity without loss"
    );
    Ok(present)
}

pub(crate) fn assess(capture: FlowCapture, context: FlowContext<'_>) -> Result<FlowAssessment> {
    ensure!(
        capture.schema == 1 && capture.scope == "observed-query-and-join-control-flow",
        "unsupported execution flow schema or scope"
    );
    ensure!(
        !capture.full_pipeline_complete && !capture.flow_complete,
        "execution flow cannot attest full execution coverage"
    );
    ensure!(
        capture.nodes.len() <= MAX_FACTS
            && capture.edges.len() <= MAX_FACTS
            && capture.scopes.len() <= MAX_FACTS,
        "execution flow inventory exceeds bounded budget"
    );
    let timing = context.timing;
    ensure!(
        timing.clock_bridge.pool_capture_id == context.pool_capture_id
            && timing.clock_bridge.query_capture_id == context.query_capture_id
            && timing.clock_bridge.pool_observer == context.observer,
        "execution flow timing belongs to another capture or observer"
    );
    let (_, nodes_complete) = inventory(
        capture.node_allocator_baseline,
        capture.node_allocator_high_water,
        capture.expected_nodes,
        capture.nodes.iter().map(|n| n.id),
        capture.lost_nodes,
        capture.identity_exhausted,
        capture.loss_overflowed,
    )?;
    let (_, edges_complete) = inventory(
        capture.edge_allocator_baseline,
        capture.edge_allocator_high_water,
        capture.expected_edges,
        capture.edges.iter().map(|n| n.id),
        capture.lost_edges,
        capture.identity_exhausted,
        capture.loss_overflowed,
    )?;
    let (_, scopes_complete) = inventory(
        capture.scope_allocator_baseline,
        capture.scope_allocator_high_water,
        capture.expected_scopes,
        capture.scopes.iter().map(|n| n.id),
        capture.lost_scopes,
        capture.identity_exhausted,
        capture.loss_overflowed,
    )?;
    let nodes: BTreeMap<_, _> = capture.nodes.iter().map(|n| (n.id, n)).collect();
    let scopes: BTreeMap<_, _> = capture.scopes.iter().map(|s| (s.id, s)).collect();
    let missing_nodes_allowed =
        capture.lost_nodes > 0 || capture.loss_overflowed || capture.identity_exhausted;
    let missing_scopes_allowed =
        capture.lost_scopes > 0 || capture.loss_overflowed || capture.identity_exhausted;
    let authority = context.query_inventory.filter(|q| {
        q.complete
            && q.capture_id == context.query_capture_id
            && q.clock.as_ref() == Some(&timing.clock_bridge)
            && context.clock_aligned
    });
    let mut names = BTreeMap::new();
    let mut scope_nodes = BTreeSet::new();
    let mut active = 0u64;
    for scope in &capture.scopes {
        ensure!(
            scope.query_context != 0 && scope.query_job != 0 && valid_name(&scope.query_name),
            "invalid execution scope query identity or category"
        );
        let key = (scope.query_context, scope.query_job);
        if let Some(previous) = names.insert(key, &scope.query_name) {
            ensure!(
                previous == &scope.query_name,
                "inconsistent execution scope query category"
            );
        }
        if let Some(q) = authority {
            ensure!(
                q.lifetimes.contains_key(&key),
                "execution scope is not an actual same-capture admitted query"
            );
        }
        reference(
            scope.entry,
            capture.node_allocator_high_water,
            nodes.contains_key(&scope.entry),
            missing_nodes_allowed,
        )?;
        ensure!(
            scope_nodes.insert(scope.entry),
            "aliased execution scope entry"
        );
        if let Some(entry) = nodes.get(&scope.entry) {
            ensure!(
                entry.kind == NodeKind::ScopeEnter && entry.scope_id == Some(scope.id),
                "nonreciprocal execution scope entry"
            );
            if let Some(q) = authority {
                let &(start, end) = &q.lifetimes[&key];
                let at = entry
                    .at_ns
                    .checked_add(timing.clock_bridge.pool_epoch_in_query_ns)
                    .ok_or_else(|| anyhow::anyhow!("scope entry query clock mapping overflow"))?;
                ensure!(
                    start <= at && at <= end,
                    "scope entry outside original query admission"
                );
            }
        }
        match (scope.outcome, scope.exit) {
            (ScopeOutcome::Active, None) => active += 1,
            (ScopeOutcome::Returned | ScopeOutcome::Unwound, Some(exit)) => {
                reference(
                    exit,
                    capture.node_allocator_high_water,
                    nodes.contains_key(&exit),
                    missing_nodes_allowed,
                )?;
                ensure!(scope_nodes.insert(exit), "aliased execution scope exit");
                if let Some(node) = nodes.get(&exit) {
                    let outcome = if scope.outcome == ScopeOutcome::Returned {
                        Outcome::Returned
                    } else {
                        Outcome::Unwound
                    };
                    ensure!(
                        node.kind == NodeKind::ScopeExit
                            && node.scope_id == Some(scope.id)
                            && node.outcome == Some(outcome),
                        "nonreciprocal execution scope terminal"
                    );
                    if let Some(entry) = nodes.get(&scope.entry) {
                        ensure!(entry.at_ns <= node.at_ns, "scope terminal precedes entry");
                    }
                    if let Some(q) = authority {
                        let &(start, end) = &q.lifetimes[&key];
                        let at = node
                            .at_ns
                            .checked_add(timing.clock_bridge.pool_epoch_in_query_ns)
                            .ok_or_else(|| {
                                anyhow::anyhow!("scope exit query clock mapping overflow")
                            })?;
                        ensure!(
                            start <= at && at <= end,
                            "scope exit outside original query admission"
                        );
                    }
                }
            }
            (ScopeOutcome::Returned | ScopeOutcome::Unwound, None)
                if capture.identity_exhausted => {}
            _ => anyhow::bail!("execution scope lifecycle does not match terminal"),
        }
    }
    ensure!(
        active <= capture.active_scopes,
        "active execution scope counter undercounts retained scopes"
    );
    if scopes_complete {
        ensure!(
            active == capture.active_scopes,
            "active execution scope inventory mismatch"
        );
    }
    let segments: BTreeMap<_, _> = timing
        .validated_segments
        .iter()
        .map(|s| (s.segment_id, s))
        .collect();
    ensure!(
        segments.len() == timing.validated_segments.len(),
        "duplicate validated CPU segment identity"
    );
    let mut assigned = BTreeSet::new();
    let mut costs = BTreeMap::new();
    let mut missing_measurement_references = 0;
    let mut unavailable_cpu_segments = 0;
    let mut cut_nodes = 0;
    let mut known_cpu = 0u64;
    let mut known_measurements = 0usize;
    let mut category_cpu = [0u64; 3];
    let mut category_measurements = [0usize; 3];
    let mut task_roles = BTreeSet::new();
    for node in &capture.nodes {
        if let Some(task) = node.task {
            ensure!(
                task != 0 && task < context.task_high_water,
                "flow task outside original allocator"
            );
            if let Some(interval) = context.task_intervals.get(&task) {
                ensure!(
                    interval.start_ns <= node.at_ns && node.at_ns <= interval.finish_ns,
                    "flow gate outside original task lifecycle"
                );
            } else {
                ensure!(
                    !context.structural_complete,
                    "flow task absent from original task lifecycles"
                );
            }
        } else {
            ensure!(
                !matches!(
                    node.kind,
                    NodeKind::Fork | NodeKind::TaskEnter | NodeKind::TaskExit | NodeKind::Join
                ),
                "task control gate lacks genuine task identity"
            );
        }
        if matches!(
            node.kind,
            NodeKind::TaskEnter | NodeKind::TaskExit | NodeKind::Fork | NodeKind::Join
        ) {
            ensure!(
                task_roles.insert((
                    node.task.expect("validated task control gate"),
                    node.kind.label()
                )),
                "duplicate flow lifecycle role for one original task"
            );
        }
        if node.kind == NodeKind::TaskExit {
            match context
                .task_outcomes
                .get(&node.task.expect("validated task exit"))
            {
                Some(&"normal") => ensure!(
                    node.outcome == Some(Outcome::Returned),
                    "flow task exit differs from original normal terminal"
                ),
                Some(&"panic") => ensure!(
                    node.outcome == Some(Outcome::Unwound),
                    "flow task exit differs from original panic terminal"
                ),
                Some(_) => anyhow::bail!("unsupported original task terminal outcome"),
                None => ensure!(
                    !context.structural_complete,
                    "flow task exit lacks original task terminal authority"
                ),
            }
        }
        ensure!(
            (node.kind == NodeKind::CpuSegment) == node.segment_id.is_some(),
            "CPU flow handle and node kind mismatch"
        );
        ensure!(
            matches!(node.kind, NodeKind::ScopeExit | NodeKind::TaskExit) == node.outcome.is_some(),
            "flow gate outcome and kind mismatch"
        );
        ensure!(
            (node.kind == NodeKind::Cut) == node.gap_reason.is_some(),
            "flow cut reason and kind mismatch"
        );
        if matches!(node.kind, NodeKind::ScopeEnter | NodeKind::ScopeExit) {
            ensure!(
                node.scope_id.is_some(),
                "scope gate lacks invocation identity"
            );
            if let Some(scope) = node.scope_id.and_then(|id| scopes.get(&id)) {
                ensure!(
                    node.id == scope.entry || Some(node.id) == scope.exit,
                    "extra scope lifecycle gate"
                );
            }
        }
        if let Some(id) = node.scope_id {
            reference(
                id,
                capture.scope_allocator_high_water,
                scopes.contains_key(&id),
                missing_scopes_allowed,
            )?;
            if let Some(scope) = scopes.get(&id) {
                if let Some(entry) = nodes.get(&scope.entry) {
                    ensure!(
                        entry.at_ns <= node.at_ns,
                        "flow node precedes actual invocation entry"
                    );
                }
                if let Some(exit) = scope.exit.and_then(|exit| nodes.get(&exit)) {
                    ensure!(
                        node.at_ns <= exit.at_ns,
                        "flow node follows actual invocation exit"
                    );
                }
            }
        }
        if node.kind == NodeKind::Cut {
            cut_nodes += 1;
        }
        let Some(segment_id) = node.segment_id else {
            costs.insert(
                node.id,
                if node.kind == NodeKind::Cut {
                    Cost::Unknown
                } else {
                    Cost::Gate
                },
            );
            continue;
        };
        ensure!(
            assigned.insert(segment_id),
            "original CPU segment assigned to multiple flow nodes"
        );
        reference(
            segment_id,
            timing.segment_allocator_high_water,
            segments.contains_key(&segment_id),
            timing.lost_segments > 0 || !timing.local_records_complete,
        )?;
        let Some(segment) = segments.get(&segment_id) else {
            missing_measurement_references += 1;
            costs.insert(node.id, Cost::Unknown);
            continue;
        };
        ensure!(
            segment.task == node.task,
            "CPU flow task differs from original segment"
        );
        if let Some(start) = segment.start_ns {
            ensure!(
                node.at_ns == start,
                "CPU flow time differs from original segment start"
            );
        }
        let mut authorized = true;
        match (segment.query_context, segment.query_job, node.scope_id) {
            (Some(query_context), Some(query_job), Some(scope_id)) => {
                if let Some(scope) = scopes.get(&scope_id) {
                    authorized = nodes.contains_key(&scope.entry)
                        && scope.exit.is_some_and(|id| nodes.contains_key(&id));
                    ensure!(
                        scope.query_context == query_context
                            && scope.query_job == query_job
                            && segment.query_name.as_deref() == Some(scope.query_name.as_str()),
                        "CPU flow invocation differs from original query owner"
                    );
                    if let (Some(end), Some(exit)) =
                        (segment.end_ns, scope.exit.and_then(|id| nodes.get(&id)))
                    {
                        ensure!(
                            end <= exit.at_ns,
                            "CPU segment extends beyond exact invocation exit"
                        );
                    }
                    if let Some(q) = authority {
                        let &(start, end) = q
                            .lifetimes
                            .get(&(query_context, query_job))
                            .ok_or_else(|| anyhow::anyhow!("CPU flow endpoint is not admitted"))?;
                        for wall in [segment.start_ns, segment.end_ns].into_iter().flatten() {
                            let wall = wall
                                .checked_add(timing.clock_bridge.pool_epoch_in_query_ns)
                                .ok_or_else(|| {
                                    anyhow::anyhow!("flow query clock mapping overflow")
                                })?;
                            ensure!(
                                start <= wall && wall <= end,
                                "CPU flow segment outside admitted query lifecycle"
                            );
                        }
                    } else {
                        authorized = false;
                    }
                } else {
                    authorized = false;
                }
            }
            (None, None, None) => {}
            _ => anyhow::bail!("CPU flow scope differs from original timing owner"),
        }
        let cost = if authorized && segment.measured && timing.endpoints_valid {
            if let Some(cpu) = segment.cpu_ns {
                known_cpu = known_cpu
                    .checked_add(cpu)
                    .ok_or_else(|| anyhow::anyhow!("known flow CPU work overflow"))?;
                known_measurements += 1;
                let category = if segment.query_context.is_some() {
                    0
                } else if segment.task.is_some() {
                    1
                } else {
                    2
                };
                category_cpu[category] = category_cpu[category]
                    .checked_add(cpu)
                    .ok_or_else(|| anyhow::anyhow!("flow CPU partition overflow"))?;
                category_measurements[category] += 1;
                Cost::Measured(cpu)
            } else {
                anyhow::bail!("validated measured segment lacks CPU cost");
            }
        } else {
            unavailable_cpu_segments += 1;
            Cost::Unknown
        };
        costs.insert(node.id, cost);
    }
    let task_edges: BTreeSet<_> = context
        .task_edges
        .iter()
        .map(|e| (e.from, e.to, e.kind))
        .collect();
    let mut edges = BTreeSet::new();
    let mut incoming = BTreeMap::<u64, usize>::new();
    let mut outgoing = BTreeMap::<u64, usize>::new();
    let mut callers = BTreeMap::<u64, Option<u64>>::new();
    let mut returns = BTreeMap::<u64, Option<u64>>::new();
    let mut observed_task_edges = BTreeSet::new();
    for edge in &capture.edges {
        reference(
            edge.from,
            capture.node_allocator_high_water,
            nodes.contains_key(&edge.from),
            missing_nodes_allowed,
        )?;
        reference(
            edge.to,
            capture.node_allocator_high_water,
            nodes.contains_key(&edge.to),
            missing_nodes_allowed,
        )?;
        ensure!(
            edge.from != edge.to && edges.insert((edge.from, edge.to)),
            "self or duplicate flow edge"
        );
        *incoming.entry(edge.to).or_default() += 1;
        *outgoing.entry(edge.from).or_default() += 1;
        if let (Some(from), Some(to)) = (nodes.get(&edge.from), nodes.get(&edge.to)) {
            validate_edge(edge.kind, from, to)?;
            if edge.kind == EdgeKind::SyncCall {
                ensure!(
                    callers
                        .insert(to.scope_id.expect("scope entry"), from.scope_id)
                        .is_none(),
                    "multiple callers for one actual invocation"
                );
            }
            if edge.kind == EdgeKind::SyncReturn {
                ensure!(
                    returns
                        .insert(from.scope_id.expect("scope exit"), to.scope_id)
                        .is_none(),
                    "multiple returns from one actual invocation"
                );
            }
            ensure!(
                from.at_ns <= to.at_ns,
                "flow edge contradicts native event clock"
            );
            if let Some(segment) = from.segment_id.and_then(|id| segments.get(&id))
                && let Some(end) = segment.end_ns
            {
                ensure!(
                    end <= to.at_ns,
                    "flow edge occurs before complete original CPU segment end"
                );
            }
            if matches!(edge.kind, EdgeKind::ForkBranch | EdgeKind::JoinRequired)
                && context.structural_complete
            {
                let role = if edge.kind == EdgeKind::ForkBranch {
                    "fork"
                } else {
                    "join"
                };
                ensure!(
                    task_edges.contains(&(
                        from.task.expect("validated task gate"),
                        to.task.expect("validated task gate"),
                        role
                    )),
                    "flow fork/join differs from original exact task lineage"
                );
                ensure!(
                    observed_task_edges.insert((
                        from.task.expect("task gate"),
                        to.task.expect("task gate"),
                        role
                    )),
                    "duplicate flow fork/join branch evidence"
                );
            }
        } else {
            // A lost original node is unknown, not an invented zero-cost gate.
            costs.entry(edge.from).or_insert(Cost::Unknown);
            costs.entry(edge.to).or_insert(Cost::Unknown);
        }
    }
    if capture.records_complete && context.structural_complete {
        ensure!(
            observed_task_edges == task_edges,
            "complete flow omits original fork/join branch evidence"
        );
    }
    for node in &capture.nodes {
        if node.kind != NodeKind::Join {
            ensure!(
                incoming.get(&node.id).copied().unwrap_or(0) <= 1,
                "multiple predecessors without explicit join gate"
            );
        }
        if node.kind != NodeKind::Fork {
            ensure!(
                outgoing.get(&node.id).copied().unwrap_or(0) <= 1,
                "multiple successors without explicit fork gate"
            );
        }
    }
    for (scope, returned_to) in &returns {
        match callers.get(scope) {
            Some(caller) => ensure!(
                caller == returned_to,
                "query execution returns to a different caller invocation"
            ),
            None => ensure!(
                !edges_complete || !nodes_complete,
                "root execution scope invents a synchronous caller return"
            ),
        }
    }
    if capture.records_complete && nodes_complete && edges_complete && scopes_complete {
        for scope in callers.keys() {
            if scopes
                .get(scope)
                .is_some_and(|scope| scope.outcome != ScopeOutcome::Active)
            {
                ensure!(
                    returns.contains_key(scope),
                    "complete called execution scope omits reciprocal return"
                );
            }
        }
    }
    if capture.tls_failed || capture.identity_exhausted {
        for cost in costs.values_mut() {
            *cost = Cost::Unknown;
        }
        known_measurements = 0;
        category_measurements = [0; 3];
    }
    let chain = known_chain(&costs, &edges)?;
    let longest_known_chain_steps = chain
        .as_ref()
        .map(|chain| {
            chain
                .path
                .iter()
                .map(|id| {
                    let node = nodes[id];
                    let scope =
                        authority.and_then(|_| node.scope_id.and_then(|id| scopes.get(&id)));
                    FlowChainStep {
                        node_id: *id,
                        kind: node.kind.label(),
                        at_ns: node.at_ns,
                        segment_id: node.segment_id,
                        scope_id: node.scope_id,
                        task: node.task,
                        query_context: scope.map(|s| s.query_context),
                        query_job: scope.map(|s| s.query_job),
                        query_name: scope
                            .and_then(|_| node.segment_id.and_then(|id| segments.get(&id)))
                            .and_then(|s| s.query_name.clone()),
                        cpu_ns: match costs[id] {
                            Cost::Measured(n) => Some(n),
                            Cost::Gate => Some(0),
                            Cost::Unknown => None,
                        },
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let unassigned: Vec<_> = segments
        .values()
        .filter(|s| !assigned.contains(&s.segment_id))
        .collect();
    let unassigned_known_cpu_ns = if timing.endpoints_valid
        && unassigned.iter().all(|s| {
            s.measured && s.cpu_ns.is_some() && (s.query_context.is_none() || authority.is_some())
        }) {
        Some(
            unassigned
                .iter()
                .try_fold(0u64, |sum, s| {
                    sum.checked_add(s.cpu_ns.expect("checked measured segment"))
                })
                .ok_or_else(|| anyhow::anyhow!("unassigned flow CPU sum overflow"))?,
        )
    } else {
        None
    };
    let local_complete = nodes_complete
        && edges_complete
        && scopes_complete
        && capture.sealed
        && capture.active_scopes == 0
        && !capture.tls_failed;
    ensure!(
        !capture.records_complete || local_complete,
        "false complete execution flow inventory"
    );
    let complete = capture.records_complete
        && local_complete
        && context.pool_sealed
        && context.structural_complete
        && timing.local_records_complete
        && timing.endpoints_valid
        && authority.is_some()
        && unassigned.is_empty()
        && missing_measurement_references == 0
        && unavailable_cpu_segments == 0;
    let mut limitations = vec!["wait/result readiness, generic dispatch, pre-attach and uninstrumented boundaries remain outside this graph".into()];
    let mut issues = Vec::new();
    if !complete {
        issues.push(
            "execution flow records or independent measurement/admission authority are incomplete"
                .into(),
        );
    }
    if cut_nodes > 0 {
        limitations.push(
            "explicit cuts are ancestry gaps, never zero-cost proof of missing CPU work".into(),
        );
    }
    Ok(FlowAssessment {
        scope: "observed-query-and-join-control-flow",
        pool_capture_id: context.pool_capture_id.into(),
        pool_observer: context.observer,
        records_complete: complete,
        full_pipeline_complete: false,
        flow_complete: false,
        nodes: capture.nodes.len(),
        edges: capture.edges.len(),
        scopes: capture.scopes.len(),
        active_scopes: capture.active_scopes,
        lost_nodes: capture.lost_nodes,
        lost_edges: capture.lost_edges,
        lost_scopes: capture.lost_scopes,
        assigned_cpu_segments: assigned.len(),
        unassigned_cpu_segments: unassigned.len(),
        unassigned_known_cpu_ns,
        missing_measurement_references,
        unavailable_cpu_segments,
        cut_nodes,
        known_cpu_work_ns: (known_measurements > 0).then_some(known_cpu),
        known_query_cpu_ns: (category_measurements[0] > 0).then_some(category_cpu[0]),
        known_task_cpu_ns: (category_measurements[1] > 0).then_some(category_cpu[1]),
        known_opaque_cpu_ns: (category_measurements[2] > 0).then_some(category_cpu[2]),
        longest_known_chain_cpu_ns: chain.as_ref().map(|c| c.cpu_ns),
        longest_known_chain_path: chain.map_or_else(Vec::new, |c| c.path),
        longest_known_chain_steps,
        cpu_work_ns: None,
        execution_span_ns: None,
        parallelism: None,
        issues,
        limitations,
    })
}

fn validate_edge(kind: EdgeKind, from: &Node, to: &Node) -> Result<()> {
    let task_compatible = from.task == to.task
        || (from.task.is_none() && matches!(from.kind, NodeKind::ScopeEnter | NodeKind::ScopeExit))
        || (to.task.is_none()
            && matches!(
                to.kind,
                NodeKind::ScopeEnter | NodeKind::ScopeExit | NodeKind::Cut
            ));
    match kind {
        EdgeKind::Strand => ensure!(
            from.scope_id == to.scope_id
                && task_compatible
                && !matches!(
                    from.kind,
                    NodeKind::ScopeExit | NodeKind::TaskExit | NodeKind::Fork | NodeKind::Join
                )
                && !matches!(
                    to.kind,
                    NodeKind::ScopeEnter | NodeKind::TaskEnter | NodeKind::Join
                ),
            "strand crosses invocation or task without explicit transition"
        ),
        EdgeKind::SyncCall => ensure!(
            to.kind == NodeKind::ScopeEnter
                && from.scope_id != to.scope_id
                && task_compatible
                && !matches!(
                    from.kind,
                    NodeKind::ScopeExit | NodeKind::TaskExit | NodeKind::Fork
                ),
            "invalid synchronous query call endpoints"
        ),
        EdgeKind::SyncReturn => ensure!(
            from.kind == NodeKind::ScopeExit
                && from.scope_id != to.scope_id
                && task_compatible
                && !matches!(
                    to.kind,
                    NodeKind::ScopeEnter | NodeKind::TaskEnter | NodeKind::Join
                ),
            "invalid synchronous query return endpoints"
        ),
        EdgeKind::ForkBranch => ensure!(
            from.kind == NodeKind::Fork && to.kind == NodeKind::TaskEnter,
            "invalid fork branch endpoints"
        ),
        EdgeKind::JoinRequired => ensure!(
            to.kind == NodeKind::Join && matches!(from.kind, NodeKind::TaskExit | NodeKind::Join),
            "invalid required join endpoints"
        ),
        EdgeKind::Continuation => ensure!(
            matches!(from.kind, NodeKind::Join | NodeKind::Cut)
                && (from.kind == NodeKind::Cut || from.task == to.task)
                && from.scope_id == to.scope_id
                && to.kind == NodeKind::CpuSegment,
            "continuation does not originate from explicit join"
        ),
    }
    Ok(())
}

/// Only an observed control gate has literal zero cost. An unavailable CPU
/// measurement breaks the known chain instead of silently becoming zero work.
#[derive(Clone, Copy, Debug)]
enum Cost {
    Gate,
    Measured(u64),
    Unknown,
}

#[derive(Debug, PartialEq, Eq)]
struct KnownChain {
    cpu_ns: u64,
    path: Vec<u64>,
}

/// All edges must already be explicitly justified by the native protocol.
/// Neither numeric IDs, row order, worker order nor timestamps create edges.
fn known_chain(
    nodes: &BTreeMap<u64, Cost>,
    edges: &BTreeSet<(u64, u64)>,
) -> Result<Option<KnownChain>> {
    let mut indegree: BTreeMap<_, usize> = nodes.keys().map(|&id| (id, 0)).collect();
    let mut successors: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for &(from, to) in edges {
        ensure!(from != to, "execution flow self edge");
        ensure!(nodes.contains_key(&from), "missing execution flow source");
        *indegree
            .get_mut(&to)
            .ok_or_else(|| anyhow::anyhow!("missing execution flow destination"))? += 1;
        successors.entry(from).or_default().push(to);
    }
    let mut ready: VecDeque<_> = indegree
        .iter()
        .filter_map(|(&id, &n)| (n == 0).then_some(id))
        .collect();
    // (known measured work, predecessor, contains an actual CPU measurement).
    let mut best: BTreeMap<u64, (u64, Option<u64>, bool)> = BTreeMap::new();
    let mut terminal = None;
    let mut visited = 0;
    while let Some(id) = ready.pop_front() {
        visited += 1;
        match nodes[&id] {
            Cost::Unknown => {
                best.remove(&id);
            }
            cost => {
                let entry = best.entry(id).or_insert((0, None, false));
                if let Cost::Measured(cpu) = cost {
                    entry.0 = entry
                        .0
                        .checked_add(cpu)
                        .ok_or_else(|| anyhow::anyhow!("known execution chain CPU overflow"))?;
                    entry.2 = true;
                }
                let current = *entry;
                if current.2
                    && terminal.is_none_or(|previous| {
                        let old = best[&previous];
                        (current.0, std::cmp::Reverse(id)) > (old.0, std::cmp::Reverse(previous))
                    })
                {
                    terminal = Some(id);
                }
            }
        }
        for &to in successors.get(&id).into_iter().flatten() {
            if let Some(&(cpu, _, measured)) = best.get(&id) {
                let candidate = (cpu, Some(id), measured);
                let destination = best.entry(to).or_insert(candidate);
                if (cpu, measured, std::cmp::Reverse(id))
                    > (
                        destination.0,
                        destination.2,
                        std::cmp::Reverse(destination.1.unwrap_or(u64::MAX)),
                    )
                {
                    *destination = candidate;
                }
            }
            let remaining = indegree.get_mut(&to).expect("validated endpoint");
            *remaining -= 1;
            if *remaining == 0 {
                ready.push_back(to);
            }
        }
    }
    ensure!(visited == nodes.len(), "execution flow cycle");
    let Some(mut id) = terminal else {
        return Ok(None);
    };
    let cpu_ns = best[&id].0;
    let mut path = Vec::new();
    loop {
        path.push(id);
        match best[&id].1 {
            Some(previous) => id = previous,
            None => break,
        }
    }
    path.reverse();
    Ok(Some(KnownChain { cpu_ns, path }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{Value, json};

    pub(crate) fn fixture() -> Value {
        json!({"schema":1,"scope":"observed-query-and-join-control-flow",
            "full_pipeline_complete":false,"flow_complete":false,"sealed":true,"records_complete":true,
            "identity_exhausted":false,"tls_failed":false,"active_scopes":0,
            "node_allocator_baseline":1,"node_allocator_high_water":5,"expected_nodes":4,
            "edge_allocator_baseline":1,"edge_allocator_high_water":3,"expected_edges":2,
            "scope_allocator_baseline":1,"scope_allocator_high_water":2,"expected_scopes":1,
            "lost_nodes":0,"lost_edges":0,"lost_scopes":0,"loss_overflowed":false,
            "nodes":[
                {"id":1,"at_ns":1,"kind":"scope_enter","segment_id":null,"scope_id":1,"task":1,"outcome":null,"gap_reason":null},
                {"id":2,"at_ns":10,"kind":"cpu_segment","segment_id":1,"scope_id":1,"task":1,"outcome":null,"gap_reason":null},
                {"id":3,"at_ns":20,"kind":"scope_exit","segment_id":null,"scope_id":1,"task":1,"outcome":"returned","gap_reason":null},
                {"id":4,"at_ns":20,"kind":"cpu_segment","segment_id":2,"scope_id":null,"task":null,"outcome":null,"gap_reason":null}],
            "edges":[{"id":1,"from":1,"to":2,"kind":"strand"},{"id":2,"from":2,"to":3,"kind":"strand"}],
            "scopes":[{"id":1,"query_context":1,"query_job":1,"query_name":"type_of","entry":1,"exit":3,"outcome":"returned"}]})
    }

    fn timing() -> crate::execution_graph_timing::ObservedCpuAssessment {
        let mut v = crate::execution_graph_timing::tests::fixture();
        v["schema"] = json!(2);
        for row in v["segments"].as_array_mut().unwrap() {
            row["query_context"] = Value::Null;
            row["query_job"] = Value::Null;
            row["query_name"] = Value::Null;
        }
        v["segments"][0]["owner_kind"] = json!("query_execution");
        v["segments"][0]["query_context"] = json!(1);
        v["segments"][0]["query_job"] = json!(1);
        v["segments"][0]["query_name"] = json!("type_of");
        let tasks = BTreeMap::from([(
            1,
            crate::execution_graph_continuation::TaskInterval {
                start_ns: 1,
                finish_ns: 100,
            },
        )]);
        crate::execution_graph_timing::assess_with_query_ownership(
            serde_json::from_value(v).unwrap(),
            crate::execution_graph_timing::TimingContext {
                pool_capture_id: "pool-test",
                query_capture_id: "query_123-safe",
                observer: 1,
                pool_sealed: true,
                structural_complete: true,
                task_high_water: 2,
                task_intervals: &tasks,
            },
            true,
        )
        .unwrap()
    }

    fn authority(
        t: &crate::execution_graph_timing::ObservedCpuAssessment,
    ) -> crate::execution_graph_query::QueryExecutionInventory {
        crate::execution_graph_query::QueryExecutionInventory {
            capture_id: "query_123-safe".into(),
            complete: true,
            clock: Some(t.clock_bridge.clone()),
            lifetimes: BTreeMap::from([((1, 1), (1, 100)), ((1, 2), (1, 100))]),
        }
    }

    fn collect(
        v: Value,
        t: &crate::execution_graph_timing::ObservedCpuAssessment,
        q: Option<&crate::execution_graph_query::QueryExecutionInventory>,
    ) -> Result<FlowAssessment> {
        let tasks = BTreeMap::from([(
            1,
            crate::execution_graph_continuation::TaskInterval {
                start_ns: 1,
                finish_ns: 100,
            },
        )]);
        assess(
            serde_json::from_value(v)?,
            FlowContext {
                pool_capture_id: "pool-test",
                query_capture_id: "query_123-safe",
                observer: 1,
                task_high_water: 2,
                task_intervals: &tasks,
                task_edges: &[],
                task_outcomes: &BTreeMap::new(),
                pool_sealed: true,
                structural_complete: true,
                timing: t,
                query_inventory: q,
                clock_aligned: true,
            },
        )
    }

    #[test]
    fn original_cpu_union_is_partitioned_without_full_metrics_or_ratio() {
        let t = timing();
        let a = collect(fixture(), &t, Some(&authority(&t))).unwrap();
        assert!(a.records_complete);
        assert_eq!(a.known_cpu_work_ns, Some(9));
        assert_eq!(a.longest_known_chain_cpu_ns, Some(6));
        assert_eq!(a.assigned_cpu_segments, 2);
        assert_eq!(a.unassigned_cpu_segments, 0);
        let value = serde_json::to_value(a).unwrap();
        for key in ["cpu_work_ns", "execution_span_ns", "parallelism"] {
            assert!(value[key].is_null());
        }
        assert_eq!(value["flow_complete"], false);
        assert_eq!(value["full_pipeline_complete"], false);
        assert!(value.get("work_span_ratio").is_none());
    }

    #[test]
    fn foreign_missing_or_incomplete_query_authority_cannot_weight_query_cpu() {
        let t = timing();
        let mut foreign = authority(&t);
        foreign.capture_id = "other-query".into();
        for q in [None, Some(&foreign)] {
            let a = collect(fixture(), &t, q).unwrap();
            assert!(!a.records_complete);
            assert_eq!(a.known_cpu_work_ns, Some(3));
            assert_eq!(a.longest_known_chain_path, vec![4]);
        }
        let mut q = authority(&t);
        q.complete = false;
        assert_eq!(
            collect(fixture(), &t, Some(&q)).unwrap().known_cpu_work_ns,
            Some(3)
        );
        q = authority(&t);
        q.clock.as_mut().unwrap().pool_observer = 2;
        assert_eq!(
            collect(fixture(), &t, Some(&q)).unwrap().known_cpu_work_ns,
            Some(3)
        );
    }

    #[test]
    fn original_references_lifecycles_categories_and_required_nulls_are_strict() {
        let t = timing();
        let q = authority(&t);
        for (pointer, replacement) in [
            ("/nodes/3/segment_id", json!(1)),
            ("/nodes/1/segment_id", json!(3)),
            ("/nodes/1/scope_id", Value::Null),
            ("/nodes/1/task", Value::Null),
            ("/nodes/1/outcome", json!("returned")),
            ("/nodes/0/gap_reason", json!("dispatch")),
            ("/scopes/0/query_job", json!(3)),
            ("/scopes/0/query_name", json!("path/private")),
            ("/scopes/0/entry", json!(2)),
            ("/scopes/0/exit", json!(1)),
            ("/scopes/0/outcome", json!("unwound")),
            ("/edges/1/to", json!(1)),
            ("/edges/0/id", json!(2)),
            ("/node_allocator_baseline", json!(0)),
            ("/expected_nodes", json!(5)),
            ("/flow_complete", json!(true)),
        ] {
            let mut v = fixture();
            *v.pointer_mut(pointer).unwrap() = replacement;
            assert!(collect(v, &t, Some(&q)).is_err(), "accepted {pointer}");
        }
        let mut v = fixture();
        v["nodes"][1].as_object_mut().unwrap().remove("scope_id");
        assert!(collect(v, &t, Some(&q)).is_err());
        let raw = serde_json::to_string(&fixture()).unwrap().replacen(
            "\"schema\":1",
            "\"schema\":1,\"schema\":1",
            1,
        );
        assert!(serde_json::from_str::<FlowCapture>(&raw).is_err());
        let mut v = fixture();
        v["nodes"][0]["unknown"] = json!(1);
        assert!(collect(v, &t, Some(&q)).is_err());
    }

    #[test]
    fn honest_lost_nodes_are_unknown_but_out_of_range_endpoints_are_forgery() {
        let t = timing();
        let q = authority(&t);
        let mut v = fixture();
        v["nodes"].as_array_mut().unwrap().remove(1);
        v["lost_nodes"] = json!(1);
        v["records_complete"] = json!(false);
        let a = collect(v.clone(), &t, Some(&q)).unwrap();
        assert!(!a.records_complete);
        assert_eq!(a.known_cpu_work_ns, Some(3));
        assert_eq!(a.unassigned_cpu_segments, 1);
        assert_eq!(a.unassigned_known_cpu_ns, Some(6));
        v["edges"][0]["to"] = json!(99);
        assert!(collect(v, &t, Some(&q)).is_err());
    }

    #[test]
    fn failed_measurement_and_explicit_cut_do_not_connect_known_cpu() {
        let mut t = timing();
        let q = authority(&t);
        t.validated_segments[0].measured = false;
        t.validated_segments[0].cpu_ns = None;
        let a = collect(fixture(), &t, Some(&q)).unwrap();
        assert_eq!(a.known_cpu_work_ns, Some(3));
        assert_eq!(a.longest_known_chain_path, vec![4]);
        assert_eq!(a.unavailable_cpu_segments, 1);
        let mut v = fixture();
        v["nodes"][0]["kind"] = json!("cut");
        v["nodes"][0]["gap_reason"] = json!("dispatch");
        assert!(
            collect(v, &t, Some(&q)).is_err(),
            "a cut cannot masquerade as a scope entry"
        );
    }

    fn two_scopes(nested: bool) -> (Value, crate::execution_graph_timing::ObservedCpuAssessment) {
        let mut t = timing();
        let second = &mut t.validated_segments[1];
        second.task = Some(1);
        second.query_context = Some(1);
        second.query_job = Some(if nested { 2 } else { 1 });
        second.query_name = Some("type_of".into());
        let mut v = fixture();
        v["node_allocator_high_water"] = json!(7);
        v["expected_nodes"] = json!(6);
        v["scope_allocator_high_water"] = json!(3);
        v["expected_scopes"] = json!(2);
        v["nodes"].as_array_mut().unwrap().pop();
        v["nodes"].as_array_mut().unwrap().extend([
            json!({"id":4,"at_ns":20,"kind":"scope_enter","segment_id":null,"scope_id":2,"task":1,"outcome":null,"gap_reason":null}),
            json!({"id":5,"at_ns":20,"kind":"cpu_segment","segment_id":2,"scope_id":2,"task":1,"outcome":null,"gap_reason":null}),
            json!({"id":6,"at_ns":30,"kind":"scope_exit","segment_id":null,"scope_id":2,"task":1,"outcome":"returned","gap_reason":null}),
        ]);
        v["scopes"].as_array_mut().unwrap().push(json!({"id":2,"query_context":1,"query_job":if nested {2} else {1},"query_name":"type_of","entry":4,"exit":6,"outcome":"returned"}));
        let edges = if nested {
            v["nodes"][2]["at_ns"] = json!(40);
            json!([
                {"id":1,"from":1,"to":2,"kind":"strand"},
                {"id":2,"from":2,"to":4,"kind":"sync_call"},
                {"id":3,"from":4,"to":5,"kind":"strand"},
                {"id":4,"from":5,"to":6,"kind":"strand"},
                {"id":5,"from":6,"to":3,"kind":"sync_return"}])
        } else {
            json!([
                {"id":1,"from":1,"to":2,"kind":"strand"},
                {"id":2,"from":2,"to":3,"kind":"strand"},
                {"id":3,"from":4,"to":5,"kind":"strand"},
                {"id":4,"from":5,"to":6,"kind":"strand"}])
        };
        v["expected_edges"] = json!(edges.as_array().unwrap().len());
        v["edge_allocator_high_water"] = json!(edges.as_array().unwrap().len() + 1);
        v["edges"] = edges;
        (v, t)
    }

    #[test]
    fn fresh_scopes_allow_same_job_reentry_without_inventing_order() {
        let (v, t) = two_scopes(false);
        let q = authority(&t);
        let a = collect(v.clone(), &t, Some(&q)).unwrap();
        assert_eq!(a.known_cpu_work_ns, Some(9));
        assert_eq!(a.longest_known_chain_cpu_ns, Some(6));
        let mut reused = v.clone();
        reused["scopes"][1]["id"] = json!(1);
        assert!(collect(reused, &t, Some(&q)).is_err());
        let mut wrong_invocation = v;
        wrong_invocation["nodes"][4]["scope_id"] = json!(1);
        assert!(collect(wrong_invocation, &t, Some(&q)).is_err());
    }

    #[test]
    fn actual_nested_call_and_return_chain_and_unwind_are_supported() {
        let (mut v, t) = two_scopes(true);
        let q = authority(&t);
        let a = collect(v.clone(), &t, Some(&q)).unwrap();
        assert_eq!(a.longest_known_chain_cpu_ns, Some(9));
        assert!(a.longest_known_chain_path.windows(2).any(|w| w == [2, 4]));
        v["nodes"][5]["outcome"] = json!("unwound");
        v["scopes"][1]["outcome"] = json!("unwound");
        assert_eq!(
            collect(v, &t, Some(&q)).unwrap().longest_known_chain_cpu_ns,
            Some(9)
        );
    }

    #[test]
    fn full_original_cpu_end_and_exact_scope_lifetime_bound_transitions() {
        let (v, t) = two_scopes(true);
        let q = authority(&t);
        let mut early_call = v;
        early_call["nodes"][3]["at_ns"] = json!(15);
        assert!(
            collect(early_call, &t, Some(&q)).is_err(),
            "gate mid-CPU is not after its source"
        );
        let mut early_exit = fixture();
        early_exit["nodes"][2]["at_ns"] = json!(15);
        assert!(collect(early_exit, &t, Some(&q)).is_err());
        let mut outside_admission = fixture();
        outside_admission["nodes"][0]["at_ns"] = json!(0);
        assert!(collect(outside_admission, &t, Some(&q)).is_err());
    }

    #[test]
    fn deep_chain_uses_iterative_scalar_predecessors() {
        let nodes = (1..=10_000).map(|id| (id, Cost::Measured(1))).collect();
        let edges = (1..10_000).map(|id| (id, id + 1)).collect();
        let a = known_chain(&nodes, &edges).unwrap().unwrap();
        assert_eq!(a.cpu_ns, 10_000);
        assert_eq!(a.path.len(), 10_000);
        assert_eq!(a.path.first(), Some(&1));
        assert_eq!(a.path.last(), Some(&10_000));
    }

    #[test]
    fn return_must_rejoin_the_actual_callers_invocation() {
        let (mut v, t) = two_scopes(true);
        v["nodes"].as_array_mut().unwrap().extend([
            json!({"id":7,"at_ns":30,"kind":"scope_enter","segment_id":null,"scope_id":3,"task":null,"outcome":null,"gap_reason":null}),
            json!({"id":8,"at_ns":40,"kind":"scope_exit","segment_id":null,"scope_id":3,"task":null,"outcome":"returned","gap_reason":null}),
        ]);
        v["scopes"].as_array_mut().unwrap().push(json!({"id":3,"query_context":1,"query_job":1,"query_name":"type_of","entry":7,"exit":8,"outcome":"returned"}));
        v["expected_nodes"] = json!(8);
        v["node_allocator_high_water"] = json!(9);
        v["expected_scopes"] = json!(3);
        v["scope_allocator_high_water"] = json!(4);
        v["edges"][4]["to"] = json!(8);
        let error = collect(v, &t, Some(&authority(&t)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("different caller"), "{error}");
    }

    #[test]
    fn complete_called_scope_requires_return_but_honest_edge_loss_is_partial() {
        let (mut v, t) = two_scopes(true);
        let q = authority(&t);
        v["edges"].as_array_mut().unwrap().pop();
        v["edge_allocator_high_water"] = json!(5);
        v["expected_edges"] = json!(4);
        assert!(
            collect(v.clone(), &t, Some(&q))
                .unwrap_err()
                .to_string()
                .contains("omits reciprocal return")
        );
        v["edge_allocator_high_water"] = json!(6);
        v["expected_edges"] = json!(5);
        v["lost_edges"] = json!(1);
        v["records_complete"] = json!(false);
        assert!(!collect(v, &t, Some(&q)).unwrap().records_complete);
    }

    #[test]
    fn continuation_destination_cannot_change_task_or_scope() {
        let node = |kind, task, scope| Node {
            id: 1,
            at_ns: 1,
            kind,
            segment_id: (kind == NodeKind::CpuSegment).then_some(1),
            scope_id: scope,
            task,
            outcome: None,
            gap_reason: None,
        };
        let join = node(NodeKind::Join, Some(4), Some(1));
        let cpu = node(NodeKind::CpuSegment, Some(4), Some(1));
        assert!(validate_edge(EdgeKind::Continuation, &join, &cpu).is_ok());
        assert!(
            validate_edge(
                EdgeKind::Continuation,
                &join,
                &node(NodeKind::CpuSegment, Some(5), Some(1))
            )
            .is_err()
        );
        assert!(
            validate_edge(
                EdgeKind::Continuation,
                &join,
                &node(NodeKind::CpuSegment, Some(4), Some(2))
            )
            .is_err()
        );
        let cut = node(NodeKind::Cut, None, Some(1));
        assert!(validate_edge(EdgeKind::Continuation, &cut, &cpu).is_ok());
        let before = node(NodeKind::CpuSegment, Some(1), Some(1));
        assert!(validate_edge(EdgeKind::Strand, &before, &cut).is_ok());
        let costs = BTreeMap::from([
            (1, Cost::Measured(7)),
            (2, Cost::Unknown),
            (3, Cost::Measured(11)),
        ]);
        assert_eq!(
            known_chain(&costs, &BTreeSet::from([(1, 2), (2, 3)]))
                .unwrap()
                .unwrap()
                .cpu_ns,
            11
        );
    }

    #[test]
    fn logical_tls_or_identity_failure_never_certifies_a_chain() {
        let t = timing();
        for field in ["tls_failed", "identity_exhausted"] {
            let mut v = fixture();
            v[field] = json!(true);
            v["records_complete"] = json!(false);
            let a = collect(v, &t, Some(&authority(&t))).unwrap();
            assert!(!a.records_complete);
            assert_eq!(a.known_cpu_work_ns, None);
            assert_eq!(a.longest_known_chain_cpu_ns, None);
            assert!(a.longest_known_chain_steps.is_empty());
        }
    }

    #[test]
    fn honest_missing_scope_terminal_is_not_invented_from_the_job_lifetime() {
        let t = timing();
        let mut v = fixture();
        v["nodes"].as_array_mut().unwrap().remove(2);
        v["lost_nodes"] = json!(1);
        v["records_complete"] = json!(false);
        let a = collect(v, &t, Some(&authority(&t))).unwrap();
        assert!(!a.records_complete);
        assert_eq!(a.known_cpu_work_ns, Some(3));
        assert_eq!(a.longest_known_chain_path, vec![4]);
    }

    #[test]
    fn scope_can_span_fork_to_new_continuation_without_serializing_siblings() {
        use crate::execution_graph_continuation::{TaskEdge, TaskInterval};
        let mut t = timing();
        for (i, row) in t.validated_segments.iter_mut().enumerate() {
            row.task = Some(i as u64 + 2);
            row.query_context = None;
            row.query_job = None;
            row.query_name = None;
        }
        let mut last = t.validated_segments[0].clone();
        last.segment_id = 3;
        last.task = Some(4);
        last.query_context = Some(1);
        last.query_job = Some(1);
        last.query_name = Some("type_of".into());
        last.start_ns = Some(35);
        last.end_ns = Some(40);
        last.cpu_ns = Some(2);
        t.validated_segments.push(last);
        t.segment_allocator_high_water = Some(4);
        let mut v = fixture();
        v["nodes"] = json!([
            {"id":1,"at_ns":5,"kind":"fork","segment_id":null,"scope_id":1,"task":1,"outcome":null,"gap_reason":null},
            {"id":2,"at_ns":5,"kind":"task_enter","segment_id":null,"scope_id":null,"task":2,"outcome":null,"gap_reason":null},
            {"id":3,"at_ns":10,"kind":"cpu_segment","segment_id":1,"scope_id":null,"task":2,"outcome":null,"gap_reason":null},
            {"id":4,"at_ns":25,"kind":"task_exit","segment_id":null,"scope_id":null,"task":2,"outcome":"returned","gap_reason":null},
            {"id":5,"at_ns":5,"kind":"task_enter","segment_id":null,"scope_id":null,"task":3,"outcome":null,"gap_reason":null},
            {"id":6,"at_ns":20,"kind":"cpu_segment","segment_id":2,"scope_id":null,"task":3,"outcome":null,"gap_reason":null},
            {"id":7,"at_ns":35,"kind":"task_exit","segment_id":null,"scope_id":null,"task":3,"outcome":"returned","gap_reason":null},
            {"id":8,"at_ns":35,"kind":"join","segment_id":null,"scope_id":1,"task":4,"outcome":null,"gap_reason":null},
            {"id":9,"at_ns":1,"kind":"scope_enter","segment_id":null,"scope_id":1,"task":null,"outcome":null,"gap_reason":null},
            {"id":10,"at_ns":40,"kind":"scope_exit","segment_id":null,"scope_id":1,"task":null,"outcome":"returned","gap_reason":null},
            {"id":11,"at_ns":35,"kind":"cpu_segment","segment_id":3,"scope_id":1,"task":4,"outcome":null,"gap_reason":null}]);
        v["scopes"][0]["entry"] = json!(9);
        v["scopes"][0]["exit"] = json!(10);
        let edge_pairs = [
            (9, 1, "strand"),
            (1, 2, "fork_branch"),
            (1, 5, "fork_branch"),
            (2, 3, "strand"),
            (3, 4, "strand"),
            (5, 6, "strand"),
            (6, 7, "strand"),
            (4, 8, "join_required"),
            (7, 8, "join_required"),
            (8, 11, "continuation"),
            (11, 10, "strand"),
        ];
        v["edges"] = json!(
            edge_pairs
                .iter()
                .enumerate()
                .map(|(i, &(from, to, kind))| json!({"id":i+1,"from":from,"to":to,"kind":kind}))
                .collect::<Vec<_>>()
        );
        v["expected_nodes"] = json!(11);
        v["node_allocator_high_water"] = json!(12);
        v["expected_edges"] = json!(11);
        v["edge_allocator_high_water"] = json!(12);
        let tasks = BTreeMap::from([
            (
                1,
                TaskInterval {
                    start_ns: 1,
                    finish_ns: 5,
                },
            ),
            (
                2,
                TaskInterval {
                    start_ns: 5,
                    finish_ns: 25,
                },
            ),
            (
                3,
                TaskInterval {
                    start_ns: 5,
                    finish_ns: 35,
                },
            ),
            (
                4,
                TaskInterval {
                    start_ns: 35,
                    finish_ns: 100,
                },
            ),
        ]);
        let task_edges = [
            TaskEdge {
                from: 1,
                to: 2,
                kind: "fork",
            },
            TaskEdge {
                from: 1,
                to: 3,
                kind: "fork",
            },
            TaskEdge {
                from: 2,
                to: 4,
                kind: "join",
            },
            TaskEdge {
                from: 3,
                to: 4,
                kind: "join",
            },
        ];
        let q = authority(&t);
        let task_outcomes =
            BTreeMap::from([(1, "normal"), (2, "normal"), (3, "normal"), (4, "normal")]);
        let run = |value| {
            assess(
                serde_json::from_value(value).unwrap(),
                FlowContext {
                    pool_capture_id: "pool-test",
                    query_capture_id: "query_123-safe",
                    observer: 1,
                    pool_sealed: true,
                    structural_complete: true,
                    timing: &t,
                    query_inventory: Some(&q),
                    clock_aligned: true,
                    task_high_water: 5,
                    task_intervals: &tasks,
                    task_edges: &task_edges,
                    task_outcomes: &task_outcomes,
                },
            )
        };
        let a = run(v.clone()).unwrap();
        assert!(a.records_complete);
        assert_eq!(a.known_cpu_work_ns, Some(11));
        assert_eq!(a.longest_known_chain_cpu_ns, Some(8));
        assert!(a.issues.is_empty());
        assert!(
            a.longest_known_chain_steps
                .iter()
                .any(|s| s.query_name.as_deref() == Some("type_of") && s.cpu_ns == Some(2))
        );
        let mut duplicate = v.clone();
        let mut wrong_outcome = v.clone();
        wrong_outcome["nodes"][3]["outcome"] = json!("unwound");
        assert!(
            run(wrong_outcome)
                .unwrap_err()
                .to_string()
                .contains("original normal terminal")
        );
        let mut extra = duplicate["nodes"][1].clone();
        extra["id"] = json!(12);
        duplicate["nodes"].as_array_mut().unwrap().push(extra);
        duplicate["expected_nodes"] = json!(12);
        duplicate["node_allocator_high_water"] = json!(13);
        assert!(
            run(duplicate)
                .unwrap_err()
                .to_string()
                .contains("duplicate flow lifecycle role")
        );
        v["edges"].as_array_mut().unwrap().remove(7);
        for (i, edge) in v["edges"].as_array_mut().unwrap().iter_mut().enumerate() {
            edge["id"] = json!(i + 1);
        }
        v["expected_edges"] = json!(10);
        v["edge_allocator_high_water"] = json!(11);
        assert!(run(v).unwrap_err().to_string().contains("omits original"));
    }

    #[test]
    fn scope_gate_clock_mapping_is_checked_and_loss_terminal_requires_evidence() {
        let mut t = timing();
        t.clock_bridge.pool_epoch_in_query_ns = 100;
        let mut q = authority(&t);
        q.lifetimes = BTreeMap::from([((1, 1), (101, 200))]);
        assert!(collect(fixture(), &t, Some(&q)).unwrap().records_complete);
        q.lifetimes.insert((1, 1), (102, 200));
        assert!(collect(fixture(), &t, Some(&q)).is_err());
        t.clock_bridge.pool_epoch_in_query_ns = u64::MAX;
        q = authority(&t);
        assert!(collect(fixture(), &t, Some(&q)).is_err());
        let t = timing();
        let q = authority(&t);
        let mut v = fixture();
        v["nodes"].as_array_mut().unwrap().remove(2);
        v["lost_nodes"] = json!(1);
        v["records_complete"] = json!(false);
        v["scopes"][0]["exit"] = Value::Null;
        assert!(collect(v.clone(), &t, Some(&q)).is_err());
        v["identity_exhausted"] = json!(true);
        let a = collect(v, &t, Some(&q)).unwrap();
        assert_eq!(a.longest_known_chain_cpu_ns, None);
        assert_eq!(a.known_cpu_work_ns, None);
    }

    #[test]
    fn sibling_cpu_is_not_serialized_and_ids_do_not_imply_order() {
        let nodes = BTreeMap::from([
            (90, Cost::Gate),
            (40, Cost::Measured(7)),
            (20, Cost::Measured(11)),
            (1, Cost::Gate),
        ]);
        let edges = BTreeSet::from([(90, 40), (90, 20), (40, 1), (20, 1)]);
        let chain = known_chain(&nodes, &edges).unwrap().unwrap();
        assert_eq!(chain.cpu_ns, 11);
        assert!(chain.path == vec![90, 20] || chain.path == vec![90, 20, 1]);
    }

    #[test]
    fn unknown_cpu_breaks_a_chain_and_is_not_a_zero_gate() {
        let nodes = BTreeMap::from([
            (1, Cost::Measured(7)),
            (2, Cost::Unknown),
            (3, Cost::Measured(11)),
        ]);
        let edges = BTreeSet::from([(1, 2), (2, 3)]);
        assert_eq!(
            known_chain(&nodes, &edges).unwrap(),
            Some(KnownChain {
                cpu_ns: 11,
                path: vec![3]
            })
        );
        assert_eq!(
            known_chain(&BTreeMap::from([(1, Cost::Gate)]), &BTreeSet::new()).unwrap(),
            None
        );
    }

    #[test]
    fn unknown_nodes_do_not_hide_cycles_or_forged_endpoints() {
        let nodes = BTreeMap::from([(1, Cost::Unknown), (2, Cost::Gate)]);
        assert!(known_chain(&nodes, &BTreeSet::from([(1, 2), (2, 1)])).is_err());
        assert!(known_chain(&nodes, &BTreeSet::from([(1, 3)])).is_err());
        assert!(known_chain(&nodes, &BTreeSet::from([(1, 1)])).is_err());
    }

    #[test]
    fn known_chain_sum_is_checked() {
        let nodes = BTreeMap::from([(1, Cost::Measured(u64::MAX)), (2, Cost::Measured(1))]);
        assert!(known_chain(&nodes, &BTreeSet::from([(1, 2)])).is_err());
    }
}
