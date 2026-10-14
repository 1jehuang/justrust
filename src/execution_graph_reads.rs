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
    pub(crate) semantic_complete: bool,
    pub(crate) observed_requests: u64,
    pub(crate) observed_reads: u64,
    pub(crate) observed_results: u64,
    pub(crate) tracked_reads: u64,
    pub(crate) resolved_job_edges: u64,
    pub(crate) unbound_reads: u64,
    pub(crate) cache_hits: u64,
    pub(crate) green_ensures: u64,
    pub(crate) lost_events: u64,
    pub(crate) opaque_boundaries: Vec<String>,
}

/// Admission endpoint checks are context-qualified. Missing endpoints remain unknown
/// on a lost capture, but a producer's claimed complete inventory must reconcile.
pub(crate) fn assess(
    capture: &ReadCapture,
    admitted: &BTreeSet<(u64, u64)>,
    expected_contexts: u64,
) -> Result<ReadAssessment> {
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
        capture.events.len() <= MAX_EVENTS,
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
    let mut unbound = 0;
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
                                resolved.insert((context, producer, consumer, index));
                            } else {
                                unbound += 1;
                            }
                        } else {
                            unbound += 1;
                        }
                    } else {
                        unbound += 1;
                    }
                } else {
                    unbound += 1;
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
    let mut incomplete = capture.lost_events != 0 || unresolved_endpoint;
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
        semantic_complete: false,
        observed_requests: starts.len() as u64,
        observed_reads: reads.len() as u64,
        observed_results: results.len() as u64,
        tracked_reads: tracked,
        resolved_job_edges: resolved.len() as u64,
        unbound_reads: unbound,
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
