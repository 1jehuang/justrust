//! Strict latch readiness provenance, independently admitted from execution flow v1.
//! Publication precursors and wake intervals are unknown barriers, never full span.
use crate::execution_graph_flow::{
    self as flow, Edge, EdgeKind, FlowAssessment, FlowCapture, FlowContext, GapReason, Node,
    NodeKind, ReadinessProof, Scope,
};
use crate::execution_graph_query::{OriginalWaitAuthority, OriginalWaitReason, QueryTerminal};
use anyhow::{Result, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_ROWS: usize = 100_000;
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

// Deliberately repeat the uniform header instead of flattening deserialization:
// original duplicate/unknown fields must be rejected before authority is used.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadinessCapture {
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
    readiness_allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    readiness_allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_readiness: Option<u64>,
    lost_readiness: u64,
    active_readiness: u64,
    readiness: Vec<ReadyRow>,
    wait_attempt_allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    wait_attempt_allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_wait_attempts: Option<u64>,
    lost_wait_attempts: u64,
    active_wait_attempts: u64,
    wait_attempts: Vec<WaitRow>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ReadyOutcome {
    Active,
    Published,
    Poisoned,
    Aborted,
    Unavailable,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadyRow {
    id: u64,
    query_context: u64,
    query_job: u64,
    #[serde(deserialize_with = "nullable")]
    prefix: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    prerequisite: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    visibility_gap: Option<u64>,
    outcome: ReadyOutcome,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WaitMode {
    Active,
    Registered,
    AlreadyReady,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WaitOutcome {
    Active,
    Consumed,
    ConsumedUnbound,
    Poisoned,
    Cycle,
    Unwound,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitRow {
    id: u64,
    query_context: u64,
    waitee_job: u64,
    #[serde(deserialize_with = "nullable")]
    consumer_job: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    caller_scope: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    wait_id: Option<u64>,
    mode: WaitMode,
    outcome: WaitOutcome,
    #[serde(deserialize_with = "nullable")]
    begin: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    wait_gap: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    readiness: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    wake_gap: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    consumed: Option<u64>,
}

pub(crate) struct ReadinessContext<'a> {
    pub flow: FlowContext<'a>,
    pub waits: Option<&'a OriginalWaitAuthority>,
}
#[derive(Debug, Serialize)]
pub(crate) struct ReadinessAssessment {
    #[serde(flatten)]
    pub flow: FlowAssessment,
    pub ready_records_complete: bool,
    pub wait_records_complete: bool,
    pub readiness_rows: usize,
    pub active_readiness: u64,
    pub lost_readiness: u64,
    pub wait_attempts: usize,
    pub active_wait_attempts: u64,
    pub lost_wait_attempts: u64,
    pub matched_readiness_transfers: usize,
    pub unbound_wait_attempts: usize,
    pub unavailable_readiness_references: usize,
}
impl ReadinessAssessment {
    pub(crate) fn unavailable(
        pool_capture_id: &str,
        observer: u64,
        reason: impl Into<String>,
    ) -> Self {
        let mut flow = FlowAssessment::unavailable(pool_capture_id, observer, reason);
        flow.scope = "observed-query-join-and-latch-control-flow";
        Self {
            flow,
            ready_records_complete: false,
            wait_records_complete: false,
            readiness_rows: 0,
            active_readiness: 0,
            lost_readiness: 0,
            wait_attempts: 0,
            active_wait_attempts: 0,
            lost_wait_attempts: 0,
            matched_readiness_transfers: 0,
            unbound_wait_attempts: 0,
            unavailable_readiness_references: 0,
        }
    }
}

/// Validate exact row-bound facts before admitting any new control edge to the
/// common DAG. This never lowers a readiness obligation into guessed v1 ancestry.
pub(crate) fn assess(
    c: ReadinessCapture,
    context: ReadinessContext<'_>,
) -> Result<ReadinessAssessment> {
    ensure!(
        c.schema == 2 && c.scope == "observed-query-join-and-latch-control-flow",
        "unsupported latch readiness schema or scope"
    );
    ensure!(
        c.readiness.len() <= MAX_ROWS && c.wait_attempts.len() <= MAX_ROWS,
        "readiness inventory exceeds independent bounded budget"
    );
    let f = FlowCapture {
        schema: c.schema,
        scope: c.scope,
        full_pipeline_complete: c.full_pipeline_complete,
        flow_complete: c.flow_complete,
        sealed: c.sealed,
        records_complete: c.records_complete,
        identity_exhausted: c.identity_exhausted,
        tls_failed: c.tls_failed,
        active_scopes: c.active_scopes,
        node_allocator_baseline: c.node_allocator_baseline,
        node_allocator_high_water: c.node_allocator_high_water,
        expected_nodes: c.expected_nodes,
        edge_allocator_baseline: c.edge_allocator_baseline,
        edge_allocator_high_water: c.edge_allocator_high_water,
        expected_edges: c.expected_edges,
        scope_allocator_baseline: c.scope_allocator_baseline,
        scope_allocator_high_water: c.scope_allocator_high_water,
        expected_scopes: c.expected_scopes,
        lost_nodes: c.lost_nodes,
        lost_edges: c.lost_edges,
        lost_scopes: c.lost_scopes,
        loss_overflowed: c.loss_overflowed,
        nodes: c.nodes,
        edges: c.edges,
        scopes: c.scopes,
    };
    let (_, ready_inventory_complete) = flow::inventory(
        c.readiness_allocator_baseline,
        c.readiness_allocator_high_water,
        c.expected_readiness,
        c.readiness.iter().map(|r| r.id),
        c.lost_readiness,
        c.identity_exhausted,
        c.loss_overflowed,
    )?;
    let (_, wait_inventory_complete) = flow::inventory(
        c.wait_attempt_allocator_baseline,
        c.wait_attempt_allocator_high_water,
        c.expected_wait_attempts,
        c.wait_attempts.iter().map(|r| r.id),
        c.lost_wait_attempts,
        c.identity_exhausted,
        c.loss_overflowed,
    )?;
    let ready_active = c
        .readiness
        .iter()
        .filter(|r| r.outcome == ReadyOutcome::Active)
        .count() as u64;
    let wait_active = c
        .wait_attempts
        .iter()
        .filter(|r| r.outcome == WaitOutcome::Active)
        .count() as u64;
    ensure!(
        ready_active <= c.active_readiness && wait_active <= c.active_wait_attempts,
        "readiness active counter undercounts original rows"
    );
    if ready_inventory_complete {
        ensure!(
            ready_active == c.active_readiness,
            "readiness active inventory mismatch"
        );
    }
    if wait_inventory_complete {
        ensure!(
            wait_active == c.active_wait_attempts,
            "wait attempt active inventory mismatch"
        );
    }
    let local_ready =
        ready_inventory_complete && c.active_readiness == 0 && c.sealed && !c.tls_failed;
    let local_wait =
        wait_inventory_complete && c.active_wait_attempts == 0 && c.sealed && !c.tls_failed;
    ensure!(
        !c.records_complete || (local_ready && local_wait),
        "false complete readiness inventory"
    );
    let nodes: BTreeMap<_, _> = f.nodes.iter().map(|n| (n.id, n)).collect();
    let scopes: BTreeMap<_, _> = f.scopes.iter().map(|s| (s.id, s)).collect();
    ensure!(
        nodes.len() == f.nodes.len() && scopes.len() == f.scopes.len(),
        "duplicate readiness original node or scope identity"
    );
    let ready: BTreeMap<_, _> = c.readiness.iter().map(|r| (r.id, r)).collect();
    let query = context.flow.query_inventory.filter(|q| {
        q.complete
            && q.capture_id == context.flow.query_capture_id
            && q.clock.as_ref() == Some(&context.flow.timing.clock_bridge)
            && context.flow.clock_aligned
    });
    let original_waits = context
        .waits
        .filter(|w| w.capture_id == context.flow.query_capture_id);
    let waits = original_waits.filter(|w| w.records_complete && query.is_some());
    if let Some(w) = waits {
        ensure!(
            w.expected_waits == Some(w.waits.len() as u64) && w.lost_events == 0,
            "complete original wait inventory has inconsistent coverage"
        );
    }
    let mut checker = Checker {
        flow: &f,
        nodes: &nodes,
        expected: BTreeMap::new(),
        claimed: BTreeSet::new(),
        proof: ReadinessProof::default(),
        unavailable: 0,
        incoming: f.edges.iter().map(|e| e.to).collect(),
        lost_ready: c.lost_readiness > 0 || c.identity_exhausted || c.loss_overflowed,
        lost_wait: c.lost_wait_attempts > 0 || c.identity_exhausted || c.loss_overflowed,
    };
    let mut ready_authority = BTreeMap::new();
    for row in &c.readiness {
        ensure!(
            row.query_context != 0 && row.query_job != 0,
            "invalid readiness producer identity"
        );
        let prefix = checker.node(row.prefix, None, None, None, false)?;
        let prerequisite = checker.node(
            row.prerequisite,
            Some(NodeKind::ReadinessPrerequisite),
            None,
            Some(None),
            true,
        )?;
        let visibility = checker.node(
            row.visibility_gap,
            Some(NodeKind::Cut),
            Some(GapReason::LatchVisibility),
            Some(None),
            true,
        )?;
        if let Some(node) = prerequisite {
            ensure!(
                node.task.is_none(),
                "readiness prerequisite fabricates task owner"
            );
        }
        if let Some(node) = visibility {
            ensure!(
                node.task.is_none(),
                "readiness visibility fabricates task owner"
            );
        }
        checker.obligation(
            row.prefix,
            row.prerequisite,
            EdgeKind::ReadinessPrepare,
            true,
        )?;
        checker.obligation(
            row.prerequisite,
            row.visibility_gap,
            EdgeKind::Strand,
            false,
        )?;
        if row.outcome != ReadyOutcome::Active && row.outcome != ReadyOutcome::Unavailable {
            checker.required(row.prerequisite)?;
            checker.required(row.visibility_gap)?;
        }
        let mut authorized = query.is_some()
            && prerequisite.is_some()
            && visibility.is_some()
            && row.outcome == ReadyOutcome::Published
            && !c.identity_exhausted
            && !c.tls_failed;
        if let Some(q) = query {
            let &(start, end) = q
                .lifetimes
                .get(&(row.query_context, row.query_job))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "readiness producer is not an original same-capture admitted job"
                    )
                })?;
            let terminal = match row.outcome {
                ReadyOutcome::Published => Some(QueryTerminal::Completed),
                ReadyOutcome::Poisoned => Some(QueryTerminal::Poisoned),
                _ => None,
            };
            if let Some(terminal) = terminal {
                ensure!(
                    q.terminals.get(&(row.query_context, row.query_job)) == Some(&terminal),
                    "readiness outcome contradicts original producer terminal"
                );
            }
            for node in [prerequisite, visibility].into_iter().flatten() {
                let at = mapped(
                    node.at_ns,
                    context.flow.timing.clock_bridge.pool_epoch_in_query_ns,
                )?;
                ensure!(
                    start <= at && at <= end,
                    "readiness precursor outside producer admission lifetime"
                );
            }
        } else {
            authorized = false;
        }
        if let (Some(prefix), Some(prerequisite)) = (prefix, prerequisite) {
            ensure!(
                prefix.at_ns <= prerequisite.at_ns,
                "readiness preparation precedes saved prefix"
            );
        }
        ready_authority.insert(row.id, authorized);
    }
    let mut original_wait_ids = BTreeSet::new();
    let mut matched_candidates = Vec::new();
    let mut unbound = 0usize;
    let mut wait_authority_complete = true;
    for row in &c.wait_attempts {
        ensure!(
            row.query_context != 0 && row.waitee_job != 0 && row.consumer_job != Some(0),
            "invalid wait attempt query identity"
        );
        ensure!(
            (row.mode == WaitMode::Registered) || row.wait_id.is_none(),
            "nonregistered encounter fabricates original wait identity"
        );
        ensure!(
            row.mode != WaitMode::Active
                || matches!(row.outcome, WaitOutcome::Active | WaitOutcome::Unwound),
            "terminal wait attempt retains active mode"
        );
        ensure!(
            row.mode != WaitMode::Registered
                || row.outcome != WaitOutcome::Consumed
                || row.wait_id.is_some(),
            "successful registered transfer lacks original wait identity"
        );
        if let Some(wait_id) = row.wait_id {
            ensure!(
                wait_id != 0 && original_wait_ids.insert(wait_id),
                "aliased original registered wait identity"
            );
        }
        let scope = if let Some(id) = row.caller_scope {
            flow::reference(
                id,
                f.scope_allocator_high_water,
                scopes.contains_key(&id),
                f.lost_scopes > 0 || f.identity_exhausted || f.loss_overflowed,
            )?;
            scopes.get(&id).copied()
        } else {
            None
        };
        if let Some(scope) = scope {
            ensure!(
                scope.query_context == row.query_context
                    && Some(scope.query_job) == row.consumer_job,
                "wait saved invocation differs from actual consumer owner"
            );
        }
        let begin = checker.node(
            row.begin,
            Some(NodeKind::WaitBegin),
            None,
            Some(row.caller_scope),
            true,
        )?;
        let wait_gap = checker.node(
            row.wait_gap,
            Some(NodeKind::Cut),
            Some(GapReason::QueryWait),
            Some(row.caller_scope),
            true,
        )?;
        let wake_reason = match row.outcome {
            WaitOutcome::Consumed | WaitOutcome::Active => GapReason::WakeToConsume,
            WaitOutcome::ConsumedUnbound => GapReason::ReadinessUnavailable,
            WaitOutcome::Poisoned => GapReason::WaitPoisoned,
            WaitOutcome::Cycle => GapReason::WaitCycle,
            WaitOutcome::Unwound => GapReason::WaitUnwound,
        };
        let wake = checker.node(
            row.wake_gap,
            Some(NodeKind::Cut),
            Some(wake_reason),
            Some(row.caller_scope),
            true,
        )?;
        let consumed = checker.node(
            row.consumed,
            Some(NodeKind::WaitConsumed),
            None,
            Some(row.caller_scope),
            true,
        )?;
        if let Some(node) = wait_gap {
            ensure!(
                node.task.is_none(),
                "independent query wait root fabricates task"
            );
        }
        if let Some(id) = row.wait_gap {
            ensure!(
                !checker.incoming.contains(&id),
                "query wait stream is not an independent root"
            );
        }
        checker.obligation(row.begin, row.wake_gap, EdgeKind::WaitBoundary, false)?;
        let ready_row = if let Some(id) = row.readiness {
            flow::reference(
                id,
                c.readiness_allocator_high_water,
                ready.contains_key(&id),
                c.lost_readiness > 0 || c.identity_exhausted || c.loss_overflowed,
            )?;
            ready.get(&id).copied()
        } else {
            None
        };
        if let Some(ready) = ready_row {
            ensure!(
                ready.query_context == row.query_context && ready.query_job == row.waitee_job,
                "diagnostic readiness belongs to another waitee"
            );
        }
        let mut qualified = query.is_some()
            && scope.is_some()
            && begin.is_some()
            && wake.is_some()
            && consumed.is_some()
            && wait_gap.is_some()
            && !c.identity_exhausted
            && !c.tls_failed;
        if let Some(q) = query {
            ensure!(
                q.lifetimes
                    .contains_key(&(row.query_context, row.waitee_job)),
                "wait waitee is not an original admitted query"
            );
            if let Some(consumer) = row.consumer_job {
                ensure!(
                    q.lifetimes.contains_key(&(row.query_context, consumer)),
                    "wait consumer is not an original admitted query"
                );
            }
        }
        if row.mode == WaitMode::Registered {
            if let (Some(id), Some(original)) = (row.wait_id, original_waits) {
                ensure!(
                    original.allocator_high_water.is_none_or(|high| id < high),
                    "registered wait outside original allocator"
                );
            }
            if let (Some(id), Some(waits)) = (row.wait_id, waits) {
                ensure!(
                    waits.allocator_high_water.is_none_or(|high| id < high),
                    "registered wait outside original allocator"
                );
                if let Some(original) = waits.waits.get(&id) {
                    ensure!(
                        original.context == row.query_context
                            && original.waitee == row.waitee_job
                            && original.consumer == row.consumer_job,
                        "wait attempt differs from original registered endpoints"
                    );
                    if matches!(
                        row.outcome,
                        WaitOutcome::Consumed | WaitOutcome::ConsumedUnbound
                    ) {
                        ensure!(
                            original.reason == Some(OriginalWaitReason::Completed),
                            "successful wait consumes poison or cycle notification"
                        );
                    }
                    if row.outcome == WaitOutcome::Poisoned {
                        ensure!(
                            original.reason == Some(OriginalWaitReason::Poisoned),
                            "poisoned wait contradicts original notification"
                        );
                    }
                    if row.outcome == WaitOutcome::Cycle {
                        ensure!(
                            original.reason == Some(OriginalWaitReason::Cycle),
                            "cycle wait contradicts original notification"
                        );
                    }
                    qualified &=
                        original.valid && original.reason == Some(OriginalWaitReason::Completed);
                    if let (Some(returned), Some(resumed)) =
                        (original.condvar_returned_ns, original.resumed_ns)
                    {
                        ensure!(
                            returned <= resumed,
                            "original allowance resumption precedes condvar return"
                        );
                    }
                    if let (Some(begin), Some(registered)) = (begin, original.registered_ns) {
                        ensure!(
                            mapped(
                                begin.at_ns,
                                context.flow.timing.clock_bridge.pool_epoch_in_query_ns
                            )? <= registered,
                            "wait attempt begins after original registration"
                        );
                    }
                    if let (Some(wake), Some(resumed)) = (wake, original.resumed_ns) {
                        ensure!(
                            resumed
                                <= mapped(
                                    wake.at_ns,
                                    context.flow.timing.clock_bridge.pool_epoch_in_query_ns
                                )?,
                            "wake barrier precedes actual allowance reacquisition"
                        );
                    }
                    if let (Some(ready), Some(notified)) = (ready_row, original.notified_ns) {
                        for id in [ready.prerequisite, ready.visibility_gap]
                            .into_iter()
                            .flatten()
                        {
                            if let Some(node) = nodes.get(&id) {
                                ensure!(
                                    mapped(
                                        node.at_ns,
                                        context.flow.timing.clock_bridge.pool_epoch_in_query_ns
                                    )? <= notified,
                                    "readiness precursor follows original completed notification"
                                );
                            }
                        }
                    }
                } else {
                    anyhow::bail!("registered wait absent from complete original inventory");
                }
            } else {
                qualified = false;
            }
        } else if row.mode != WaitMode::AlreadyReady {
            qualified = false;
        }
        if row.outcome == WaitOutcome::Consumed {
            checker.required(row.begin)?;
            checker.required(row.wait_gap)?;
            checker.required(row.wake_gap)?;
            checker.required(row.consumed)?;
            ensure!(
                row.readiness.is_some() || c.identity_exhausted,
                "consumed wait lacks readiness identity without exhaustion"
            );
            checker.obligation(row.wake_gap, row.consumed, EdgeKind::WaitRequired, false)?;
            if let Some(ready) = ready_row {
                ensure!(
                    ready.query_context == row.query_context && ready.query_job == row.waitee_job,
                    "consumed readiness belongs to another waitee"
                );
                ensure!(
                    ready.outcome == ReadyOutcome::Published,
                    "consumed wait uses unpublished readiness"
                );
                checker.obligation(
                    ready.visibility_gap,
                    row.consumed,
                    EdgeKind::ReadinessRequired,
                    true,
                )?;
                qualified &= ready_authority.get(&ready.id) == Some(&true);
            } else {
                qualified = false;
            }
            if let Some(id) = row.consumed {
                checker.proof.dual_nodes.insert(id);
            }
            if qualified {
                let ready = ready_row.expect("qualified readiness row");
                matched_candidates.push([
                    row.begin,
                    row.wait_gap,
                    row.wake_gap,
                    row.consumed,
                    ready.prefix,
                    ready.prerequisite,
                    ready.visibility_gap,
                ]);
            } else {
                wait_authority_complete = false;
                if let Some(id) = row.consumed {
                    checker.proof.unknown_nodes.insert(id);
                }
            }
        } else {
            ensure!(
                row.consumed.is_none(),
                "non-success encounter fabricates successful consumption gate"
            );
            if row.outcome == WaitOutcome::ConsumedUnbound {
                unbound += 1;
                wait_authority_complete = false;
            }
            if row.mode == WaitMode::Registered && row.wait_id.is_none() {
                wait_authority_complete = false;
            }
        }
        if let (Some(begin), Some(wake)) = (begin, wake) {
            ensure!(
                begin.at_ns <= wake.at_ns,
                "wake precedes exact wait encounter"
            );
        }
        if let (Some(wake), Some(consumed)) = (wake, consumed) {
            ensure!(
                wake.at_ns <= consumed.at_ns,
                "consumption precedes exact wake barrier"
            );
        }
    }
    let unmatched_original = waits.map_or(0, |waits| {
        waits
            .waits
            .keys()
            .filter(|id| !original_wait_ids.contains(id))
            .count()
    });
    if unmatched_original != 0 {
        wait_authority_complete = false;
    }
    checker.finish()?;
    let unavailable = checker.unavailable;
    let matched = matched_candidates
        .iter()
        .filter(|ids| {
            ids.iter()
                .flatten()
                .all(|id| !checker.proof.unknown_nodes.contains(id))
        })
        .count();
    wait_authority_complete &= matched == matched_candidates.len();
    let ready_records_complete = local_ready
        && query.is_some()
        && c.readiness
            .iter()
            .filter(|r| r.outcome == ReadyOutcome::Published)
            .all(|r| ready_authority.get(&r.id) == Some(&true));
    let wait_records_complete = local_wait && wait_authority_complete && waits.is_some();
    checker.proof.complete = ready_records_complete && wait_records_complete && unavailable == 0;
    let proof = checker.proof;
    let mut assessment = flow::assess_readiness(f, context.flow, &proof)?;
    if unmatched_original != 0 {
        assessment.issues.push(format!(
            "{unmatched_original} original registered waits lack readiness encounters"
        ));
    }
    assessment.limitations = vec!["latch publication visibility and wake-to-consume intervals are unknown barriers; other scheduler, I/O, pre-attach and uninstrumented boundaries remain outside this graph".into()];
    Ok(ReadinessAssessment {
        flow: assessment,
        ready_records_complete,
        wait_records_complete,
        readiness_rows: c.readiness.len(),
        active_readiness: c.active_readiness,
        lost_readiness: c.lost_readiness,
        wait_attempts: c.wait_attempts.len(),
        active_wait_attempts: c.active_wait_attempts,
        lost_wait_attempts: c.lost_wait_attempts,
        matched_readiness_transfers: matched,
        unbound_wait_attempts: unbound,
        unavailable_readiness_references: unavailable,
    })
}

fn mapped(at: u64, offset: u64) -> Result<u64> {
    at.checked_add(offset)
        .ok_or_else(|| anyhow::anyhow!("readiness query-clock mapping overflow"))
}

struct Checker<'a> {
    flow: &'a FlowCapture,
    nodes: &'a BTreeMap<u64, &'a Node>,
    expected: BTreeMap<(u64, u64, u8), bool>,
    claimed: BTreeSet<u64>,
    proof: ReadinessProof,
    unavailable: usize,
    incoming: BTreeSet<u64>,
    lost_ready: bool,
    lost_wait: bool,
}
fn role(kind: EdgeKind) -> u8 {
    kind as u8
}
fn wake_gap(reason: Option<GapReason>) -> bool {
    matches!(
        reason,
        Some(
            GapReason::WakeToConsume
                | GapReason::ReadinessUnavailable
                | GapReason::WaitCycle
                | GapReason::WaitPoisoned
                | GapReason::WaitUnwound
        )
    )
}
impl<'a> Checker<'a> {
    fn required(&mut self, id: Option<u64>) -> Result<()> {
        ensure!(
            id.is_some() || self.flow.identity_exhausted,
            "terminal readiness endpoint missing without checked exhaustion"
        );
        if id.is_none() {
            self.unavailable += 1;
        }
        Ok(())
    }
    fn node(
        &mut self,
        id: Option<u64>,
        kind: Option<NodeKind>,
        gap: Option<GapReason>,
        scope: Option<Option<u64>>,
        claim: bool,
    ) -> Result<Option<&'a Node>> {
        let Some(id) = id else { return Ok(None) };
        flow::reference(
            id,
            self.flow.node_allocator_high_water,
            self.nodes.contains_key(&id),
            self.flow.lost_nodes > 0 || self.flow.identity_exhausted || self.flow.loss_overflowed,
        )?;
        if claim {
            ensure!(
                self.claimed.insert(id),
                "readiness endpoints alias another original row"
            );
        }
        let Some(node) = self.nodes.get(&id).copied() else {
            self.unavailable += 1;
            return Ok(None);
        };
        if let Some(kind) = kind {
            ensure!(
                node.kind == kind,
                "readiness endpoint has incorrect gate kind"
            );
        }
        if kind.is_some() {
            ensure!(
                node.gap_reason == gap,
                "readiness endpoint has incorrect gap reason"
            );
        }
        if let Some(scope) = scope {
            ensure!(
                node.scope_id == scope,
                "readiness endpoint differs from saved invocation"
            );
        }
        Ok(Some(node))
    }
    fn obligation(
        &mut self,
        from: Option<u64>,
        to: Option<u64>,
        kind: EdgeKind,
        side: bool,
    ) -> Result<()> {
        if let (Some(from), Some(to)) = (from, to) {
            ensure!(
                self.expected.insert((from, to, role(kind)), side).is_none(),
                "aliased row-bound readiness obligation"
            );
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        for edge in &self.flow.edges {
            let key = (edge.from, edge.to, role(edge.kind));
            if let Some(side) = self.expected.remove(&key) {
                self.proof.edge_ids.insert(edge.id);
                if side {
                    self.proof.side_edge_ids.insert(edge.id);
                }
                continue;
            }
            let from = self.nodes.get(&edge.from).copied();
            let to = self.nodes.get(&edge.to).copied();
            if matches!(
                edge.kind,
                EdgeKind::ReadinessPrepare
                    | EdgeKind::WaitBoundary
                    | EdgeKind::WaitRequired
                    | EdgeKind::ReadinessRequired
            ) {
                let lost_binding = match edge.kind {
                    EdgeKind::ReadinessPrepare => {
                        self.lost_ready
                            && !self.claimed.contains(&edge.to)
                            && to.is_none_or(|n| {
                                n.kind == NodeKind::ReadinessPrerequisite
                                    && n.scope_id.is_none()
                                    && n.task.is_none()
                            })
                    }
                    EdgeKind::ReadinessRequired => {
                        ((self.lost_ready && !self.claimed.contains(&edge.from))
                            || (self.lost_wait && !self.claimed.contains(&edge.to)))
                            && to.is_none_or(|n| n.kind == NodeKind::WaitConsumed)
                            && from.is_none_or(|n| n.gap_reason == Some(GapReason::LatchVisibility))
                    }
                    EdgeKind::WaitBoundary | EdgeKind::WaitRequired => {
                        self.lost_wait
                            && !self.claimed.contains(&edge.from)
                            && !self.claimed.contains(&edge.to)
                            && match edge.kind {
                                EdgeKind::WaitBoundary => {
                                    from.is_none_or(|n| n.kind == NodeKind::WaitBegin)
                                        && to.is_none_or(|n| wake_gap(n.gap_reason))
                                }
                                _ => {
                                    from.is_none_or(|n| {
                                        n.gap_reason == Some(GapReason::WakeToConsume)
                                    }) && to.is_none_or(|n| n.kind == NodeKind::WaitConsumed)
                                }
                            }
                    }
                    _ => false,
                };
                ensure!(
                    lost_binding,
                    "readiness edge does not match exact row-bound obligation"
                );
                self.proof.edge_ids.insert(edge.id);
                if matches!(
                    edge.kind,
                    EdgeKind::ReadinessPrepare | EdgeKind::ReadinessRequired
                ) {
                    self.proof.side_edge_ids.insert(edge.id);
                }
                self.proof.unknown_nodes.extend([edge.from, edge.to]);
                if edge.kind == EdgeKind::ReadinessRequired {
                    self.proof.dual_nodes.insert(edge.to);
                }
                self.unavailable += 1;
                continue;
            }
            if let (Some(from), Some(to)) = (from, to) {
                if from.kind == NodeKind::WaitConsumed {
                    ensure!(
                        edge.kind == EdgeKind::Continuation
                            && to.kind == NodeKind::CpuSegment
                            && to.scope_id == from.scope_id
                            && (from.task.is_none() || to.task == from.task),
                        "wait continuation changes exact scope, task or role"
                    );
                    self.proof.edge_ids.insert(edge.id);
                } else if from.gap_reason == Some(GapReason::QueryWait) {
                    ensure!(
                        matches!(edge.kind, EdgeKind::Strand | EdgeKind::Continuation)
                            && to.kind == NodeKind::CpuSegment
                            && to.scope_id == from.scope_id,
                        "independent wait stream reconnects unrelated ancestry"
                    );
                    self.proof.edge_ids.insert(edge.id);
                } else if from.kind == NodeKind::ReadinessPrerequisite
                    || from.gap_reason == Some(GapReason::LatchVisibility)
                    || from.kind == NodeKind::WaitBegin
                    || from.gap_reason == Some(GapReason::WakeToConsume)
                    || to.kind == NodeKind::ReadinessPrerequisite
                    || to.kind == NodeKind::WaitConsumed
                    || to.gap_reason == Some(GapReason::LatchVisibility)
                    || wake_gap(to.gap_reason)
                {
                    let lost_visibility = self.lost_ready
                        && !self.claimed.contains(&from.id)
                        && !self.claimed.contains(&to.id)
                        && from.kind == NodeKind::ReadinessPrerequisite
                        && to.gap_reason == Some(GapReason::LatchVisibility)
                        && edge.kind == EdgeKind::Strand;
                    ensure!(
                        lost_visibility,
                        "unbound edge changes exact readiness gate obligations"
                    );
                    self.proof.edge_ids.insert(edge.id);
                    self.proof.unknown_nodes.extend([from.id, to.id]);
                    self.unavailable += 1;
                }
            }
        }
        if !self.expected.is_empty() {
            for &(from, to, _) in self.expected.keys() {
                ensure!(
                    self.flow.lost_edges > 0
                        || self.flow.identity_exhausted
                        || self.flow.loss_overflowed
                        || (self.flow.lost_nodes > 0
                            && (!self.nodes.contains_key(&from) || !self.nodes.contains_key(&to))),
                    "retained readiness rows omit required original edges"
                );
            }
            self.unavailable += self.expected.len();
            for &(from, to, _) in self.expected.keys() {
                self.proof.unknown_nodes.extend([from, to]);
            }
        }
        for node in self.flow.nodes.iter().filter(|n| {
            matches!(
                n.kind,
                NodeKind::ReadinessPrerequisite | NodeKind::WaitBegin | NodeKind::WaitConsumed
            ) || wake_gap(n.gap_reason)
                || matches!(
                    n.gap_reason,
                    Some(
                        GapReason::LatchVisibility
                            | GapReason::QueryWait
                            | GapReason::WakeToConsume
                    )
                )
        }) {
            let missing_row_allowed = if node.kind == NodeKind::ReadinessPrerequisite
                || node.gap_reason == Some(GapReason::LatchVisibility)
            {
                self.lost_ready
            } else {
                self.lost_wait
            };
            ensure!(
                self.claimed.contains(&node.id) || missing_row_allowed,
                "new readiness role lacks an original row binding"
            );
            if !self.claimed.contains(&node.id) {
                self.proof.unknown_nodes.insert(node.id);
                self.unavailable += 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::execution_graph_query::{OriginalWait, QueryExecutionInventory};
    use crate::execution_graph_timing::ObservedCpuAssessment;
    use serde_json::{Value, json};

    pub(crate) fn fixture() -> Value {
        let mut v = flow::tests::fixture();
        v["schema"] = json!(2);
        v["scope"] = json!("observed-query-join-and-latch-control-flow");
        for (k, val) in [
            ("readiness_allocator_baseline", json!(1)),
            ("readiness_allocator_high_water", json!(1)),
            ("expected_readiness", json!(0)),
            ("lost_readiness", json!(0)),
            ("active_readiness", json!(0)),
            ("readiness", json!([])),
            ("wait_attempt_allocator_baseline", json!(1)),
            ("wait_attempt_allocator_high_water", json!(1)),
            ("expected_wait_attempts", json!(0)),
            ("lost_wait_attempts", json!(0)),
            ("active_wait_attempts", json!(0)),
            ("wait_attempts", json!([])),
        ] {
            v[k] = val;
        }
        v
    }
    fn encounter() -> Value {
        let mut v = fixture();
        v["nodes"][2]["at_ns"] = json!(90);
        let n = |id, at, kind, scope, task, gap: Option<&str>| json!({"id":id,"at_ns":at,"kind":kind,"segment_id":null,"scope_id":scope,"task":task,"outcome":null,"gap_reason":gap});
        v["nodes"].as_array_mut().unwrap().extend([
            n(5,30,"readiness_prerequisite",None,None,None),n(6,31,"cut",None,None,Some("latch_visibility")),
            n(7,20,"wait_begin",Some(1),Some(1),None),n(8,20,"cut",Some(1),None,Some("query_wait")),
            n(9,50,"cut",Some(1),Some(1),Some("wake_to_consume")),n(10,51,"wait_consumed",Some(1),Some(1),None),
            json!({"id":11,"at_ns":52,"kind":"cpu_segment","segment_id":3,"scope_id":1,"task":1,"outcome":null,"gap_reason":null})]);
        v["nodes"][0]["task"] = Value::Null;
        v["nodes"][2]["task"] = Value::Null;
        v["node_allocator_high_water"] = json!(12);
        v["expected_nodes"] = json!(11);
        v["edges"] = json!([
            {"id":1,"from":1,"to":2,"kind":"strand"},{"id":2,"from":2,"to":7,"kind":"strand"},
            {"id":3,"from":4,"to":5,"kind":"readiness_prepare"},{"id":4,"from":5,"to":6,"kind":"strand"},
            {"id":5,"from":7,"to":9,"kind":"wait_boundary"},{"id":6,"from":9,"to":10,"kind":"wait_required"},
            {"id":7,"from":6,"to":10,"kind":"readiness_required"},{"id":8,"from":10,"to":11,"kind":"continuation"},
            {"id":9,"from":11,"to":3,"kind":"strand"}]);
        v["edge_allocator_high_water"] = json!(10);
        v["expected_edges"] = json!(9);
        v["readiness_allocator_high_water"] = json!(2);
        v["expected_readiness"] = json!(1);
        v["wait_attempt_allocator_high_water"] = json!(2);
        v["expected_wait_attempts"] = json!(1);
        v["readiness"] = json!([{"id":1,"query_context":1,"query_job":2,"prefix":4,"prerequisite":5,"visibility_gap":6,"outcome":"published"}]);
        v["wait_attempts"] = json!([{"id":1,"query_context":1,"waitee_job":2,"consumer_job":1,"caller_scope":1,"wait_id":1,
            "mode":"registered","outcome":"consumed","begin":7,"wait_gap":8,"readiness":1,"wake_gap":9,"consumed":10}]);
        v
    }
    fn timing() -> ObservedCpuAssessment {
        let mut t = flow::tests::timing();
        let mut segment = t.validated_segments[0].clone();
        segment.segment_id = 3;
        segment.start_ns = Some(52);
        segment.end_ns = Some(60);
        segment.cpu_start_ns = Some(100);
        segment.cpu_end_ns = Some(102);
        segment.cpu_ns = Some(2);
        t.validated_segments.push(segment);
        t.segment_allocator_high_water = Some(4);
        t
    }
    fn waits() -> OriginalWaitAuthority {
        OriginalWaitAuthority {
            capture_id: "query_123-safe".into(),
            records_complete: true,
            allocator_high_water: Some(2),
            expected_waits: Some(1),
            lost_events: 0,
            waits: BTreeMap::from([(
                1,
                OriginalWait {
                    context: 1,
                    waitee: 2,
                    consumer: Some(1),
                    registered_ns: Some(25),
                    notified_ns: Some(40),
                    condvar_returned_ns: Some(45),
                    resumed_ns: Some(49),
                    reason: Some(OriginalWaitReason::Completed),
                    valid: true,
                },
            )]),
        }
    }
    fn empty_waits() -> OriginalWaitAuthority {
        OriginalWaitAuthority {
            capture_id: "query_123-safe".into(),
            records_complete: true,
            allocator_high_water: Some(1),
            expected_waits: Some(0),
            lost_events: 0,
            waits: BTreeMap::new(),
        }
    }
    fn collect(
        v: Value,
        t: &ObservedCpuAssessment,
        q: Option<&QueryExecutionInventory>,
        w: Option<&OriginalWaitAuthority>,
    ) -> Result<ReadinessAssessment> {
        let tasks = BTreeMap::from([(
            1,
            crate::execution_graph_continuation::TaskInterval {
                start_ns: 1,
                finish_ns: 100,
            },
        )]);
        assess(
            serde_json::from_value(v)?,
            ReadinessContext {
                flow: FlowContext {
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
                waits: w,
            },
        )
    }
    #[test]
    fn empty_readiness_inventory_preserves_v1_known_cpu_and_bytes() {
        let t = flow::tests::timing();
        let q = flow::tests::authority(&t);
        let a = collect(fixture(), &t, Some(&q), Some(&empty_waits())).unwrap();
        assert!(a.flow.records_complete && a.ready_records_complete && a.wait_records_complete);
        assert_eq!(a.flow.known_cpu_work_ns, Some(9));
        assert_eq!(a.matched_readiness_transfers, 0);
        assert!(serde_json::from_value::<FlowCapture>(fixture()).is_err());
    }
    #[test]
    fn registered_exact_two_obligations_leave_visibility_and_wake_unknown() {
        let t = timing();
        let q = flow::tests::authority(&t);
        let a = collect(encounter(), &t, Some(&q), Some(&waits())).unwrap();
        assert!(a.flow.records_complete && a.ready_records_complete && a.wait_records_complete);
        assert_eq!(a.matched_readiness_transfers, 1);
        assert_eq!(a.flow.known_cpu_work_ns, Some(11));
        assert_eq!(a.flow.longest_known_chain_cpu_ns, Some(6));
        assert!(!a.flow.full_pipeline_complete && !a.flow.flow_complete);
        assert!(a.flow.issues.is_empty());
    }
    #[test]
    fn already_ready_never_invents_an_original_registered_wait() {
        let t = timing();
        let q = flow::tests::authority(&t);
        let mut v = encounter();
        v["wait_attempts"][0]["mode"] = json!("already_ready");
        v["wait_attempts"][0]["wait_id"] = Value::Null;
        assert_eq!(
            collect(v.clone(), &t, Some(&q), None)
                .unwrap()
                .matched_readiness_transfers,
            1
        );
        let mut poisoned = flow::tests::authority(&t);
        poisoned.terminals.insert((1, 2), QueryTerminal::Poisoned);
        assert!(
            collect(v, &t, Some(&poisoned), None)
                .unwrap_err()
                .to_string()
                .contains("terminal")
        );
    }
    #[test]
    fn wrong_scope_producer_and_obligation_are_rejected() {
        let t = timing();
        let q = flow::tests::authority(&t);
        for (key, val) in [
            ("consumer_job", json!(2)),
            ("waitee_job", json!(1)),
            ("caller_scope", json!(2)),
            ("wait_id", Value::Null),
        ] {
            let mut v = encounter();
            v["wait_attempts"][0][key] = val;
            assert!(collect(v, &t, Some(&q), Some(&waits())).is_err(), "{key}");
        }
        for index in [2, 5, 6] {
            let mut v = encounter();
            v["edges"][index]["from"] = json!(1);
            assert!(collect(v, &t, Some(&q), Some(&waits())).is_err());
        }
    }
    #[test]
    fn original_poison_cycle_and_late_notification_cannot_authorize_success() {
        let t = timing();
        let q = flow::tests::authority(&t);
        for reason in [OriginalWaitReason::Poisoned, OriginalWaitReason::Cycle] {
            let mut w = waits();
            w.waits.get_mut(&1).unwrap().reason = Some(reason);
            assert!(collect(encounter(), &t, Some(&q), Some(&w)).is_err());
        }
        let mut w = waits();
        w.waits.get_mut(&1).unwrap().notified_ns = Some(30);
        assert!(collect(encounter(), &t, Some(&q), Some(&w)).is_err());
    }
    #[test]
    fn missing_authority_preserves_only_independent_known_cpu() {
        let t = timing();
        let q = flow::tests::authority(&t);
        let a = collect(encounter(), &t, Some(&q), None).unwrap();
        assert!(!a.flow.records_complete && !a.wait_records_complete);
        assert_eq!(a.matched_readiness_transfers, 0);
        assert_eq!(a.flow.known_cpu_work_ns, Some(11));
        let a = collect(encounter(), &t, None, None).unwrap();
        assert_eq!(a.matched_readiness_transfers, 0);
        assert_eq!(a.flow.known_cpu_work_ns, Some(3));
    }
    #[test]
    fn independent_ready_or_wait_row_loss_is_unknown_not_a_forged_endpoint() {
        let t = timing();
        let q = flow::tests::authority(&t);
        for (rows, lost) in [
            ("readiness", "lost_readiness"),
            ("wait_attempts", "lost_wait_attempts"),
        ] {
            let mut v = encounter();
            v[rows] = json!([]);
            v[lost] = json!(1);
            v["records_complete"] = json!(false);
            let a = collect(v, &t, Some(&q), Some(&waits())).unwrap();
            assert!(!a.flow.records_complete);
            assert_eq!(a.matched_readiness_transfers, 0);
            assert!(a.unavailable_readiness_references > 0);
        }
    }
    #[test]
    fn omitted_required_edge_needs_independent_loss_and_never_matches() {
        let t = timing();
        let q = flow::tests::authority(&t);
        let mut v = encounter();
        v["edges"].as_array_mut().unwrap().remove(6);
        v["expected_edges"] = json!(8);
        v["edge_allocator_high_water"] = json!(9);
        for (i, e) in v["edges"].as_array_mut().unwrap().iter_mut().enumerate() {
            e["id"] = json!(i + 1);
        }
        assert!(collect(v, &t, Some(&q), Some(&waits())).is_err());
        let mut v = encounter();
        v["edges"].as_array_mut().unwrap().remove(6);
        v["lost_edges"] = json!(1);
        v["records_complete"] = json!(false);
        assert_eq!(
            collect(v, &t, Some(&q), Some(&waits()))
                .unwrap()
                .matched_readiness_transfers,
            0
        );
    }
    #[test]
    fn forged_mid_cpu_preparation_and_unrelated_continuation_are_rejected() {
        let t = timing();
        let q = flow::tests::authority(&t);
        let mut v = encounter();
        v["nodes"][4]["at_ns"] = json!(25);
        assert!(collect(v, &t, Some(&q), Some(&waits())).is_err());
        let mut v = encounter();
        v["edges"][7]["to"] = json!(4);
        assert!(collect(v, &t, Some(&q), Some(&waits())).is_err());
    }
    #[test]
    fn independently_bounded_ids_and_active_counts_fail_closed() {
        let t = timing();
        let q = flow::tests::authority(&t);
        for key in [
            "active_readiness",
            "active_wait_attempts",
            "lost_readiness",
            "lost_wait_attempts",
        ] {
            let mut v = encounter();
            v[key] = json!(1);
            assert!(collect(v, &t, Some(&q), Some(&waits())).is_err());
        }
        let mut v = encounter();
        v["wait_attempts"][0]["readiness"] = json!(2);
        assert!(collect(v, &t, Some(&q), Some(&waits())).is_err());
        let mut v = encounter();
        v["wait_attempts"][0]["begin"] = json!(12);
        assert!(collect(v, &t, Some(&q), Some(&waits())).is_err());
    }

    #[test]
    fn complete_inventory_cannot_delete_whole_registered_encounter() {
        let t = flow::tests::timing();
        let q = flow::tests::authority(&t);
        let a = collect(fixture(), &t, Some(&q), Some(&waits())).unwrap();
        assert!(!a.flow.records_complete && !a.wait_records_complete);
        assert!(
            a.flow
                .issues
                .iter()
                .any(|s| s.contains("lack readiness encounters"))
        );
        let mut v = fixture();
        v["records_complete"] = json!(false);
        v["lost_wait_attempts"] = json!(1);
        v["expected_wait_attempts"] = json!(1);
        v["wait_attempt_allocator_high_water"] = json!(2);
        let a = collect(v, &t, Some(&q), Some(&waits())).unwrap();
        assert!(!a.flow.records_complete && !a.wait_records_complete);
    }
    #[test]
    fn missing_or_partial_original_waits_cannot_certify_empty_reverse_coverage() {
        let t = flow::tests::timing();
        let q = flow::tests::authority(&t);
        let a = collect(fixture(), &t, Some(&q), None).unwrap();
        assert!(!a.wait_records_complete && !a.flow.records_complete);
        let mut w = empty_waits();
        w.records_complete = false;
        let a = collect(fixture(), &t, Some(&q), Some(&w)).unwrap();
        assert!(!a.wait_records_complete && !a.flow.records_complete);
    }

    fn failed(outcome: &str, gap: &str) -> Value {
        let mut v = encounter();
        v["wait_attempts"][0]["outcome"] = json!(outcome);
        v["wait_attempts"][0]["consumed"] = Value::Null;
        v["nodes"][8]["gap_reason"] = json!(gap);
        v["nodes"].as_array_mut().unwrap().remove(9);
        v["nodes"][9]["id"] = json!(10);
        v["node_allocator_high_water"] = json!(11);
        v["expected_nodes"] = json!(10);
        v["edges"]
            .as_array_mut()
            .unwrap()
            .retain(|e| e["kind"] != "wait_required" && e["kind"] != "readiness_required");
        for (i, e) in v["edges"].as_array_mut().unwrap().iter_mut().enumerate() {
            e["id"] = json!(i + 1);
            if e["kind"] == "continuation" {
                e["from"] = json!(9);
                e["to"] = json!(10);
            } else if e["from"] == 11 {
                e["from"] = json!(10);
            }
        }
        v["edge_allocator_high_water"] = json!(8);
        v["expected_edges"] = json!(7);
        v
    }
    #[test]
    fn publication_finalization_race_is_unbound_even_with_exact_inventory() {
        let t = timing();
        let q = flow::tests::authority(&t);
        let a = collect(
            failed("consumed_unbound", "readiness_unavailable"),
            &t,
            Some(&q),
            Some(&waits()),
        )
        .unwrap();
        assert_eq!(a.unbound_wait_attempts, 1);
        assert_eq!(a.matched_readiness_transfers, 0);
        assert!(!a.flow.records_complete && !a.wait_records_complete);
        assert_eq!(a.flow.known_cpu_work_ns, Some(11));
        let mut v = failed("consumed_unbound", "readiness_unavailable");
        v["readiness"][0]["outcome"] = json!("active");
        v["active_readiness"] = json!(1);
        v["records_complete"] = json!(false);
        assert_eq!(
            collect(v, &t, Some(&q), Some(&waits()))
                .unwrap()
                .matched_readiness_transfers,
            0
        );
    }
    #[test]
    fn real_failure_roles_and_preclassification_unwind_never_transfer() {
        let t = timing();
        for (outcome, gap, reason) in [
            ("cycle", "wait_cycle", OriginalWaitReason::Cycle),
            ("poisoned", "wait_poisoned", OriginalWaitReason::Poisoned),
        ] {
            let mut q = flow::tests::authority(&t);
            let mut w = waits();
            let mut v = failed(outcome, gap);
            w.waits.get_mut(&1).unwrap().reason = Some(reason);
            if outcome == "poisoned" {
                q.terminals.insert((1, 2), QueryTerminal::Poisoned);
                v["readiness"][0]["outcome"] = json!("poisoned");
            }
            let a = collect(v, &t, Some(&q), Some(&w)).unwrap();
            assert_eq!(a.matched_readiness_transfers, 0);
            let mut wrong = failed(outcome, gap);
            wrong["wait_attempts"][0]["readiness"] = Value::Null;
            assert!(collect(wrong, &t, Some(&q), Some(&waits())).is_err());
        }
        let q = flow::tests::authority(&t);
        let mut v = failed("unwound", "wait_unwound");
        v["wait_attempts"][0]["mode"] = json!("active");
        v["wait_attempts"][0]["wait_id"] = Value::Null;
        let a = collect(v, &t, Some(&q), None).unwrap();
        assert_eq!(a.active_wait_attempts, 0);
        assert_eq!(a.matched_readiness_transfers, 0);
    }
    #[test]
    fn lost_node_does_not_fabricate_an_attempted_edge_or_success() {
        let t = timing();
        let q = flow::tests::authority(&t);
        let mut v = encounter();
        v["nodes"].as_array_mut().unwrap().remove(5); // allocated visibility gate was not retained
        v["edges"]
            .as_array_mut()
            .unwrap()
            .retain(|e| e["from"] != 6 && e["to"] != 6);
        for (i, e) in v["edges"].as_array_mut().unwrap().iter_mut().enumerate() {
            e["id"] = json!(i + 1);
        }
        v["expected_edges"] = json!(7);
        v["edge_allocator_high_water"] = json!(8);
        v["lost_nodes"] = json!(1);
        v["records_complete"] = json!(false);
        let a = collect(v, &t, Some(&q), Some(&waits())).unwrap();
        assert_eq!(a.matched_readiness_transfers, 0);
        assert!(a.unavailable_readiness_references > 0);
    }
}
