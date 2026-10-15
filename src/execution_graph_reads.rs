//! Strict validation of the native direct-read slice. This is not full semantic coverage.
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_EVENTS: usize = 100_000;
const BOUNDARIES: &[&str] = &[
    "execution-request-read-binding",
    "previous-session-promotion",
    "feed-producer-provenance",
    "missing-consumer-context",
    "no-implicit-context-reads",
    "nonquery",
];
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadCapture {
    schema: u32,
    records_complete: bool,
    semantic_complete: bool,
    full_pipeline_complete: bool,
    dep_node_namespace: String,
    request_high_water: u64,
    read_high_water: u64,
    result_high_water: u64,
    expected_requests: u64,
    expected_reads: u64,
    expected_results: u64,
    lost_events: u64,
    opaque_boundaries: Vec<String>,
    events: Vec<ReadEvent>,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    RequestStarted,
    RequestFinished,
    RequestInterrupted,
    CacheHit,
    DepRead,
    ResultProduced,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Get,
    EnsureOk,
    EnsureResult,
    Feed,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Value,
    Ensured,
    GreenEnsure,
    Fed,
    Interrupted,
    TrackedNew,
    TrackedDuplicate,
    EvalAlways,
    Ignore,
    Disabled,
    MissingContext,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadEvent {
    kind: Kind,
    #[serde(deserialize_with = "nullable")]
    request_id: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    read_id: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    result_id: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    context_id: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    consumer_job_id: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    producer_job_id: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    dep_node_index: Option<u32>,
    #[serde(deserialize_with = "nullable")]
    mode: Option<Mode>,
    #[serde(deserialize_with = "nullable")]
    outcome: Option<Outcome>,
}
#[derive(Debug, Serialize)]
pub(crate) struct ReadAssessment {
    pub(crate) records_complete: bool,
    pub(crate) endpoints_complete: bool,
    pub(crate) semantic_complete: bool,
    pub(crate) observed_requests: u64,
    pub(crate) observed_reads: u64,
    pub(crate) observed_results: u64,
    pub(crate) tracked_reads: u64,
    /// Individual observed reads resolved with prior provenance, including duplicates
    /// and self edges. Not a measure of full-pipeline semantic coverage.
    pub(crate) resolved_reads: u64,
    pub(crate) resolved_job_edges: u64,
    pub(crate) unbound_reads: u64,
    pub(crate) unbound_read_reasons: UnboundReadReasons,
    pub(crate) cache_hits: u64,
    pub(crate) green_ensures: u64,
    pub(crate) lost_events: u64,
    pub(crate) opaque_boundaries: Vec<String>,
}

/// Mutually exclusive reasons for observed direct reads only. Precedence is
/// exact nontracked outcome, missing tracked context/consumer, absent prior
/// provenance (later publication distinguished by preinventory), then unavailable
/// producer or consumer endpoint. Later provenance never binds an earlier read,
/// even when its endpoints would be unavailable as well.
#[derive(Debug, Default, Serialize)]
pub(crate) struct UnboundReadReasons {
    pub(crate) eval_always: u64,
    pub(crate) ignore: u64,
    pub(crate) disabled: u64,
    pub(crate) missing_context: u64,
    pub(crate) tracked_missing_context_or_consumer: u64,
    pub(crate) no_prior_producer: u64,
    /// A prior producer exists, but its or the consumer's admission is unavailable.
    pub(crate) job_endpoint_unavailable: u64,
    pub(crate) producer_published_too_late: u64,
}

impl UnboundReadReasons {
    fn total(&self) -> u64 {
        self.eval_always
            + self.ignore
            + self.disabled
            + self.missing_context
            + self.tracked_missing_context_or_consumer
            + self.no_prior_producer
            + self.job_endpoint_unavailable
            + self.producer_published_too_late
    }
}

/// Admission endpoint checks are context-qualified. Missing endpoints remain unknown
/// on a lost capture, but a producer's claimed complete inventory must reconcile.
pub(crate) fn assess(
    capture: &ReadCapture,
    admitted: &BTreeSet<(u64, u64)>,
    expected_contexts: u64,
) -> Result<ReadAssessment> {
    assess_policy(capture, admitted, expected_contexts, None, MAX_EVENTS)
}

/// Schema4 supplies independent job allocators, so absent admission records are
/// unknown endpoints rather than false read-local record coverage claims.
pub(crate) fn assess_with_inventory(
    capture: &ReadCapture,
    admitted: &BTreeSet<(u64, u64)>,
    expected_contexts: u64,
    context_high_water: &BTreeMap<u64, Option<u64>>,
) -> Result<ReadAssessment> {
    assess_with_inventory_limit(
        capture,
        admitted,
        expected_contexts,
        context_high_water,
        MAX_EVENTS,
    )
}

/// Explicit opt-in budget for query-read capture events, never full pipeline coverage.
/// Strict callers retain the 100,000-event default. Budgets must fall within
/// 1..=1,000,000 events, including for empty captures.
pub(crate) fn assess_with_inventory_limit(
    capture: &ReadCapture,
    admitted: &BTreeSet<(u64, u64)>,
    expected_contexts: u64,
    context_high_water: &BTreeMap<u64, Option<u64>>,
    max_events: usize,
) -> Result<ReadAssessment> {
    assess_policy(
        capture,
        admitted,
        expected_contexts,
        Some(context_high_water),
        max_events,
    )
}

fn assess_policy(
    capture: &ReadCapture,
    admitted: &BTreeSet<(u64, u64)>,
    expected_contexts: u64,
    context_high_water: Option<&BTreeMap<u64, Option<u64>>>,
    max_events: usize,
) -> Result<ReadAssessment> {
    ensure!(
        (1..=1_000_000).contains(&max_events),
        "query-read event budget outside safety range 1..=1000000"
    );
    ensure!(capture.schema == 1, "unsupported query-read schema");
    ensure!(
        !capture.semantic_complete && !capture.full_pipeline_complete,
        "query reads cannot claim complete semantic coverage"
    );
    ensure!(
        capture.dep_node_namespace == "current-context",
        "foreign dep-node namespace"
    );
    ensure!(
        capture.events.len() <= max_events,
        "query-read event bound exceeded"
    );
    let boundaries: BTreeSet<_> = capture
        .opaque_boundaries
        .iter()
        .map(String::as_str)
        .collect();
    ensure!(
        boundaries.len() == capture.opaque_boundaries.len()
            && boundaries == BOUNDARIES.iter().copied().collect(),
        "invalid query-read opaque boundaries"
    );
    for (high, expected) in [
        (capture.request_high_water, capture.expected_requests),
        (capture.read_high_water, capture.expected_reads),
        (capture.result_high_water, capture.expected_results),
    ] {
        ensure!(
            high.checked_sub(1) == Some(expected),
            "query-read allocator inventory mismatch"
        );
    }
    let mut starts = BTreeMap::new();
    let mut terminals = BTreeMap::new();
    let mut reads = BTreeSet::new();
    let mut results = BTreeSet::new();
    let mut produced = BTreeMap::new();
    let mut unresolved_endpoint = false;
    let mut hits = 0;
    let mut green = 0;
    let mut tracked = 0;
    let mut resolved = BTreeSet::new();
    let mut resolved_reads = 0;
    let mut reasons = UnboundReadReasons::default();
    // Diagnostic only: the validated, ordered map remains the binding evidence.
    let publications: BTreeSet<_> = capture
        .events
        .iter()
        .filter(|event| event.kind == Kind::ResultProduced)
        .filter_map(|event| Some((event.context_id?, event.dep_node_index?)))
        .collect();
    for (position, event) in capture.events.iter().enumerate() {
        if let Some(request) = event.request_id {
            ensure!(
                request > 0 && request < capture.request_high_water,
                "read request outside allocator inventory"
            );
        }
        if let Some(context) = event.context_id {
            ensure!(
                context >= 1 && context <= expected_contexts,
                "foreign read context"
            );
        }
        for job in [event.consumer_job_id, event.producer_job_id]
            .into_iter()
            .flatten()
        {
            ensure!(job > 0, "zero query-read job identity");
            let context = event
                .context_id
                .ok_or_else(|| anyhow::anyhow!("job without context"))?;
            if let Some(high) = context_high_water
                .and_then(|ranges| ranges.get(&context))
                .copied()
                .flatten()
            {
                ensure!(
                    job < high,
                    "read job outside independent context allocator inventory"
                );
            }
            if !admitted.contains(&(context, job)) {
                unresolved_endpoint = true;
            }
        }
        match event.kind {
            Kind::RequestStarted
            | Kind::RequestFinished
            | Kind::RequestInterrupted
            | Kind::CacheHit => {
                ensure!(
                    event.read_id.is_none()
                        && event.result_id.is_none()
                        && event.producer_job_id.is_none()
                        && event.mode.is_some(),
                    "invalid request fields"
                );
                let id = event
                    .request_id
                    .ok_or_else(|| anyhow::anyhow!("request without identity"))?;
                ensure!(
                    id > 0 && id < capture.request_high_water,
                    "request outside allocator inventory"
                );
                match event.kind {
                    Kind::RequestStarted => {
                        ensure!(
                            event.outcome.is_none() && event.dep_node_index.is_none(),
                            "invalid request start"
                        );
                        ensure!(
                            starts.insert(id, (position, event)).is_none(),
                            "duplicate query request"
                        );
                    }
                    Kind::CacheHit => {
                        ensure!(
                            event.outcome.is_none() && event.dep_node_index.is_some(),
                            "invalid cache hit"
                        );
                        hits += 1;
                    }
                    _ => {
                        ensure!(
                            event.dep_node_index.is_none(),
                            "terminal request has dep-node"
                        );
                        let outcome = event
                            .outcome
                            .ok_or_else(|| anyhow::anyhow!("terminal without outcome"))?;
                        let valid = match event.kind {
                            Kind::RequestInterrupted => outcome == Outcome::Interrupted,
                            _ => match event.mode.unwrap() {
                                Mode::Get => outcome == Outcome::Value,
                                Mode::EnsureOk | Mode::EnsureResult => {
                                    matches!(outcome, Outcome::Ensured | Outcome::GreenEnsure)
                                }
                                Mode::Feed => outcome == Outcome::Fed,
                            },
                        };
                        ensure!(valid, "request mode/outcome mismatch");
                        if outcome == Outcome::GreenEnsure {
                            green += 1;
                        }
                        ensure!(
                            terminals.insert(id, position).is_none(),
                            "duplicate query request terminal"
                        );
                    }
                }
            }
            Kind::DepRead => {
                ensure!(
                    event.result_id.is_none()
                        && event.producer_job_id.is_none()
                        && event.mode.is_none()
                        && event.dep_node_index.is_some(),
                    "invalid dep-read fields"
                );
                let id = event
                    .read_id
                    .ok_or_else(|| anyhow::anyhow!("dep-read without identity"))?;
                ensure!(
                    id > 0 && id < capture.read_high_water && reads.insert(id),
                    "duplicate/out-of-range read"
                );
                let outcome = event
                    .outcome
                    .ok_or_else(|| anyhow::anyhow!("dep-read without outcome"))?;
                ensure!(
                    matches!(
                        outcome,
                        Outcome::TrackedNew
                            | Outcome::TrackedDuplicate
                            | Outcome::EvalAlways
                            | Outcome::Ignore
                            | Outcome::Disabled
                            | Outcome::MissingContext
                    ),
                    "invalid dep-read outcome"
                );
                if matches!(outcome, Outcome::TrackedNew | Outcome::TrackedDuplicate) {
                    tracked += 1;
                    if let (Some(context), Some(consumer), Some(index)) = (
                        event.context_id,
                        event.consumer_job_id,
                        event.dep_node_index,
                    ) {
                        if let Some(&(producer, producer_position)) =
                            produced.get(&(context, index))
                        {
                            // Publication follows actual completion of this result. A later
                            // provenance record cannot retroactively prove an earlier read.
                            if producer_position < position
                                && admitted.contains(&(context, producer))
                                && admitted.contains(&(context, consumer))
                            {
                                resolved_reads += 1;
                                resolved.insert((context, producer, consumer, index));
                            } else {
                                reasons.job_endpoint_unavailable += 1;
                            }
                        } else if publications.contains(&(context, index)) {
                            reasons.producer_published_too_late += 1;
                        } else {
                            reasons.no_prior_producer += 1;
                        }
                    } else {
                        reasons.tracked_missing_context_or_consumer += 1;
                    }
                } else {
                    match outcome {
                        Outcome::EvalAlways => reasons.eval_always += 1,
                        Outcome::Ignore => reasons.ignore += 1,
                        Outcome::Disabled => reasons.disabled += 1,
                        Outcome::MissingContext => reasons.missing_context += 1,
                        _ => unreachable!("dep-read outcome validated above"),
                    }
                }
            }
            Kind::ResultProduced => {
                ensure!(
                    event.request_id.is_none()
                        && event.read_id.is_none()
                        && event.consumer_job_id.is_none()
                        && event.mode.is_none()
                        && event.outcome.is_none(),
                    "invalid result provenance fields"
                );
                let id = event
                    .result_id
                    .ok_or_else(|| anyhow::anyhow!("result without identity"))?;
                ensure!(
                    id > 0 && id < capture.result_high_water && results.insert(id),
                    "duplicate/out-of-range result"
                );
                let context = event
                    .context_id
                    .ok_or_else(|| anyhow::anyhow!("result without context"))?;
                let job = event
                    .producer_job_id
                    .ok_or_else(|| anyhow::anyhow!("result without producer"))?;
                let index = event
                    .dep_node_index
                    .ok_or_else(|| anyhow::anyhow!("result without dep-node"))?;
                ensure!(
                    produced.insert((context, index), (job, position)).is_none(),
                    "duplicate dep-node producer binding"
                );
            }
        }
    }
    let mut incomplete =
        capture.lost_events != 0 || (context_high_water.is_none() && unresolved_endpoint);
    for (position, event) in capture.events.iter().enumerate() {
        if let Some(id) = event.request_id {
            if let Some(&(start_position, start)) = starts.get(&id) {
                ensure!(
                    event.context_id == start.context_id
                        && event.consumer_job_id == start.consumer_job_id,
                    "foreign request endpoint"
                );
                if event.kind != Kind::DepRead {
                    ensure!(event.mode == start.mode, "request mode changed");
                }
                ensure!(position >= start_position, "request event precedes start");
                if let Some(&end) = terminals.get(&id) {
                    ensure!(position <= end, "request event follows terminal");
                } else {
                    incomplete = true;
                }
            } else {
                incomplete = true;
            }
        }
    }
    let contiguous = |ids: &BTreeSet<u64>, expected: u64| {
        ids.len() as u64 == expected && ids.iter().enumerate().all(|(i, &id)| id == i as u64 + 1)
    };
    let start_ids: BTreeSet<_> = starts.keys().copied().collect();
    let end_ids: BTreeSet<_> = terminals.keys().copied().collect();
    incomplete |= !contiguous(&start_ids, capture.expected_requests)
        || start_ids != end_ids
        || !contiguous(&reads, capture.expected_reads)
        || !contiguous(&results, capture.expected_results);
    if capture.records_complete && incomplete {
        bail!("query-read records_complete contradicts independent inventory/endpoints");
    }
    Ok(ReadAssessment {
        records_complete: capture.records_complete && !incomplete,
        endpoints_complete: !unresolved_endpoint,
        semantic_complete: false,
        observed_requests: starts.len() as u64,
        observed_reads: reads.len() as u64,
        observed_results: results.len() as u64,
        tracked_reads: tracked,
        resolved_reads,
        resolved_job_edges: resolved.len() as u64,
        unbound_reads: reasons.total(),
        unbound_read_reasons: reasons,
        cache_hits: hits,
        green_ensures: green,
        lost_events: capture.lost_events,
        opaque_boundaries: capture.opaque_boundaries.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn event(kind: &str) -> Value {
        json!({"kind":kind,"request_id":null,"read_id":null,"result_id":null,"context_id":1,"consumer_job_id":null,"producer_job_id":null,"dep_node_index":null,"mode":null,"outcome":null})
    }
    fn fixture() -> Value {
        let mut start = event("request_started");
        start["request_id"] = json!(1);
        start["consumer_job_id"] = json!(2);
        start["mode"] = json!("get");
        let mut produced = event("result_produced");
        produced["result_id"] = json!(1);
        produced["producer_job_id"] = json!(1);
        produced["dep_node_index"] = json!(42);
        let mut read = event("dep_read");
        read["request_id"] = json!(1);
        read["read_id"] = json!(1);
        read["consumer_job_id"] = json!(2);
        read["dep_node_index"] = json!(42);
        read["outcome"] = json!("tracked_new");
        let mut end = start.clone();
        end["kind"] = json!("request_finished");
        end["outcome"] = json!("value");
        json!({"schema":1,"records_complete":true,"semantic_complete":false,"full_pipeline_complete":false,"dep_node_namespace":"current-context","request_high_water":2,"read_high_water":2,"result_high_water":2,"expected_requests":1,"expected_reads":1,"expected_results":1,"lost_events":0,"opaque_boundaries":BOUNDARIES,"events":[start,produced,read,end]})
    }
    fn run(value: Value) -> Result<ReadAssessment> {
        let capture: ReadCapture = serde_json::from_value(value)?;
        assess(&capture, &BTreeSet::from([(1, 1), (1, 2)]), 1)
    }
    fn assert_partition(report: &ReadAssessment) {
        assert_eq!(report.unbound_reads, report.unbound_read_reasons.total());
        assert_eq!(
            report.observed_reads,
            report.resolved_reads + report.unbound_reads
        );
    }

    #[test]
    fn all_read_outcomes_partition_with_duplicates_and_self_edges() {
        let mut value = fixture();
        let mut events = vec![value["events"][1].clone()];
        for (i, outcome) in [
            "tracked_new",
            "tracked_duplicate",
            "eval_always",
            "ignore",
            "disabled",
            "missing_context",
        ]
        .iter()
        .enumerate()
        {
            let mut read = value["events"][2].clone();
            read["request_id"] = Value::Null;
            read["read_id"] = json!(i + 1);
            read["consumer_job_id"] = json!(1);
            read["outcome"] = json!(outcome);
            events.push(read);
        }
        value["events"] = json!(events);
        value["expected_requests"] = json!(0);
        value["request_high_water"] = json!(1);
        value["expected_reads"] = json!(6);
        value["read_high_water"] = json!(7);
        let report = run(value).unwrap();
        assert_partition(&report);
        assert_eq!(report.resolved_reads, 2);
        assert_eq!(report.resolved_job_edges, 1);
        assert_eq!(report.tracked_reads, 2);
        let reasons = report.unbound_read_reasons;
        assert_eq!(
            (
                reasons.eval_always,
                reasons.ignore,
                reasons.disabled,
                reasons.missing_context
            ),
            (1, 1, 1, 1)
        );
    }

    #[test]
    fn reason_precedence_and_late_publication_never_invent_edges() {
        for (later, missing_consumer, outcome, expected) in [
            (true, false, "tracked_new", "producer_published_too_late"),
            (false, false, "tracked_duplicate", "no_prior_producer"),
            (
                true,
                true,
                "tracked_new",
                "tracked_missing_context_or_consumer",
            ),
            (true, true, "ignore", "ignore"),
        ] {
            let mut value = fixture();
            let mut read = value["events"][2].clone();
            read["request_id"] = Value::Null;
            read["outcome"] = json!(outcome);
            if missing_consumer {
                read["consumer_job_id"] = Value::Null;
                read["context_id"] = Value::Null;
            }
            let mut events = vec![read];
            if later {
                events.push(value["events"][1].clone());
            }
            value["events"] = json!(events);
            value["expected_requests"] = json!(0);
            value["request_high_water"] = json!(1);
            value["expected_results"] = json!(u64::from(later));
            value["result_high_water"] = json!(1 + u64::from(later));
            let capture: ReadCapture = serde_json::from_value(value).unwrap();
            // Missing producer admission must not override the earlier reason.
            let report = assess_with_inventory(
                &capture,
                &BTreeSet::from([(1, 2)]),
                1,
                &BTreeMap::from([(1, Some(3))]),
            )
            .unwrap();
            assert_partition(&report);
            assert_eq!(report.resolved_reads, 0);
            assert_eq!(report.resolved_job_edges, 0);
            assert_eq!(
                serde_json::to_value(report.unbound_read_reasons).unwrap()[expected],
                json!(1)
            );
        }
    }

    #[test]
    fn explicit_inventory_budget_is_inclusive_and_hard_bounded() {
        let mut capture: ReadCapture = serde_json::from_value(fixture()).unwrap();
        let admitted = BTreeSet::from([(1, 1), (1, 2)]);
        let ranges = BTreeMap::from([(1, Some(3))]);
        assert!(assess_with_inventory_limit(&capture, &admitted, 1, &ranges, 4).is_ok());
        for budget in [0, 3, 1_000_001, usize::MAX] {
            assert!(assess_with_inventory_limit(&capture, &admitted, 1, &ranges, budget).is_err());
        }
        capture.expected_requests = 0;
        capture.request_high_water = 1;
        capture.expected_results = 0;
        capture.result_high_water = 1;
        capture.expected_reads = 0;
        capture.read_high_water = 1;
        capture.events.clear();
        assert!(assess_with_inventory_limit(&capture, &admitted, 1, &ranges, 1).is_ok());
        assert!(
            assess_with_inventory_limit(&capture, &admitted, 1, &ranges, 0)
                .unwrap_err()
                .to_string()
                .contains("event budget outside safety range")
        );
        capture.expected_reads = 1_000_000;
        capture.read_high_water = 1_000_001;
        capture.events = (1..=1_000_000)
            .map(|id| ReadEvent {
                kind: Kind::DepRead,
                request_id: None,
                read_id: Some(id),
                result_id: None,
                context_id: None,
                consumer_job_id: None,
                producer_job_id: None,
                dep_node_index: Some(42),
                mode: None,
                outcome: Some(Outcome::Disabled),
            })
            .collect();
        assert!(assess(&capture, &admitted, 1).is_err());
        assert!(assess_with_inventory(&capture, &admitted, 1, &ranges).is_err());
        let report =
            assess_with_inventory_limit(&capture, &admitted, 1, &ranges, 1_000_000).unwrap();
        assert_partition(&report);
        assert!(report.records_complete);
        assert_eq!(report.unbound_read_reasons.disabled, 1_000_000);
        capture.events.push(ReadEvent {
            kind: Kind::DepRead,
            request_id: None,
            read_id: None,
            result_id: None,
            context_id: None,
            consumer_job_id: None,
            producer_job_id: None,
            dep_node_index: None,
            mode: None,
            outcome: None,
        });
        assert!(
            assess_with_inventory_limit(&capture, &admitted, 1, &ranges, 1_000_000)
                .unwrap_err()
                .to_string()
                .contains("event bound exceeded")
        );
    }

    #[test]
    fn default_event_budget_is_unchanged_and_inclusive() {
        let mut value = fixture();
        let mut events = Vec::with_capacity(MAX_EVENTS + 1);
        for id in 1..=MAX_EVENTS {
            let mut read = value["events"][2].clone();
            read["request_id"] = Value::Null;
            read["read_id"] = json!(id);
            read["outcome"] = json!("ignore");
            events.push(read);
        }
        value["events"] = json!(events);
        value["expected_requests"] = json!(0);
        value["request_high_water"] = json!(1);
        value["expected_results"] = json!(0);
        value["result_high_water"] = json!(1);
        value["expected_reads"] = json!(MAX_EVENTS);
        value["read_high_water"] = json!(MAX_EVENTS + 1);
        let report = run(value.clone()).unwrap();
        assert_partition(&report);
        assert_eq!(report.unbound_read_reasons.ignore, MAX_EVENTS as u64);
        let extra = value["events"][0].clone();
        value["events"].as_array_mut().unwrap().push(extra);
        assert!(
            run(value)
                .unwrap_err()
                .to_string()
                .contains("event bound exceeded")
        );
    }

    #[test]
    fn schema4_missing_admission_is_unknown_not_read_record_loss() {
        let capture: ReadCapture = serde_json::from_value(fixture()).unwrap();
        let ranges = BTreeMap::from([(1, Some(3))]);
        let report =
            assess_with_inventory(&capture, &BTreeSet::from([(1, 2)]), 1, &ranges).unwrap();
        assert!(report.records_complete);
        assert!(!report.endpoints_complete && !report.semantic_complete);
        assert_eq!(report.resolved_job_edges, 0);
        assert_eq!(report.unbound_reads, 1);
        assert_eq!(report.unbound_read_reasons.job_endpoint_unavailable, 1);
        assert_partition(&report);
        for admitted in [BTreeSet::from([(1, 1)]), BTreeSet::new()] {
            let report = assess_with_inventory(&capture, &admitted, 1, &ranges).unwrap();
            assert_partition(&report);
            assert_eq!(report.unbound_read_reasons.job_endpoint_unavailable, 1);
        }
        assert!(assess(&capture, &BTreeSet::from([(1, 2)]), 1).is_err());
        let mut value = fixture();
        value["events"][1]["producer_job_id"] = json!(99);
        let capture: ReadCapture = serde_json::from_value(value).unwrap();
        assert!(assess_with_inventory(&capture, &BTreeSet::from([(1, 2)]), 1, &ranges).is_err());
    }

    #[test]
    fn resolved_native_binding_is_not_full_semantics() {
        let report = run(fixture()).unwrap();
        assert!(report.records_complete);
        assert_eq!(report.tracked_reads, 1);
        assert_eq!(report.resolved_job_edges, 1);
        assert!(!report.semantic_complete);
    }
    #[test]
    fn no_producer_or_untracked_does_not_resolve() {
        let mut value = fixture();
        value["events"][2]["outcome"] = json!("ignore");
        assert_eq!(run(value).unwrap().resolved_job_edges, 0);
        let mut value = fixture();
        value["events"].as_array_mut().unwrap().remove(1);
        value["expected_results"] = json!(0);
        value["result_high_water"] = json!(1);
        assert_eq!(run(value).unwrap().resolved_job_edges, 0);
    }
    #[test]
    fn strict_shape_and_semantic_claims_rejected() {
        let mut value = fixture();
        value["unknown"] = json!(0);
        assert!(run(value).is_err());
        let mut value = fixture();
        value["events"][0]
            .as_object_mut()
            .unwrap()
            .remove("read_id");
        assert!(run(value).is_err());
        let mut value = fixture();
        value["semantic_complete"] = json!(true);
        assert!(run(value).is_err());
        let mut value = fixture();
        value["events"][2]["outcome"] = json!("foreign");
        assert!(run(value).is_err());
    }
    #[test]
    fn independent_inventory_duplicate_and_foreign_rejected() {
        let mut value = fixture();
        value["events"].as_array_mut().unwrap().remove(2);
        assert!(run(value).is_err());
        let mut value = fixture();
        let read = value["events"][2].clone();
        value["events"].as_array_mut().unwrap().insert(2, read);
        assert!(run(value).is_err());
        let mut value = fixture();
        value["events"][2]["context_id"] = json!(2);
        assert!(run(value).is_err());
        let mut value = fixture();
        value["events"][1]["producer_job_id"] = json!(99);
        assert!(run(value).is_err());
        let mut value = fixture();
        value["events"].as_array_mut().unwrap().swap(0, 3);
        assert!(run(value).is_err());
    }
    #[test]
    fn explicit_loss_is_incomplete_not_false_complete() {
        let mut value = fixture();
        value["records_complete"] = json!(false);
        value["lost_events"] = json!(1);
        value["events"].as_array_mut().unwrap().remove(2);
        let report = run(value).unwrap();
        assert!(!report.records_complete);
        assert!(!report.semantic_complete);
    }
}
