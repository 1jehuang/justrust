//! Scoped observations of exclusive thread CPU segments, never whole-build work/span.
use crate::execution_graph_continuation::TaskInterval;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SEGMENTS: usize = 100_000;
fn nullable<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(d)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClockBridge {
    pub query_capture_id: String,
    pub pool_capture_id: String,
    pub pool_observer: u64,
    pub pool_epoch_in_query_ns: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TimingCapture {
    schema: u32,
    scope: String,
    clock_bridge: ClockBridge,
    cpu_clock: String,
    segment_allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    segment_allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_segments: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    thread_allocator_high_water: Option<u64>,
    lost_segments: u64,
    loss_overflowed: bool,
    identity_exhausted: bool,
    clock_failures: u64,
    active_segments: u64,
    sealed: bool,
    records_complete: bool,
    segments: Vec<Segment>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Segment {
    segment_id: u64,
    thread_id: u64,
    #[serde(deserialize_with = "nullable")]
    task: Option<u64>,
    owner_kind: OwnerKind,
    #[serde(deserialize_with = "nullable")]
    start_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    end_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    cpu_start_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    cpu_end_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    cpu_ns: Option<u64>,
    status: Status,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OwnerKind {
    Task,
    Opaque,
}
#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Status {
    Measured,
    ClockFailed,
    ClockRegressed,
    WallOverflow,
}

#[derive(Debug, Serialize)]
pub(crate) struct ObservedCpuAssessment {
    pub scope: &'static str,
    pub cpu_clock: &'static str,
    pub records_complete: bool,
    /// Sum of validated retained measurements only. Never full CPU work.
    pub observed_cpu_ns: Option<u64>,
    pub task_cpu_ns: Option<u64>,
    pub opaque_cpu_ns: Option<u64>,
    pub measured_segments: usize,
    pub failed_segments: usize,
    pub observed_threads: usize,
    pub expected_segments: Option<u64>,
    pub lost_segments: u64,
    pub active_segments: u64,
    pub clock_failures: u64,
    pub clock_bridge: ClockBridge,
    /// A pool receipt alone cannot establish reciprocal query clock authority.
    pub query_clock_aligned: bool,
    pub issues: Vec<String>,
}

pub(crate) struct TimingContext<'a> {
    pub pool_capture_id: &'a str,
    pub query_capture_id: &'a str,
    pub observer: u64,
    pub pool_sealed: bool,
    pub structural_complete: bool,
    pub task_high_water: u64,
    pub task_intervals: &'a BTreeMap<u64, TaskInterval>,
}

pub(crate) fn assess(
    capture: TimingCapture,
    context: TimingContext<'_>,
) -> Result<ObservedCpuAssessment> {
    ensure!(
        capture.schema == 1 && capture.scope == "observed-exclusive-thread-segments",
        "unsupported CPU timing schema or scope"
    );
    ensure!(
        capture.cpu_clock == "linux-thread-cpu",
        "unsupported CPU timing clock"
    );
    let bridge = &capture.clock_bridge;
    ensure!(
        bridge.pool_capture_id == context.pool_capture_id
            && bridge.query_capture_id == context.query_capture_id
            && bridge.pool_observer == context.observer
            && bridge.pool_epoch_in_query_ns == 0,
        "CPU timing bridge does not match same-epoch pool/query authority"
    );
    ensure!(
        capture.segment_allocator_baseline == 1 && capture.segments.len() <= MAX_SEGMENTS,
        "invalid or oversized CPU segment inventory"
    );
    match (
        capture.segment_allocator_high_water,
        capture.expected_segments,
    ) {
        (Some(high), Some(expected)) if high.checked_sub(1) == Some(expected) => {}
        (None, None) if capture.identity_exhausted => {}
        _ => bail!("invalid CPU segment allocator inventory"),
    }
    ensure!(
        capture
            .thread_allocator_high_water
            .is_some_and(|high| high > 0)
            || (capture.thread_allocator_high_water.is_none() && capture.identity_exhausted),
        "invalid CPU thread allocator inventory"
    );
    ensure!(
        capture.sealed == context.pool_sealed,
        "CPU timing seal differs from pool seal"
    );
    let mut issues = Vec::new();
    if !context.structural_complete {
        issues.push("pool task lifecycle authority is incomplete".into());
    }
    if capture.identity_exhausted || capture.loss_overflowed {
        issues.push("CPU timing identity or loss accounting is exhausted".into());
    }
    if !capture.sealed || capture.active_segments != 0 || capture.lost_segments != 0 {
        issues.push("CPU timing capture is unsealed, active, or lost segments".into());
    }
    if capture.thread_allocator_high_water.is_none() {
        issues.push("CPU timing thread inventory is unknown".into());
    }
    let accounted = (capture.segments.len() as u64)
        .checked_add(capture.lost_segments)
        .and_then(|n| n.checked_add(capture.active_segments));
    if accounted.is_none() || capture.expected_segments != accounted {
        issues.push("CPU timing independent segment coverage is incomplete or overflowed".into());
    }
    let mut ids = BTreeSet::new();
    let mut threads: BTreeMap<u64, Vec<&Segment>> = BTreeMap::new();
    let mut owned_tasks: BTreeMap<u64, Vec<&Segment>> = BTreeMap::new();
    let mut measured = 0usize;
    let mut failed = 0usize;
    let mut clock_failed_rows = 0u64;
    let mut task_cpu = 0u64;
    let mut opaque_cpu = 0u64;
    // Exhaustion also represents a failed TLS attribution cursor in the native
    // recorder. Without a separate integrity flag, retained CPU ownership is
    // unknown, even when some allocator bounds are still available.
    let mut endpoints_valid = context.structural_complete
        && capture.thread_allocator_high_water.is_some()
        && !capture.identity_exhausted
        && !capture.loss_overflowed;
    for row in &capture.segments {
        ensure!(
            row.segment_id > 0
                && capture
                    .segment_allocator_high_water
                    .is_none_or(|high| row.segment_id < high)
                && ids.insert(row.segment_id),
            "duplicate or foreign CPU segment identity"
        );
        ensure!(
            row.thread_id > 0
                && capture
                    .thread_allocator_high_water
                    .is_none_or(|high| row.thread_id < high),
            "foreign CPU thread identity"
        );
        threads.entry(row.thread_id).or_default().push(row);
        if row.cpu_start_ns.is_none() || row.cpu_end_ns.is_none() {
            clock_failed_rows += 1;
        }
        let interval = match (row.owner_kind, row.task) {
            (OwnerKind::Task, Some(task)) if task > 0 && task < context.task_high_water => {
                owned_tasks.entry(task).or_default().push(row);
                let interval = context.task_intervals.get(&task);
                if interval.is_none() {
                    endpoints_valid = false;
                    issues.push(format!(
                        "CPU segment {} has no complete task lifecycle",
                        row.segment_id
                    ));
                }
                interval
            }
            (OwnerKind::Opaque, None) => None,
            _ => bail!("CPU segment owner/task mismatch or foreign task"),
        };
        for wall in [row.start_ns, row.end_ns].into_iter().flatten() {
            ensure!(
                wall.checked_add(bridge.pool_epoch_in_query_ns).is_some(),
                "CPU segment query-clock mapping overflow"
            );
            if let Some(interval) = interval {
                ensure!(
                    interval.start_ns <= wall && wall <= interval.finish_ns,
                    "CPU segment outside task lifecycle"
                );
            }
        }
        if let (Some(start), Some(end)) = (row.start_ns, row.end_ns) {
            ensure!(start <= end, "CPU segment wall clock regressed");
        }
        match row.status {
            Status::Measured => {
                let (Some(start), Some(end), Some(cpu_start), Some(cpu_end), Some(cpu_ns)) = (
                    row.start_ns,
                    row.end_ns,
                    row.cpu_start_ns,
                    row.cpu_end_ns,
                    row.cpu_ns,
                ) else {
                    bail!("measured CPU segment has null endpoints");
                };
                ensure!(
                    cpu_end.checked_sub(cpu_start) == Some(cpu_ns),
                    "CPU segment delta mismatch or regression"
                );
                // These clocks have no exact cross-domain rate/error bound.
                // An inconsistent envelope cannot authorize a CPU quantity, but
                // must not reject an authentic capture for sampling/rate skew.
                if cpu_ns > end - start {
                    endpoints_valid = false;
                    issues.push(
                        "CPU/wall clock envelope inconsistency makes aggregate CPU unknown".into(),
                    );
                }
                let total = match row.owner_kind {
                    OwnerKind::Task => &mut task_cpu,
                    OwnerKind::Opaque => &mut opaque_cpu,
                };
                *total = total
                    .checked_add(cpu_ns)
                    .ok_or_else(|| anyhow::anyhow!("observed CPU sum overflow"))?;
                measured += 1;
            }
            Status::ClockFailed => {
                ensure!(
                    row.start_ns.is_some()
                        && row.end_ns.is_some()
                        && (row.cpu_start_ns.is_none() || row.cpu_end_ns.is_none())
                        && row.cpu_ns.is_none(),
                    "invalid failed CPU clock segment"
                );
                failed += 1;
            }
            Status::ClockRegressed => {
                ensure!(
                    row.start_ns.is_some()
                        && row.end_ns.is_some()
                        && row.cpu_ns.is_none()
                        && matches!((row.cpu_start_ns, row.cpu_end_ns), (Some(start), Some(end)) if end < start),
                    "invalid regressed CPU clock segment"
                );
                failed += 1;
            }
            Status::WallOverflow => {
                ensure!(
                    (row.start_ns.is_none() || row.end_ns.is_none()) && row.cpu_ns.is_none(),
                    "invalid overflowing CPU wall segment"
                );
                failed += 1;
            }
        }
    }
    // A native task guard cannot execute concurrently on different threads.
    // Validate the task axis as well as thread axes to prevent relabeling a
    // duplicated owned interval into another otherwise-valid thread stream.
    for rows in owned_tasks.values_mut() {
        rows.sort_by_key(|row| (row.start_ns, row.segment_id));
        let mut previous_end = None;
        for row in rows {
            if let (Some(start), Some(end)) = (row.start_ns, row.end_ns) {
                ensure!(
                    previous_end.is_none_or(|previous| previous <= start),
                    "overlapping CPU segments for one logical task across threads"
                );
                previous_end = Some(end);
            }
        }
    }
    for rows in threads.values_mut() {
        let regressed = rows.iter().any(|row| row.status == Status::ClockRegressed);
        if regressed {
            endpoints_valid = false;
            issues.push("CPU clock regression makes aggregate CPU unknown".into());
        }
        rows.sort_by_key(|row| (row.start_ns, row.segment_id));
        let mut wall_end = None;
        let mut cpu_end = None;
        for row in rows {
            if let (Some(start), Some(end)) = (row.start_ns, row.end_ns) {
                ensure!(
                    wall_end.is_none_or(|previous| previous <= start),
                    "overlapping exclusive CPU wall segments on one thread"
                );
                wall_end = Some(end);
            }
            if row.status == Status::Measured && !regressed {
                ensure!(
                    cpu_end.is_none_or(|previous| Some(previous) <= row.cpu_start_ns),
                    "overlapping or regressed exclusive CPU counters on one thread"
                );
                cpu_end = row.cpu_end_ns;
            }
        }
    }
    if failed != 0 || capture.clock_failures != 0 {
        issues.push("CPU timing includes clock failures or unavailable measurements".into());
    }
    ensure!(
        capture.clock_failures >= clock_failed_rows,
        "CPU clock-failed rows exceed read-failure counter"
    );
    let inventory_complete = capture.sealed
        && capture.active_segments == 0
        && capture.lost_segments == 0
        && !capture.loss_overflowed
        && !capture.identity_exhausted
        && capture.thread_allocator_high_water.is_some()
        && capture.segment_allocator_high_water.is_some()
        && capture.expected_segments == Some(capture.segments.len() as u64)
        && failed == 0
        && capture.clock_failures == 0;
    ensure!(
        !capture.records_complete || inventory_complete,
        "false CPU timing records_complete claim"
    );
    let complete = issues.is_empty() && endpoints_valid;
    if !capture.records_complete && complete {
        issues.push("CPU timing producer declared records incomplete".into());
    }
    let total = task_cpu
        .checked_add(opaque_cpu)
        .ok_or_else(|| anyhow::anyhow!("observed CPU sum overflow"))?;
    Ok(ObservedCpuAssessment {
        scope: "observed-exclusive-thread-segments",
        cpu_clock: "linux-thread-cpu",
        records_complete: capture.records_complete && complete,
        observed_cpu_ns: endpoints_valid.then_some(total),
        task_cpu_ns: endpoints_valid.then_some(task_cpu),
        opaque_cpu_ns: endpoints_valid.then_some(opaque_cpu),
        measured_segments: measured,
        failed_segments: failed,
        observed_threads: threads.len(),
        expected_segments: capture.expected_segments,
        lost_segments: capture.lost_segments,
        active_segments: capture.active_segments,
        clock_failures: capture.clock_failures,
        clock_bridge: capture.clock_bridge,
        query_clock_aligned: false,
        issues,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{Value, json};

    pub(crate) fn fixture() -> Value {
        json!({"schema":1,"scope":"observed-exclusive-thread-segments",
        "clock_bridge":{"query_capture_id":"query_123-safe","pool_capture_id":"pool-test","pool_observer":1,"pool_epoch_in_query_ns":0},
        "cpu_clock":"linux-thread-cpu","segment_allocator_baseline":1,
        "segment_allocator_high_water":3,"expected_segments":2,"thread_allocator_high_water":8,
        "lost_segments":0,"loss_overflowed":false,"identity_exhausted":false,
        "clock_failures":0,"active_segments":0,"sealed":true,"records_complete":true,
        "segments":[
            {"segment_id":1,"thread_id":7,"task":1,"owner_kind":"task","start_ns":10,"end_ns":20,"cpu_start_ns":100,"cpu_end_ns":106,"cpu_ns":6,"status":"measured"},
            {"segment_id":2,"thread_id":7,"task":null,"owner_kind":"opaque","start_ns":20,"end_ns":30,"cpu_start_ns":106,"cpu_end_ns":109,"cpu_ns":3,"status":"measured"}
        ]})
    }
    fn intervals() -> BTreeMap<u64, TaskInterval> {
        BTreeMap::from([(
            1,
            TaskInterval {
                start_ns: 1,
                finish_ns: 100,
            },
        )])
    }
    fn context(tasks: &BTreeMap<u64, TaskInterval>) -> TimingContext<'_> {
        TimingContext {
            pool_capture_id: "pool-test",
            query_capture_id: "query_123-safe",
            observer: 1,
            pool_sealed: true,
            structural_complete: true,
            task_high_water: 2,
            task_intervals: tasks,
        }
    }
    fn collect(v: &Value) -> Result<ObservedCpuAssessment> {
        assess(
            serde_json::from_slice(&serde_json::to_vec(v)?)?,
            context(&intervals()),
        )
    }

    #[test]
    fn observed_cpu_is_retained_exclusive_work_not_full_work_or_alignment() {
        let a = collect(&fixture()).unwrap();
        assert!(a.records_complete);
        assert_eq!(a.observed_cpu_ns, Some(9));
        assert_eq!(a.task_cpu_ns, Some(6));
        assert_eq!(a.opaque_cpu_ns, Some(3));
        assert_eq!(a.observed_threads, 1); // Global thread IDs need not be contiguous here.
        assert_eq!(a.measured_segments, 2);
        assert!(!a.query_clock_aligned);
    }

    #[test]
    fn required_nulls_original_duplicates_and_unknown_fields_are_strict() {
        for field in [
            "segment_allocator_high_water",
            "expected_segments",
            "thread_allocator_high_water",
        ] {
            let mut v = fixture();
            v.as_object_mut().unwrap().remove(field);
            assert!(collect(&v).is_err(), "{field}");
        }
        for field in [
            "task",
            "start_ns",
            "end_ns",
            "cpu_start_ns",
            "cpu_end_ns",
            "cpu_ns",
        ] {
            let mut v = fixture();
            v["segments"][0].as_object_mut().unwrap().remove(field);
            assert!(collect(&v).is_err(), "{field}");
        }
        let raw = serde_json::to_string(&fixture()).unwrap();
        for (needle, replacement) in [
            ("\"schema\":1", "\"schema\":1,\"schema\":1"),
            (
                "\"pool_observer\":1",
                "\"pool_observer\":1,\"pool_observer\":1",
            ),
            ("\"cpu_ns\":6", "\"cpu_ns\":6,\"cpu_ns\":6"),
        ] {
            assert!(
                serde_json::from_str::<TimingCapture>(&raw.replacen(needle, replacement, 1))
                    .is_err()
            );
        }
        for pointer in ["", "/clock_bridge", "/segments/0"] {
            let mut v = fixture();
            v.pointer_mut(pointer).unwrap()["unknown"] = json!(true);
            assert!(collect(&v).is_err());
        }
    }

    #[test]
    fn forged_clocks_owners_deltas_and_lifecycles_fail_closed() {
        for (pointer, value) in [
            ("/cpu_clock", json!("wall-clock")),
            ("/clock_bridge/query_capture_id", json!("foreign")),
            ("/clock_bridge/pool_capture_id", json!("foreign")),
            ("/clock_bridge/pool_observer", json!(2)),
            ("/clock_bridge/pool_epoch_in_query_ns", json!(1)),
            ("/clock_bridge/pool_epoch_in_query_ns", json!(u64::MAX)),
            ("/segment_allocator_baseline", json!(0)),
            ("/segment_allocator_high_water", json!(0)),
            ("/expected_segments", json!(3)),
            ("/thread_allocator_high_water", json!(0)),
            ("/segments/0/thread_id", json!(0)),
            ("/segments/0/thread_id", json!(8)),
            ("/segments/0/segment_id", json!(0)),
            ("/segments/0/segment_id", json!(3)),
            ("/segments/1/segment_id", json!(1)),
            ("/segments/0/task", json!(0)),
            ("/segments/0/task", json!(2)),
            ("/segments/0/task", Value::Null),
            ("/segments/1/task", json!(1)),
            ("/segments/0/start_ns", json!(0)),
            ("/segments/0/end_ns", json!(101)),
            ("/segments/0/end_ns", json!(9)),
            ("/segments/0/cpu_ns", json!(7)),
            ("/segments/0/cpu_end_ns", json!(99)),
            ("/segments/0/cpu_start_ns", Value::Null),
        ] {
            let mut v = fixture();
            *v.pointer_mut(pointer).unwrap() = value;
            assert!(collect(&v).is_err(), "accepted {pointer}: {v}");
        }
        // Instant and the thread CPU clock have no exact rate/error-bound
        // contract. Validate each domain, not a forged exact cross-rate bound.
        let mut v = fixture();
        v["segments"][0]["cpu_end_ns"] = json!(111);
        v["segments"][0]["cpu_ns"] = json!(11);
        v["segments"][1]["cpu_start_ns"] = json!(111);
        v["segments"][1]["cpu_end_ns"] = json!(114);
        let a = collect(&v).unwrap();
        assert!(a.observed_cpu_ns.is_none() && !a.records_complete);
    }

    #[test]
    fn per_thread_exclusivity_checks_wall_and_cpu_not_cross_thread_counters() {
        let mut v = fixture();
        v["segments"][1]["start_ns"] = json!(19);
        assert!(collect(&v).is_err());
        let mut v = fixture();
        v["segments"][1]["cpu_start_ns"] = json!(105);
        v["segments"][1]["cpu_end_ns"] = json!(108);
        assert!(collect(&v).is_err());
        v["segments"][1]["thread_id"] = json!(6);
        v["segments"][1]["start_ns"] = json!(10);
        assert!(collect(&v).unwrap().records_complete);
        let mut v = fixture();
        v["segments"][0]["end_ns"] = json!(10);
        v["segments"][0]["cpu_end_ns"] = json!(100);
        v["segments"][0]["cpu_ns"] = json!(0);
        assert!(collect(&v).unwrap().records_complete);
    }

    #[test]
    fn one_task_cannot_be_relabelled_as_concurrent_cpu_on_another_thread() {
        let mut v = fixture();
        let mut duplicate = v["segments"][0].clone();
        duplicate["segment_id"] = json!(3);
        duplicate["thread_id"] = json!(6);
        duplicate["cpu_start_ns"] = json!(0);
        duplicate["cpu_end_ns"] = json!(6);
        v["segments"].as_array_mut().unwrap().push(duplicate);
        v["expected_segments"] = json!(3);
        v["segment_allocator_high_water"] = json!(4);
        assert!(collect(&v).is_err());
        // A sequential task migration would not duplicate exclusive work.
        v["segments"][2]["start_ns"] = json!(30);
        v["segments"][2]["end_ns"] = json!(40);
        assert_eq!(collect(&v).unwrap().task_cpu_ns, Some(12));
        // Opaque work on independent threads is intentionally not one task.
        v["segments"][2]["start_ns"] = json!(10);
        v["segments"][2]["end_ns"] = json!(20);
        v["segments"][2]["task"] = Value::Null;
        v["segments"][2]["owner_kind"] = json!("opaque");
        assert!(collect(&v).unwrap().records_complete);
    }

    #[test]
    fn honest_failed_clock_rows_are_explicit_null_not_zero_work() {
        for (status, cpu_start, cpu_end, wall_end, failures) in [
            ("clock_failed", json!(100), Value::Null, json!(20), 1),
            ("clock_regressed", json!(100), json!(99), json!(20), 0),
            ("wall_overflow", json!(100), json!(106), Value::Null, 0),
        ] {
            let mut v = fixture();
            v["segments"][0]["status"] = json!(status);
            v["segments"][0]["cpu_start_ns"] = cpu_start;
            v["segments"][0]["cpu_end_ns"] = cpu_end;
            v["segments"][0]["end_ns"] = wall_end;
            v["segments"][0]["cpu_ns"] = Value::Null;
            v["clock_failures"] = json!(failures);
            v["records_complete"] = json!(false);
            let a = collect(&v).unwrap();
            assert!(!a.records_complete);
            assert_eq!(a.measured_segments, 1);
            assert_eq!(a.failed_segments, 1);
            assert_eq!(
                a.observed_cpu_ns,
                if status == "clock_regressed" {
                    None
                } else {
                    Some(3)
                }
            );
            v["records_complete"] = json!(true);
            assert!(collect(&v).is_err());
        }
        let mut v = fixture();
        v["segments"][0]["status"] = json!("clock_failed");
        v["records_complete"] = json!(false);
        assert!(collect(&v).is_err()); // Failure status cannot mask measured endpoints.
    }

    #[test]
    fn explicit_cpu_regression_allows_recovery_but_poisons_all_cpu_aggregates() {
        let mut v = fixture();
        let mut recovered = v["segments"][1].clone();
        recovered["segment_id"] = json!(3);
        recovered["start_ns"] = json!(30);
        recovered["end_ns"] = json!(40);
        recovered["cpu_start_ns"] = json!(6);
        recovered["cpu_end_ns"] = json!(9);
        v["segments"][1]["status"] = json!("clock_regressed");
        v["segments"][1]["cpu_start_ns"] = json!(120);
        v["segments"][1]["cpu_end_ns"] = json!(5);
        v["segments"][1]["cpu_ns"] = Value::Null;
        v["segments"].as_array_mut().unwrap().push(recovered);
        v["segment_allocator_high_water"] = json!(4);
        v["expected_segments"] = json!(3);
        v["records_complete"] = json!(false);
        let a = collect(&v).unwrap();
        assert!(!a.records_complete);
        assert_eq!(a.measured_segments, 2);
        assert!(
            a.observed_cpu_ns.is_none() && a.task_cpu_ns.is_none() && a.opaque_cpu_ns.is_none()
        );
        v["records_complete"] = json!(true);
        assert!(collect(&v).is_err());
        v["records_complete"] = json!(false);
        v["segments"][2]["start_ns"] = json!(29);
        assert!(collect(&v).is_err()); // Regression never excuses wall overlap.
    }

    #[test]
    fn loss_open_threads_exhaustion_and_counter_overflow_never_attest_complete() {
        for (pointer, value) in [
            ("/lost_segments", json!(1)),
            ("/active_segments", json!(1)),
            ("/loss_overflowed", json!(true)),
            ("/identity_exhausted", json!(true)),
            ("/clock_failures", json!(1)),
        ] {
            let mut v = fixture();
            *v.pointer_mut(pointer).unwrap() = value;
            assert!(collect(&v).is_err());
            v["records_complete"] = json!(false);
            let a = collect(&v).unwrap();
            assert!(!a.records_complete);
            if matches!(pointer, "/identity_exhausted" | "/loss_overflowed") {
                assert!(
                    a.observed_cpu_ns.is_none()
                        && a.task_cpu_ns.is_none()
                        && a.opaque_cpu_ns.is_none()
                );
            }
        }
        let mut v = fixture();
        v["records_complete"] = json!(false);
        v["lost_segments"] = json!(u64::MAX);
        v["active_segments"] = json!(1);
        assert!(!collect(&v).unwrap().records_complete);
        v["segment_allocator_high_water"] = Value::Null;
        v["expected_segments"] = Value::Null;
        v["thread_allocator_high_water"] = Value::Null;
        assert!(collect(&v).is_err());
        v["identity_exhausted"] = json!(true);
        let a = collect(&v).unwrap();
        assert!(!a.records_complete);
        assert!(a.observed_cpu_ns.is_none());
        let mut v = fixture();
        v["records_complete"] = json!(false);
        v["sealed"] = json!(false);
        let tasks = intervals();
        let mut ctx = context(&tasks);
        ctx.pool_sealed = false;
        assert!(
            !assess(serde_json::from_value(v).unwrap(), ctx)
                .unwrap()
                .records_complete
        );
    }

    #[test]
    fn independent_timing_cap_and_checked_aggregate_cpu_sum() {
        let mut capture: TimingCapture = serde_json::from_value(fixture()).unwrap();
        let row = capture.segments[0].clone();
        capture.segments.resize(MAX_SEGMENTS + 1, row);
        assert!(assess(capture, context(&intervals())).is_err());
        let mut v = fixture();
        for i in 0..2 {
            v["segments"][i]["task"] = Value::Null;
            v["segments"][i]["owner_kind"] = json!("opaque");
            v["segments"][i]["thread_id"] = json!(i + 1);
            v["segments"][i]["start_ns"] = json!(0);
            v["segments"][i]["end_ns"] = json!(u64::MAX);
            v["segments"][i]["cpu_start_ns"] = json!(0);
            v["segments"][i]["cpu_end_ns"] = json!(u64::MAX / 2 + 1);
            v["segments"][i]["cpu_ns"] = json!(u64::MAX / 2 + 1);
        }
        assert!(collect(&v).is_err());
    }
}
