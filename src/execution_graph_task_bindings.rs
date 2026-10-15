//! Allocator-backed query-point to continuation-task lineage validation.
//! Local record coverage and reciprocal pool authority are separate evidence.
use anyhow::{Result, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_POINTS: usize = 100_000;
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Source {
    Admitted { context_id: u64, job_id: u64 },
    RequestStarted { request_id: u64 },
    DepRead { read_id: u64 },
    ResultProduced { result_id: u64 },
    WaitRegistered { wait_id: u64 },
    WaitResumed { wait_id: u64 },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BindingCapture {
    schema: u32,
    records_complete: bool,
    all_targets_bound: bool,
    #[serde(deserialize_with = "nullable")]
    expected_points: Option<u64>,
    lost_events: u64,
    loss_overflowed: bool,
    #[serde(deserialize_with = "nullable")]
    pool: Option<PoolReference>,
    #[serde(deserialize_with = "nullable")]
    unavailable_reason: Option<UnavailableReason>,
    events: Vec<BindingEvent>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PoolReference {
    capture_id: String,
    observer: u64,
}
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum UnavailableReason {
    PoolUnavailable,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BindingEvent {
    source: Source,
    #[serde(deserialize_with = "nullable")]
    target: Option<Target>,
    #[serde(deserialize_with = "nullable")]
    unbound_reason: Option<UnboundReason>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    task: u64,
    at_pool_ns: u64,
}
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum UnboundReason {
    PoolUnavailable,
    NoActive,
    Suspended,
    ForeignObserver,
    ClockOverflow,
}

#[derive(Clone, Debug, Serialize)]
pub struct BindingAssessment {
    /// Exact independent allocator coverage, unaffected by underlying event loss.
    pub records_complete: bool,
    pub all_targets_bound: bool,
    /// Complete coverage plus valid reciprocal authority and all targets bound.
    pub graph_binding_complete: bool,
    /// Valid recorded lineage, even when explicitly unbound points remain.
    pub graph_binding_valid: bool,
    /// Underlying query/read/wait endpoints were available for every recorded point.
    pub source_endpoints_complete: bool,
    pub expected_points: Option<u64>,
    pub bound_points: usize,
    pub unbound_points: usize,
    pub lost_events: u64,
    pub loss_overflowed: bool,
    pub issues: Vec<String>,
}
#[derive(Debug)]
pub struct ValidatedBindingCapture {
    capture: BindingCapture,
    pub assessment: BindingAssessment,
}
impl ValidatedBindingCapture {
    pub fn pool_capture_id(&self) -> Option<&str> {
        self.capture
            .pool
            .as_ref()
            .map(|pool| pool.capture_id.as_str())
    }
}

/// Sources are validated against independent allocator ranges, not binding rows.
/// Ranges stay lazy: independent counters may exceed the retained-event budget.
#[derive(Default)]
pub(crate) struct QuerySourceInventory {
    pub expected_points: Option<u64>,
    pub expected_contexts: u64,
    pub context_high_water: BTreeMap<u64, Option<u64>>,
    pub read_high_water: [u64; 3],
    pub wait_high_water: Option<u64>,
    pub observed: BTreeSet<Source>,
    pub issues: Vec<String>,
    pub unknown_contexts: BTreeSet<u64>,
    pub unknown_waits: bool,
}

#[allow(dead_code)] // Strict compatibility entry point.
pub(crate) fn assess(
    capture: BindingCapture,
    inventory: QuerySourceInventory,
) -> Result<ValidatedBindingCapture> {
    assess_with_limit(capture, inventory, MAX_POINTS)
}

pub(crate) fn assess_with_limit(
    capture: BindingCapture,
    mut inventory: QuerySourceInventory,
    max_points: usize,
) -> Result<ValidatedBindingCapture> {
    ensure!(
        max_points <= 1_000_000,
        "task-binding policy exceeds hard bound"
    );
    ensure!(capture.schema == 1, "unsupported task-binding schema");
    ensure!(
        capture.events.len() <= max_points,
        "task-binding event budget exceeded"
    );
    if capture.expected_points.is_none() && inventory.expected_points.is_some() {
        ensure!(
            !capture.records_complete && !capture.all_targets_bound,
            "unknown task-binding total cannot attest completeness"
        );
        inventory.expected_points = None;
        inventory.issues.push(
            "task-binding independent total declared unknown after allocator loss or overflow"
                .into(),
        );
    } else {
        ensure!(
            capture.expected_points == inventory.expected_points,
            "task-binding independent expected point count mismatch"
        );
    }
    ensure!(
        capture.pool.is_none() == capture.unavailable_reason.is_some(),
        "task-binding pool availability mismatch"
    );
    if let Some(pool) = &capture.pool {
        ensure!(
            !pool.capture_id.is_empty() && pool.capture_id.len() <= 128 && pool.observer > 0,
            "invalid task-binding pool identity"
        );
    }
    let mut issues = inventory.issues;
    let independent_inventory_valid = issues.is_empty();
    let mut source_endpoints_complete = true;
    let mut seen = BTreeSet::new();
    let mut bound_points = 0;
    let mut unbound_points = 0;
    for row in &capture.events {
        ensure!(
            seen.insert(row.source.clone()),
            "duplicate task-binding source"
        );
        let unknown_range = match row.source {
            Source::Admitted { context_id, job_id } => {
                job_id > 0
                    && (inventory.unknown_contexts.contains(&context_id)
                        || (context_id > 0
                            && context_id <= inventory.expected_contexts
                            && !inventory.context_high_water.contains_key(&context_id)))
            }
            Source::WaitRegistered { wait_id } | Source::WaitResumed { wait_id } => {
                wait_id > 0 && inventory.unknown_waits
            }
            _ => false,
        };
        let in_range = match row.source {
            Source::Admitted { context_id, job_id } => {
                job_id > 0
                    && inventory
                        .context_high_water
                        .get(&context_id)
                        .copied()
                        .flatten()
                        .is_some_and(|high| job_id < high)
            }
            Source::RequestStarted { request_id } => {
                request_id > 0 && request_id < inventory.read_high_water[0]
            }
            Source::DepRead { read_id } => read_id > 0 && read_id < inventory.read_high_water[1],
            Source::ResultProduced { result_id } => {
                result_id > 0 && result_id < inventory.read_high_water[2]
            }
            Source::WaitRegistered { wait_id } | Source::WaitResumed { wait_id } => {
                wait_id > 0 && inventory.wait_high_water.is_some_and(|high| wait_id < high)
            }
        };
        ensure!(
            in_range || unknown_range,
            "foreign task-binding source outside allocator inventory"
        );
        if !inventory.observed.contains(&row.source) {
            source_endpoints_complete = false;
            issues.push(format!(
                "task-binding source has no recorded underlying endpoint: {:?}",
                row.source
            ));
        }
        ensure!(
            row.target.is_some() != row.unbound_reason.is_some(),
            "task-binding target/reason mismatch"
        );
        match &row.target {
            Some(target) => {
                ensure!(
                    capture.pool.is_some() && target.task > 0,
                    "bound task without valid pool/task identity"
                );
                bound_points += 1;
            }
            None => {
                ensure!(
                    capture.pool.is_some()
                        || row.unbound_reason == Some(UnboundReason::PoolUnavailable),
                    "unbound task reason disagrees with pool availability"
                );
                unbound_points += 1;
            }
        }
    }
    let coverage =
        inventory.expected_points.is_some() && inventory.expected_points == Some(seen.len() as u64);
    if !coverage {
        issues.push("task-binding independent source coverage is incomplete".into());
    }
    if capture.lost_events != 0 || capture.loss_overflowed {
        issues.push("task-binding capture lost events".into());
    }
    let records_complete = coverage
        && capture.lost_events == 0
        && !capture.loss_overflowed
        && independent_inventory_valid;
    ensure!(
        !capture.records_complete || records_complete,
        "false task-binding records_complete claim"
    );
    let all_targets_bound = records_complete && unbound_points == 0 && capture.pool.is_some();
    ensure!(
        !capture.all_targets_bound || (all_targets_bound && capture.records_complete),
        "false task-binding all_targets_bound claim"
    );
    if !capture.records_complete {
        issues.push("task-binding records declared incomplete".into());
    }
    if !capture.all_targets_bound {
        issues.push("task-binding targets declared incomplete".into());
    }
    let assessment = BindingAssessment {
        records_complete: records_complete && capture.records_complete,
        all_targets_bound: all_targets_bound && capture.all_targets_bound,
        graph_binding_complete: false,
        graph_binding_valid: false,
        source_endpoints_complete,
        expected_points: inventory.expected_points,
        bound_points,
        unbound_points,
        lost_events: capture.lost_events,
        loss_overflowed: capture.loss_overflowed,
        issues,
    };
    Ok(ValidatedBindingCapture {
        capture,
        assessment,
    })
}

/// A target becomes graph lineage only after matching reciprocal pool authority.
pub fn reconcile_bindings(
    query_capture_id: &str,
    bindings: &ValidatedBindingCapture,
    pools: &BTreeMap<String, crate::execution_graph_continuation::PoolInventory>,
) -> BindingAssessment {
    let mut assessment = bindings.assessment.clone();
    let Some(reference) = &bindings.capture.pool else {
        assessment
            .issues
            .push("task-binding pool unavailable".into());
        return assessment;
    };
    if reference.capture_id == query_capture_id {
        assessment
            .issues
            .push("query and pool capture authorities must be distinct".into());
        return assessment;
    }
    let Some(pool) = pools.get(&reference.capture_id) else {
        assessment
            .issues
            .push("task-binding matching pool authority unavailable".into());
        return assessment;
    };
    if !matches!(pool.schema, 2 | 3)
        || pool.capture_id != reference.capture_id
        || pool.observer != reference.observer
        || pool.query_capture_id.as_deref() != Some(query_capture_id)
        || !pool.structural_complete
        || (pool.schema == 3
            && pool.timing_bridge.as_ref().is_none_or(|bridge| {
                bridge.pool_capture_id != pool.capture_id
                    || bridge.query_capture_id != query_capture_id
                    || bridge.pool_observer != pool.observer
                    || bridge.pool_epoch_in_query_ns != 0
            }))
    {
        assessment
            .issues
            .push("task-binding reciprocal pool authority mismatch or incomplete".into());
        return assessment;
    }
    let mut targets_valid = true;
    for row in &bindings.capture.events {
        if let Some(target) = &row.target
            && pool.validate_point(target.task, target.at_pool_ns).is_err()
        {
            targets_valid = false;
            assessment.issues.push(format!(
                "task-binding target {} outside defined complete task interval",
                target.task
            ));
        }
    }
    assessment.graph_binding_valid = targets_valid && assessment.source_endpoints_complete;
    assessment.graph_binding_complete = assessment.records_complete
        && assessment.all_targets_bound
        && assessment.graph_binding_valid;
    assessment
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_graph_continuation::{PoolInventory, TaskInterval};
    use crate::execution_graph_query::{collect_bytes, tests::task_binding_fixture};

    #[test]
    fn large_capture_binding_limit_is_explicit_and_preserves_validation() {
        let count = MAX_POINTS + 1;
        let make = || {
            let events = (1..=count as u64)
                .map(|request_id| BindingEvent {
                    source: Source::RequestStarted { request_id },
                    target: Some(Target {
                        task: 1,
                        at_pool_ns: 10,
                    }),
                    unbound_reason: None,
                })
                .collect();
            let capture = BindingCapture {
                schema: 1,
                records_complete: true,
                all_targets_bound: true,
                expected_points: Some(count as u64),
                lost_events: 0,
                loss_overflowed: false,
                pool: Some(PoolReference {
                    capture_id: "pool-1".into(),
                    observer: 1,
                }),
                unavailable_reason: None,
                events,
            };
            let inventory = QuerySourceInventory {
                expected_points: Some(count as u64),
                read_high_water: [count as u64 + 1, 1, 1],
                observed: (1..=count as u64)
                    .map(|request_id| Source::RequestStarted { request_id })
                    .collect(),
                ..Default::default()
            };
            (capture, inventory)
        };
        let (capture, inventory) = make();
        assert!(assess(capture, inventory).is_err());
        let (capture, inventory) = make();
        let valid = assess_with_limit(capture, inventory, 1_000_000).unwrap();
        assert!(valid.assessment.records_complete && valid.assessment.all_targets_bound);
        assert_eq!(valid.assessment.bound_points, count);
        let (mut capture, inventory) = make();
        capture.events[0].source = Source::RequestStarted {
            request_id: count as u64 + 1,
        };
        assert!(assess_with_limit(capture, inventory, 1_000_000).is_err());
        let (capture, inventory) = make();
        assert!(assess_with_limit(capture, inventory, count - 1).is_err());
        let (capture, inventory) = make();
        assert!(assess_with_limit(capture, inventory, 1_000_001).is_err());
    }
    use serde_json::json;

    fn pool(query: &str) -> PoolInventory {
        PoolInventory {
            schema: 2,
            timing_bridge: None,
            capture_id: "pool-1".into(),
            observer: 1,
            query_capture_id: Some(query.into()),
            structural_complete: true,
            task_intervals: BTreeMap::from([(
                1,
                TaskInterval {
                    start_ns: 10,
                    finish_ns: 20,
                },
            )]),
        }
    }

    #[test]
    fn valid_partial_lineage_is_not_complete_lineage() {
        let mut fixture = task_binding_fixture();
        fixture["task_bindings"]["all_targets_bound"] = json!(false);
        fixture["task_bindings"]["events"][0]["target"] = serde_json::Value::Null;
        fixture["task_bindings"]["events"][0]["unbound_reason"] = json!("pool_unavailable");
        for invalid_target in [false, true] {
            let mut fixture = fixture.clone();
            if invalid_target {
                fixture["task_bindings"]["events"][1]["target"]["task"] = json!(99);
            }
            let query = collect_bytes(&serde_json::to_vec(&fixture).unwrap()).unwrap();
            let id = &query.assessment.capture_id;
            let a = reconcile_bindings(
                id,
                query.bindings.as_ref().unwrap(),
                &BTreeMap::from([("pool-1".into(), pool(id))]),
            );
            assert_eq!(a.graph_binding_valid, !invalid_target);
            assert!(a.records_complete);
            assert!(!a.all_targets_bound && !a.graph_binding_complete);
            let a = reconcile_bindings(id, query.bindings.as_ref().unwrap(), &BTreeMap::new());
            assert!(!a.graph_binding_valid);
        }
        let mut fixture = task_binding_fixture();
        fixture["query_reads"]["events"]
            .as_array_mut()
            .unwrap()
            .remove(1);
        fixture["query_reads"]["records_complete"] = json!(false);
        fixture["query_reads"]["lost_events"] = json!(1);
        let query = collect_bytes(&serde_json::to_vec(&fixture).unwrap()).unwrap();
        let id = &query.assessment.capture_id;
        let a = reconcile_bindings(
            id,
            query.bindings.as_ref().unwrap(),
            &BTreeMap::from([("pool-1".into(), pool(id))]),
        );
        assert!(a.records_complete && a.all_targets_bound);
        assert!(
            !a.source_endpoints_complete && !a.graph_binding_valid && !a.graph_binding_complete
        );
    }

    #[test]
    fn reciprocal_authority_and_inclusive_intervals() {
        for time in [9, 10, 15, 20, 21, u64::MAX] {
            let mut fixture = task_binding_fixture();
            for row in fixture["task_bindings"]["events"].as_array_mut().unwrap() {
                row["target"]["at_pool_ns"] = json!(time);
            }
            let query = collect_bytes(&serde_json::to_vec(&fixture).unwrap()).unwrap();
            let id = &query.assessment.capture_id;
            let pools = BTreeMap::from([("pool-1".into(), pool(id))]);
            let a = reconcile_bindings(id, query.bindings.as_ref().unwrap(), &pools);
            assert_eq!(a.graph_binding_complete, (10..=20).contains(&time), "{a:?}");
            assert!(a.records_complete && a.all_targets_bound);
        }
    }

    #[test]
    fn missing_legacy_foreign_incomplete_and_undefined_pool_matrix() {
        let query = collect_bytes(&serde_json::to_vec(&task_binding_fixture()).unwrap()).unwrap();
        let id = &query.assessment.capture_id;
        let bindings = query.bindings.as_ref().unwrap();
        let missing = reconcile_bindings(id, bindings, &BTreeMap::new());
        assert!(!missing.graph_binding_complete);
        for case in 0..7 {
            let mut authority = pool(id);
            match case {
                0 => authority.schema = 1,
                1 => authority.observer = 2,
                2 => authority.capture_id = "foreign".into(),
                3 => authority.query_capture_id = Some("foreign".into()),
                4 => authority.query_capture_id = None,
                5 => authority.structural_complete = false,
                _ => authority.task_intervals.clear(),
            }
            let pools = BTreeMap::from([("pool-1".into(), authority)]);
            let a = reconcile_bindings(id, bindings, &pools);
            assert!(
                !a.graph_binding_complete && !a.issues.is_empty(),
                "case {case}: {a:?}"
            );
            assert!(a.records_complete && a.all_targets_bound);
        }
        let a = reconcile_bindings(
            "pool-1",
            bindings,
            &BTreeMap::from([("pool-1".into(), pool("pool-1"))]),
        );
        assert!(!a.graph_binding_complete);
    }
}
