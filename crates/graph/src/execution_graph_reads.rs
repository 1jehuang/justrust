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

/// Explicit opt-in wire grammar. Do not flatten the legacy event: duplicate,
/// unknown, and missing nullable fields must be rejected on original bytes.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadCaptureV2 {
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
    events: Vec<ReadEventV2>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadEventV2 {
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
    #[serde(deserialize_with = "nullable")]
    resolution: Option<DispatchResolution>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DispatchResolution {
    source: DispatchResolutionSource,
    #[serde(deserialize_with = "nullable")]
    dep_node_index: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DispatchResolutionSource {
    ResolvedDispatch,
}

/// Actual terminal result availability, not fresh execution, prerequisites, or
/// a whole-job DAG. Record completeness describes only the read event stream.
#[derive(Debug, Serialize)]
pub(crate) struct DispatchResultAssessment {
    pub(crate) scope: &'static str,
    pub(crate) records_complete: bool,
    pub(crate) observed_requests: u64,
    pub(crate) observed_dispatch_resolutions: u64,
    pub(crate) resolved_requests: u64,
    pub(crate) unresolved_requests: u64,
    pub(crate) distinct_publishing_job_relations: u64,
    pub(crate) lost_events: u64,
    pub(crate) unresolved_reasons: DispatchResultUnresolvedReasons,
    pub(crate) relations: Vec<DispatchResultRelation>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DispatchResultRelation {
    pub(crate) request_id: u64,
    pub(crate) context_id: u64,
    pub(crate) consumer_job_id: u64,
    pub(crate) result_id: u64,
    pub(crate) producer_job_id: u64,
    pub(crate) dep_node_index: u32,
    /// Zero-based indices in the original, unchanged single event stream.
    pub(crate) request_start_event_index: usize,
    pub(crate) publication_event_index: usize,
    pub(crate) request_terminal_event_index: usize,
}

/// Mutually exclusive partition of observed request starts, in field order.
/// Forbidden resolution metadata is a grammar error, not an unresolved route.
#[derive(Debug, Default, Serialize)]
pub(crate) struct DispatchResultUnresolvedReasons {
    pub(crate) feed_request: u64,
    pub(crate) interrupted_request: u64,
    pub(crate) missing_request_terminal: u64,
    pub(crate) cache_hit_request: u64,
    pub(crate) no_dispatch_resolution: u64,
    pub(crate) null_dep_node_index: u64,
    pub(crate) missing_context_or_consumer: u64,
    pub(crate) capture_incomplete: u64,
    pub(crate) no_prior_producer: u64,
    pub(crate) producer_published_too_late: u64,
    pub(crate) job_endpoint_unavailable: u64,
    pub(crate) admission_authority_incomplete: u64,
}

impl DispatchResultAssessment {
    pub(crate) fn suppress_incomplete_admission_authority(&mut self) {
        self.unresolved_reasons.admission_authority_incomplete += self.resolved_requests;
        self.unresolved_requests += self.resolved_requests;
        self.resolved_requests = 0;
        self.distinct_publishing_job_relations = 0;
        self.relations.clear();
    }
}

impl ReadCaptureV2 {
    /// Validate original typed rows, then move every legacy field into the base
    /// grammar without filtering, reordering, or a serialization roundtrip.
    pub(crate) fn into_base_and_assess(
        self,
        admitted: &BTreeSet<(u64, u64)>,
        expected_contexts: u64,
        context_high_water: &BTreeMap<u64, Option<u64>>,
        max_events: usize,
    ) -> Result<(ReadCapture, DispatchResultAssessment)> {
        ensure!(self.schema == 2, "unsupported dispatch query-read schema");
        ensure!(
            (1..=1_000_000).contains(&max_events),
            "query-read event budget outside safety range 1..=1000000"
        );
        ensure!(
            self.events.len() <= max_events,
            "query-read event bound exceeded"
        );
        let mut events = Vec::with_capacity(self.events.len());
        let mut resolutions = Vec::with_capacity(self.events.len());
        for event in self.events {
            resolutions.push(event.resolution);
            events.push(ReadEvent {
                kind: event.kind,
                request_id: event.request_id,
                read_id: event.read_id,
                result_id: event.result_id,
                context_id: event.context_id,
                consumer_job_id: event.consumer_job_id,
                producer_job_id: event.producer_job_id,
                dep_node_index: event.dep_node_index,
                mode: event.mode,
                outcome: event.outcome,
            });
        }
        let base = ReadCapture {
            schema: 1,
            records_complete: self.records_complete,
            semantic_complete: self.semantic_complete,
            full_pipeline_complete: self.full_pipeline_complete,
            dep_node_namespace: self.dep_node_namespace,
            request_high_water: self.request_high_water,
            read_high_water: self.read_high_water,
            result_high_water: self.result_high_water,
            expected_requests: self.expected_requests,
            expected_reads: self.expected_reads,
            expected_results: self.expected_results,
            lost_events: self.lost_events,
            opaque_boundaries: self.opaque_boundaries,
            events,
        };
        let read = assess_with_inventory_limit(
            &base,
            admitted,
            expected_contexts,
            context_high_water,
            max_events,
        )?;
        let mut starts = BTreeMap::new();
        let mut terminals = BTreeMap::new();
        let mut hits = BTreeSet::new();
        let mut publications = BTreeMap::new();
        for (position, event) in base.events.iter().enumerate() {
            match event.kind {
                Kind::RequestStarted => {
                    starts.insert(event.request_id.unwrap(), (position, event));
                }
                Kind::RequestFinished | Kind::RequestInterrupted => {
                    terminals.insert(event.request_id.unwrap(), (position, event));
                }
                Kind::CacheHit => {
                    hits.insert(event.request_id.unwrap());
                }
                Kind::ResultProduced => {
                    publications.insert(
                        (event.context_id.unwrap(), event.dep_node_index.unwrap()),
                        (position, event),
                    );
                }
                Kind::DepRead => {}
            }
        }
        let mut observed_dispatch_resolutions = 0;
        for (position, resolution) in resolutions.iter().enumerate() {
            if let Some(resolution) = resolution {
                match resolution.source {
                    DispatchResolutionSource::ResolvedDispatch => {}
                }
                let event = &base.events[position];
                ensure!(
                    event.kind == Kind::RequestFinished
                        && matches!(
                            (event.mode, event.outcome),
                            (Some(Mode::Get), Some(Outcome::Value))
                                | (
                                    Some(Mode::EnsureOk | Mode::EnsureResult),
                                    Some(Outcome::Ensured)
                                )
                        ),
                    "dispatch resolution on incompatible event or return route"
                );
                ensure!(
                    !hits.contains(&event.request_id.unwrap()),
                    "dispatch resolution coexists with cache hit"
                );
                observed_dispatch_resolutions += 1;
            }
        }
        let mut report = DispatchResultAssessment {
            scope: "observed-dispatch-result-availability-not-execution-dag",
            records_complete: read.records_complete,
            observed_requests: read.observed_requests,
            observed_dispatch_resolutions,
            resolved_requests: 0,
            unresolved_requests: 0,
            distinct_publishing_job_relations: 0,
            lost_events: read.lost_events,
            unresolved_reasons: DispatchResultUnresolvedReasons::default(),
            relations: Vec::new(),
        };
        let mut job_relations = BTreeSet::new();
        for (&request_id, &(start_position, start)) in &starts {
            let reasons = &mut report.unresolved_reasons;
            let reason = if start.mode == Some(Mode::Feed) {
                Some(&mut reasons.feed_request)
            } else if let Some(&(terminal_position, terminal)) = terminals.get(&request_id) {
                if terminal.kind == Kind::RequestInterrupted {
                    Some(&mut reasons.interrupted_request)
                } else if hits.contains(&request_id) {
                    Some(&mut reasons.cache_hit_request)
                } else if let Some(resolution) = &resolutions[terminal_position] {
                    if let Some(index) = resolution.dep_node_index {
                        if let (Some(context), Some(consumer)) =
                            (start.context_id, start.consumer_job_id)
                        {
                            if !read.records_complete {
                                Some(&mut reasons.capture_incomplete)
                            } else if let Some(&(publication_position, publication)) =
                                publications.get(&(context, index))
                            {
                                if publication_position >= terminal_position {
                                    Some(&mut reasons.producer_published_too_late)
                                } else if let Some(producer) =
                                    publication.producer_job_id.filter(|producer| {
                                        admitted.contains(&(context, *producer))
                                            && admitted.contains(&(context, consumer))
                                    })
                                {
                                    job_relations.insert((context, producer, consumer));
                                    report.relations.push(DispatchResultRelation {
                                        request_id,
                                        context_id: context,
                                        consumer_job_id: consumer,
                                        result_id: publication.result_id.unwrap(),
                                        producer_job_id: producer,
                                        dep_node_index: index,
                                        request_start_event_index: start_position,
                                        publication_event_index: publication_position,
                                        request_terminal_event_index: terminal_position,
                                    });
                                    None
                                } else {
                                    Some(&mut reasons.job_endpoint_unavailable)
                                }
                            } else {
                                Some(&mut reasons.no_prior_producer)
                            }
                        } else {
                            Some(&mut reasons.missing_context_or_consumer)
                        }
                    } else {
                        Some(&mut reasons.null_dep_node_index)
                    }
                } else {
                    Some(&mut reasons.no_dispatch_resolution)
                }
            } else {
                Some(&mut reasons.missing_request_terminal)
            };
            if let Some(reason) = reason {
                *reason += 1;
            }
        }
        report.resolved_requests = report.relations.len() as u64;
        report.unresolved_requests = report.observed_requests - report.resolved_requests;
        report.distinct_publishing_job_relations = job_relations.len() as u64;
        Ok((base, report))
    }
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

/// Observed result availability only, never an execution DAG or semantic coverage.
#[derive(Debug, Serialize)]
pub(crate) struct CacheCallAssessment {
    pub(crate) scope: &'static str,
    pub(crate) records_complete: bool,
    pub(crate) observed_requests: u64,
    pub(crate) observed_cache_hits: u64,
    pub(crate) orphan_cache_hits: u64,
    pub(crate) resolved_requests: u64,
    pub(crate) unresolved_requests: u64,
    pub(crate) distinct_publishing_job_relations: u64,
    pub(crate) lost_events: u64,
    pub(crate) unresolved_reasons: CacheCallUnresolvedReasons,
    pub(crate) relations: Vec<CacheCallRelation>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CacheCallRelation {
    pub(crate) request_id: u64,
    pub(crate) context_id: u64,
    pub(crate) consumer_job_id: u64,
    pub(crate) result_id: u64,
    pub(crate) producer_job_id: u64,
    pub(crate) dep_node_index: u32,
    /// All positions are zero-based indices in the original mutex-ordered stream.
    pub(crate) request_start_event_index: usize,
    pub(crate) publication_event_index: usize,
    pub(crate) cache_hit_event_index: usize,
    pub(crate) request_terminal_event_index: usize,
}

/// Exclusive precedence: feed, missing/interrupted terminal, multiple/no hits,
/// incompatible return route, missing context/consumer, incomplete capture,
/// absent/later publication, then unavailable endpoints. Admission suppression
/// applies only to otherwise resolved rows. No later publication binds a hit.
#[derive(Debug, Default, Serialize)]
pub(crate) struct CacheCallUnresolvedReasons {
    pub(crate) feed_request: u64,
    pub(crate) interrupted_request: u64,
    pub(crate) missing_request_terminal: u64,
    pub(crate) ambiguous_cache_hits: u64,
    pub(crate) no_cache_hit: u64,
    pub(crate) incompatible_cache_hit_route: u64,
    pub(crate) missing_context_or_consumer: u64,
    pub(crate) capture_incomplete: u64,
    pub(crate) no_prior_producer: u64,
    pub(crate) producer_published_too_late: u64,
    pub(crate) job_endpoint_unavailable: u64,
    pub(crate) admission_authority_incomplete: u64,
}

impl CacheCallAssessment {
    pub(crate) fn suppress_incomplete_admission_authority(&mut self) {
        self.unresolved_reasons.admission_authority_incomplete += self.resolved_requests;
        self.unresolved_requests += self.resolved_requests;
        self.resolved_requests = 0;
        self.distinct_publishing_job_relations = 0;
        self.relations.clear();
    }
}

/// Explicit query4/5 analysis. Strict validation runs before any additional maps
/// are allocated. Maps and relation rows are bounded by the validated event budget,
/// not allocator high-water values. Loss suppresses certification of uniqueness.
pub(crate) fn assess_cache_calls_with_inventory_limit(
    capture: &ReadCapture,
    admitted: &BTreeSet<(u64, u64)>,
    expected_contexts: u64,
    context_high_water: &BTreeMap<u64, Option<u64>>,
    max_events: usize,
) -> Result<CacheCallAssessment> {
    let read = assess_policy(
        capture,
        admitted,
        expected_contexts,
        Some(context_high_water),
        max_events,
    )?;
    let mut starts = BTreeMap::new();
    let mut terminals = BTreeMap::new();
    let mut hits = BTreeMap::<u64, (usize, &ReadEvent, usize)>::new();
    let mut publications = BTreeMap::new();
    for (position, event) in capture.events.iter().enumerate() {
        match event.kind {
            Kind::RequestStarted => {
                starts.insert(event.request_id.unwrap(), (position, event));
            }
            Kind::RequestFinished | Kind::RequestInterrupted => {
                terminals.insert(event.request_id.unwrap(), (position, event));
            }
            Kind::CacheHit => {
                hits.entry(event.request_id.unwrap())
                    .and_modify(|facts| facts.2 += 1)
                    .or_insert((position, event, 1));
            }
            Kind::ResultProduced => {
                if let (Some(context), Some(index)) = (event.context_id, event.dep_node_index) {
                    publications.insert((context, index), (position, event));
                }
            }
            Kind::DepRead => {}
        }
    }
    let mut report = CacheCallAssessment {
        scope: "observed-cache-result-availability-not-execution-dag",
        records_complete: read.records_complete,
        observed_requests: read.observed_requests,
        observed_cache_hits: read.cache_hits,
        orphan_cache_hits: hits
            .iter()
            .filter(|(id, _)| !starts.contains_key(id))
            .map(|(_, facts)| facts.2 as u64)
            .sum(),
        resolved_requests: 0,
        unresolved_requests: 0,
        distinct_publishing_job_relations: 0,
        lost_events: read.lost_events,
        unresolved_reasons: CacheCallUnresolvedReasons::default(),
        relations: Vec::new(),
    };
    let mut job_relations = BTreeSet::new();
    for (&request_id, &(start_position, start)) in &starts {
        let reasons = &mut report.unresolved_reasons;
        let reason = if start.mode == Some(Mode::Feed) {
            Some(&mut reasons.feed_request)
        } else if !terminals.contains_key(&request_id) {
            Some(&mut reasons.missing_request_terminal)
        } else {
            let &(terminal_position, terminal) = &terminals[&request_id];
            let hit = hits.get(&request_id);
            if terminal.kind == Kind::RequestInterrupted {
                Some(&mut reasons.interrupted_request)
            } else if hit.is_some_and(|facts| facts.2 > 1) {
                Some(&mut reasons.ambiguous_cache_hits)
            } else if let Some(&(hit_position, hit, _)) = hit {
                let route = matches!(
                    (start.mode, terminal.outcome),
                    (Some(Mode::Get), Some(Outcome::Value))
                        | (
                            Some(Mode::EnsureOk | Mode::EnsureResult),
                            Some(Outcome::Ensured)
                        )
                );
                if !route || !(start_position < hit_position && hit_position < terminal_position) {
                    Some(&mut reasons.incompatible_cache_hit_route)
                } else if let (Some(context), Some(consumer)) =
                    (start.context_id, start.consumer_job_id)
                {
                    if !read.records_complete {
                        Some(&mut reasons.capture_incomplete)
                    } else if let Some(&(publication_position, publication)) =
                        publications.get(&(context, hit.dep_node_index.unwrap()))
                    {
                        if publication_position >= hit_position {
                            Some(&mut reasons.producer_published_too_late)
                        } else if let Some(producer) =
                            publication.producer_job_id.filter(|producer| {
                                admitted.contains(&(context, *producer))
                                    && admitted.contains(&(context, consumer))
                            })
                        {
                            job_relations.insert((context, producer, consumer));
                            report.relations.push(CacheCallRelation {
                                request_id,
                                context_id: context,
                                consumer_job_id: consumer,
                                result_id: publication.result_id.unwrap(),
                                producer_job_id: producer,
                                dep_node_index: hit.dep_node_index.unwrap(),
                                request_start_event_index: start_position,
                                publication_event_index: publication_position,
                                cache_hit_event_index: hit_position,
                                request_terminal_event_index: terminal_position,
                            });
                            None
                        } else {
                            Some(&mut reasons.job_endpoint_unavailable)
                        }
                    } else {
                        Some(&mut reasons.no_prior_producer)
                    }
                } else {
                    Some(&mut reasons.missing_context_or_consumer)
                }
            } else {
                Some(&mut reasons.no_cache_hit)
            }
        };
        if let Some(reason) = reason {
            *reason += 1;
        }
    }
    report.resolved_requests = report.relations.len() as u64;
    report.unresolved_requests = report.observed_requests - report.resolved_requests;
    report.distinct_publishing_job_relations = job_relations.len() as u64;
    Ok(report)
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
    fn dispatch_fixture() -> Value {
        let mut value = fixture();
        value["schema"] = json!(2);
        for event in value["events"].as_array_mut().unwrap() {
            event["resolution"] = Value::Null;
        }
        value["events"][3]["resolution"] =
            json!({"source":"resolved_dispatch","dep_node_index":42});
        value
    }
    fn dispatch(
        value: Value,
        admitted: BTreeSet<(u64, u64)>,
    ) -> Result<(ReadCapture, DispatchResultAssessment)> {
        serde_json::from_value::<ReadCaptureV2>(value)?.into_base_and_assess(
            &admitted,
            1,
            &BTreeMap::from([(1, Some(3))]),
            MAX_EVENTS,
        )
    }
    fn dispatch_partition(report: &DispatchResultAssessment) {
        let reasons = serde_json::to_value(&report.unresolved_reasons).unwrap();
        let total: u64 = reasons
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_u64().unwrap())
            .sum();
        assert_eq!(total, report.unresolved_requests);
        assert_eq!(total + report.resolved_requests, report.observed_requests);
        assert_eq!(report.resolved_requests, report.relations.len() as u64);
    }
    #[test]
    fn dispatch_result_projection_preserves_legacy_metrics_and_original_ordinals() {
        let admitted = BTreeSet::from([(1, 1), (1, 2)]);
        let old = assess(&serde_json::from_value(fixture()).unwrap(), &admitted, 1).unwrap();
        let (base, mut report) = dispatch(dispatch_fixture(), admitted.clone()).unwrap();
        assert_eq!(base.events.len(), 4);
        let projected = assess(&base, &admitted, 1).unwrap();
        assert_eq!(
            serde_json::to_value(old).unwrap(),
            serde_json::to_value(projected).unwrap()
        );
        dispatch_partition(&report);
        assert_eq!(report.observed_dispatch_resolutions, 1);
        assert_eq!(report.resolved_requests, 1);
        assert_eq!(
            report.scope,
            "observed-dispatch-result-availability-not-execution-dag"
        );
        let row = &report.relations[0];
        assert_eq!(
            (
                row.request_start_event_index,
                row.publication_event_index,
                row.request_terminal_event_index
            ),
            (0, 1, 3)
        );
        assert_eq!(
            (
                row.request_id,
                row.context_id,
                row.consumer_job_id,
                row.result_id,
                row.producer_job_id,
                row.dep_node_index
            ),
            (1, 1, 2, 1, 1, 42)
        );
        report.suppress_incomplete_admission_authority();
        report.suppress_incomplete_admission_authority();
        dispatch_partition(&report);
        assert_eq!(report.unresolved_reasons.admission_authority_incomplete, 1);
        assert_eq!(report.distinct_publishing_job_relations, 0);
    }
    #[test]
    fn dispatch_result_nullable_fields_are_mandatory_and_wire_is_strict() {
        for position in 0..4 {
            let mut value = dispatch_fixture();
            value["events"][position]
                .as_object_mut()
                .unwrap()
                .remove("resolution");
            assert!(serde_json::from_value::<ReadCaptureV2>(value).is_err());
        }
        for field in ["source", "dep_node_index"] {
            let mut value = dispatch_fixture();
            value["events"][3]["resolution"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(serde_json::from_value::<ReadCaptureV2>(value).is_err());
        }
        for field in [
            "request_id",
            "read_id",
            "result_id",
            "context_id",
            "consumer_job_id",
            "producer_job_id",
            "dep_node_index",
            "mode",
            "outcome",
        ] {
            let mut value = dispatch_fixture();
            value["events"][0].as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<ReadCaptureV2>(value).is_err());
        }
        for (pointer, replacement) in [
            ("/events/3/resolution/source", json!("fresh_provider")),
            ("/events/3/resolution/extra", json!(null)),
            ("/events/0/extra", json!(null)),
            ("/extra", json!(null)),
            ("/events/3/resolution/dep_node_index", json!(-1)),
            ("/events/3/resolution/dep_node_index", json!(4294967296_u64)),
        ] {
            let mut value = dispatch_fixture();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            value
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(key.to_owned(), replacement);
            assert!(
                serde_json::from_value::<ReadCaptureV2>(value).is_err(),
                "{pointer}"
            );
        }
        let raw = serde_json::to_string(&dispatch_fixture()).unwrap();
        for (needle, replacement) in [
            (
                "\"resolution\":null",
                "\"resolution\":null,\"resolution\":null",
            ),
            (
                "\"source\":\"resolved_dispatch\"",
                "\"source\":\"resolved_dispatch\",\"source\":\"resolved_dispatch\"",
            ),
            (
                "\"dep_node_index\":42",
                "\"dep_node_index\":42,\"dep_node_index\":42",
            ),
        ] {
            let duplicate = raw.replacen(needle, replacement, 1);
            assert_ne!(raw, duplicate);
            assert!(serde_json::from_str::<ReadCaptureV2>(&duplicate).is_err());
        }
        assert!(
            serde_json::from_str::<DispatchResolution>(
                r#"{"source":"resolved_dispatch","dep_node_index":42,"dep_node_index":43}"#
            )
            .is_err()
        );
        assert!(serde_json::from_value::<ReadCapture>(dispatch_fixture()).is_err());
    }
    #[test]
    fn dispatch_result_forbidden_metadata_routes_are_errors_even_with_loss() {
        for position in [0, 1, 2] {
            let mut value = dispatch_fixture();
            value["events"][position]["resolution"] = value["events"][3]["resolution"].clone();
            assert!(dispatch(value, BTreeSet::from([(1, 1), (1, 2)])).is_err());
        }
        for route in ["feed", "interrupted", "green_ensure", "cache_hit"] {
            let mut value = dispatch_fixture();
            match route {
                "feed" => {
                    value["events"][0]["mode"] = json!("feed");
                    value["events"][3]["mode"] = json!("feed");
                    value["events"][3]["outcome"] = json!("fed");
                }
                "interrupted" => {
                    value["events"][3]["kind"] = json!("request_interrupted");
                    value["events"][3]["outcome"] = json!("interrupted");
                }
                "green_ensure" => {
                    value["events"][0]["mode"] = json!("ensure_ok");
                    value["events"][3]["mode"] = json!("ensure_ok");
                    value["events"][3]["outcome"] = json!("green_ensure");
                }
                "cache_hit" => {
                    let mut hit = value["events"][0].clone();
                    hit["kind"] = json!("cache_hit");
                    hit["dep_node_index"] = json!(42);
                    value["events"].as_array_mut().unwrap().insert(3, hit);
                }
                _ => unreachable!(),
            }
            value["records_complete"] = json!(false);
            value["lost_events"] = json!(1);
            assert!(
                dispatch(value, BTreeSet::from([(1, 1), (1, 2)])).is_err(),
                "{route}"
            );
        }
    }
    #[test]
    fn dispatch_result_unresolved_reasons_partition_observed_requests() {
        for reason in [
            "feed_request",
            "interrupted_request",
            "missing_request_terminal",
            "cache_hit_request",
            "no_dispatch_resolution",
            "null_dep_node_index",
            "missing_context_or_consumer",
            "capture_incomplete",
            "no_prior_producer",
            "producer_published_too_late",
            "job_endpoint_unavailable",
        ] {
            let mut value = dispatch_fixture();
            let mut admitted = BTreeSet::from([(1, 1), (1, 2)]);
            match reason {
                "feed_request" => {
                    value["events"][0]["mode"] = json!("feed");
                    value["events"][3]["mode"] = json!("feed");
                    value["events"][3]["outcome"] = json!("fed");
                    value["events"][3]["resolution"] = Value::Null;
                }
                "interrupted_request" => {
                    value["events"][3]["kind"] = json!("request_interrupted");
                    value["events"][3]["outcome"] = json!("interrupted");
                    value["events"][3]["resolution"] = Value::Null;
                }
                "missing_request_terminal" => {
                    value["events"].as_array_mut().unwrap().pop();
                    value["records_complete"] = json!(false);
                }
                "cache_hit_request" => {
                    value["events"][3]["resolution"] = Value::Null;
                    let mut hit = value["events"][0].clone();
                    hit["kind"] = json!("cache_hit");
                    hit["dep_node_index"] = json!(42);
                    value["events"].as_array_mut().unwrap().insert(3, hit);
                }
                "no_dispatch_resolution" => value["events"][3]["resolution"] = Value::Null,
                "null_dep_node_index" => {
                    value["events"][3]["resolution"]["dep_node_index"] = Value::Null
                }
                "missing_context_or_consumer" => {
                    for i in [0, 2, 3] {
                        value["events"][i]["consumer_job_id"] = Value::Null;
                    }
                }
                "capture_incomplete" => {
                    value["records_complete"] = json!(false);
                    value["lost_events"] = json!(1);
                }
                "no_prior_producer" => {
                    value["events"][3]["resolution"]["dep_node_index"] = json!(43)
                }
                "producer_published_too_late" => {
                    let publication = value["events"].as_array_mut().unwrap().remove(1);
                    value["events"].as_array_mut().unwrap().push(publication);
                }
                "job_endpoint_unavailable" => {
                    admitted.remove(&(1, 1));
                }
                _ => unreachable!(),
            }
            let (_, report) = dispatch(value, admitted).unwrap();
            dispatch_partition(&report);
            assert_eq!(report.resolved_requests, 0, "{reason}");
            assert_eq!(
                serde_json::to_value(report.unresolved_reasons).unwrap()[reason],
                json!(1),
                "{reason}"
            );
        }
    }
    #[test]
    fn dispatch_results_interleaved_terminals_bind_their_own_exact_index() {
        for mode in ["get", "ensure_ok", "ensure_result"] {
            let mut value = dispatch_fixture();
            let mut start1 = value["events"][0].clone();
            start1["mode"] = json!(mode);
            let mut start2 = start1.clone();
            start2["request_id"] = json!(2);
            let pub1 = value["events"][1].clone();
            let mut pub2 = pub1.clone();
            pub2["result_id"] = json!(2);
            pub2["dep_node_index"] = json!(43);
            pub2["producer_job_id"] = json!(2);
            let mut end1 = value["events"][3].clone();
            end1["mode"] = json!(mode);
            end1["outcome"] = json!(if mode == "get" { "value" } else { "ensured" });
            let mut end2 = end1.clone();
            end2["request_id"] = json!(2);
            end2["resolution"]["dep_node_index"] = json!(43);
            value["events"] = json!([start1, start2, pub1, pub2, end2, end1]);
            value["expected_requests"] = json!(2);
            value["request_high_water"] = json!(3);
            value["expected_reads"] = json!(0);
            value["read_high_water"] = json!(1);
            value["expected_results"] = json!(2);
            value["result_high_water"] = json!(3);
            let (_, report) = dispatch(value, BTreeSet::from([(1, 1), (1, 2)])).unwrap();
            dispatch_partition(&report);
            assert_eq!(report.resolved_requests, 2);
            assert_eq!(report.distinct_publishing_job_relations, 2);
            let a = &report.relations[0];
            let b = &report.relations[1];
            assert_eq!(
                (
                    a.request_id,
                    a.dep_node_index,
                    a.publication_event_index,
                    a.request_terminal_event_index
                ),
                (1, 42, 2, 5)
            );
            assert_eq!(
                (
                    b.request_id,
                    b.dep_node_index,
                    b.publication_event_index,
                    b.request_terminal_event_index
                ),
                (2, 43, 3, 4)
            );
            assert_eq!(b.producer_job_id, b.consumer_job_id); // Event relation, not a whole-job loop.
        }
    }
    #[test]
    fn dispatch_result_duplicate_authority_foreign_context_and_bounds_fail_closed() {
        let mut value = dispatch_fixture();
        let mut duplicate = value["events"][1].clone();
        duplicate["result_id"] = json!(2);
        value["events"].as_array_mut().unwrap().insert(2, duplicate);
        value["expected_results"] = json!(2);
        value["result_high_water"] = json!(3);
        assert!(dispatch(value, BTreeSet::from([(1, 1), (1, 2)])).is_err());
        for position in [0, 1, 2, 3] {
            let mut value = dispatch_fixture();
            value["events"][position]["context_id"] = json!(2);
            assert!(dispatch(value, BTreeSet::from([(1, 1), (1, 2)])).is_err());
        }
        for budget in [0, 3, 1_000_001] {
            let source: ReadCaptureV2 = serde_json::from_value(dispatch_fixture()).unwrap();
            assert!(
                source
                    .into_base_and_assess(
                        &BTreeSet::from([(1, 1), (1, 2)]),
                        1,
                        &BTreeMap::from([(1, Some(3))]),
                        budget
                    )
                    .is_err()
            );
        }
        for schema in [1, 3] {
            let mut value = dispatch_fixture();
            value["schema"] = json!(schema);
            assert!(dispatch(value, BTreeSet::from([(1, 1), (1, 2)])).is_err());
        }
    }
    #[test]
    fn dispatch_projection_keeps_cache_calls_separate_and_unchanged() {
        let admitted = BTreeSet::from([(1, 1), (1, 2)]);
        let old = calls(call_fixture(), admitted.clone()).unwrap();
        let mut value = call_fixture();
        value["schema"] = json!(2);
        for event in value["events"].as_array_mut().unwrap() {
            event["resolution"] = Value::Null;
        }
        let (base, report) = dispatch(value, admitted.clone()).unwrap();
        assert_eq!(report.observed_dispatch_resolutions, 0);
        assert_eq!(report.resolved_requests, 0);
        assert_eq!(report.unresolved_reasons.cache_hit_request, 1);
        dispatch_partition(&report);
        let projected = assess_cache_calls_with_inventory_limit(
            &base,
            &admitted,
            1,
            &BTreeMap::from([(1, Some(3))]),
            MAX_EVENTS,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(old).unwrap(),
            serde_json::to_value(projected).unwrap()
        );
    }
    #[test]
    fn dispatch_results_do_not_borrow_other_context_or_partial_authority() {
        let mut value = dispatch_fixture();
        value["events"][1]["context_id"] = json!(2);
        let source: ReadCaptureV2 = serde_json::from_value(value).unwrap();
        let (_, report) = source
            .into_base_and_assess(
                &BTreeSet::from([(1, 1), (1, 2), (2, 1)]),
                2,
                &BTreeMap::from([(1, Some(3)), (2, Some(2))]),
                MAX_EVENTS,
            )
            .unwrap();
        dispatch_partition(&report);
        assert_eq!(report.resolved_requests, 0);
        assert_eq!(report.unresolved_reasons.no_prior_producer, 1);

        let mut value = dispatch_fixture();
        value["records_complete"] = json!(false);
        let (_, report) = dispatch(value, BTreeSet::from([(1, 1), (1, 2)])).unwrap();
        dispatch_partition(&report);
        assert_eq!(report.unresolved_reasons.capture_incomplete, 1);

        let mut value = dispatch_fixture();
        value["events"].as_array_mut().unwrap().remove(0);
        value["records_complete"] = json!(false);
        value["lost_events"] = json!(1);
        let (_, report) = dispatch(value, BTreeSet::from([(1, 1), (1, 2)])).unwrap();
        dispatch_partition(&report);
        assert_eq!(report.observed_dispatch_resolutions, 1);
        assert_eq!(report.observed_requests, 0);
        assert!(report.relations.is_empty());
        assert!(!report.records_complete);
    }
    fn call_fixture() -> Value {
        let mut value = fixture();
        value["events"][2]["kind"] = json!("cache_hit");
        value["events"][2]["read_id"] = Value::Null;
        value["events"][2]["mode"] = json!("get");
        value["events"][2]["outcome"] = Value::Null;
        value["expected_reads"] = json!(0);
        value["read_high_water"] = json!(1);
        value
    }
    fn calls(value: Value, admitted: BTreeSet<(u64, u64)>) -> Result<CacheCallAssessment> {
        assess_cache_calls_with_inventory_limit(
            &serde_json::from_value(value)?,
            &admitted,
            1,
            &BTreeMap::from([(1, Some(3))]),
            MAX_EVENTS,
        )
    }
    fn call_partition(report: &CacheCallAssessment) {
        let reasons = serde_json::to_value(&report.unresolved_reasons).unwrap();
        let total: u64 = reasons
            .as_object()
            .unwrap()
            .values()
            .map(|v| v.as_u64().unwrap())
            .sum();
        assert_eq!(total, report.unresolved_requests);
        assert_eq!(total + report.resolved_requests, report.observed_requests);
        assert_eq!(report.resolved_requests, report.relations.len() as u64);
    }
    #[test]
    fn cache_calls_publish_during_request_and_allow_forward_self_relation() {
        for producer in [1, 2] {
            let mut value = call_fixture();
            value["events"][1]["producer_job_id"] = json!(producer);
            let mut report = calls(value, BTreeSet::from([(1, 1), (1, 2)])).unwrap();
            call_partition(&report);
            assert_eq!(report.resolved_requests, 1);
            let row = &report.relations[0];
            assert_eq!(
                (
                    row.request_start_event_index,
                    row.publication_event_index,
                    row.cache_hit_event_index,
                    row.request_terminal_event_index
                ),
                (0, 1, 2, 3)
            );
            assert_eq!(
                (
                    row.request_id,
                    row.result_id,
                    row.context_id,
                    row.producer_job_id,
                    row.consumer_job_id,
                    row.dep_node_index
                ),
                (1, 1, 1, producer, 2, 42)
            );
            report.suppress_incomplete_admission_authority();
            report.suppress_incomplete_admission_authority();
            call_partition(&report);
            assert_eq!(report.unresolved_reasons.admission_authority_incomplete, 1);
            assert_eq!(report.distinct_publishing_job_relations, 0);
        }
    }
    #[test]
    fn cache_call_unresolved_reasons_partition_adversarial_routes() {
        for reason in [
            "feed_request",
            "missing_request_terminal",
            "interrupted_request",
            "ambiguous_cache_hits",
            "no_cache_hit",
            "incompatible_cache_hit_route",
            "missing_context_or_consumer",
            "capture_incomplete",
            "no_prior_producer",
            "producer_published_too_late",
            "job_endpoint_unavailable",
        ] {
            let mut value = call_fixture();
            let mut admitted = BTreeSet::from([(1, 1), (1, 2)]);
            match reason {
                "feed_request" => {
                    for i in [0, 2, 3] {
                        value["events"][i]["mode"] = json!("feed");
                    }
                    value["events"][3]["outcome"] = json!("fed");
                }
                "missing_request_terminal" => {
                    value["events"].as_array_mut().unwrap().pop();
                    value["records_complete"] = json!(false);
                }
                "interrupted_request" => {
                    value["events"][3]["kind"] = json!("request_interrupted");
                    value["events"][3]["outcome"] = json!("interrupted");
                }
                "ambiguous_cache_hits" => {
                    let hit = value["events"][2].clone();
                    value["events"].as_array_mut().unwrap().insert(3, hit);
                }
                "no_cache_hit" => {
                    value["events"].as_array_mut().unwrap().remove(2);
                }
                "incompatible_cache_hit_route" => {
                    for i in [0, 2, 3] {
                        value["events"][i]["mode"] = json!("ensure_ok");
                    }
                    value["events"][3]["outcome"] = json!("green_ensure");
                }
                "missing_context_or_consumer" => {
                    for i in [0, 2, 3] {
                        value["events"][i]["consumer_job_id"] = Value::Null;
                    }
                }
                "capture_incomplete" => {
                    value["records_complete"] = json!(false);
                    value["lost_events"] = json!(1);
                }
                "no_prior_producer" => {
                    value["events"].as_array_mut().unwrap().remove(1);
                    value["expected_results"] = json!(0);
                    value["result_high_water"] = json!(1);
                }
                "producer_published_too_late" => {
                    value["events"].as_array_mut().unwrap().swap(1, 2);
                }
                "job_endpoint_unavailable" => {
                    admitted.remove(&(1, 1));
                }
                _ => unreachable!(),
            }
            let report = calls(value, admitted).unwrap();
            call_partition(&report);
            assert_eq!(report.resolved_requests, 0, "{reason}");
            assert_eq!(
                serde_json::to_value(report.unresolved_reasons).unwrap()[reason],
                json!(1),
                "{reason}"
            );
        }
    }
    #[test]
    fn cache_calls_preserve_strict_duplicate_producer_and_endpoint_checks() {
        let mut value = call_fixture();
        let mut duplicate = value["events"][1].clone();
        duplicate["result_id"] = json!(2);
        value["events"].as_array_mut().unwrap().insert(2, duplicate);
        value["expected_results"] = json!(2);
        value["result_high_water"] = json!(3);
        assert!(calls(value, BTreeSet::from([(1, 1), (1, 2)])).is_err());
        let mut value = call_fixture();
        value["events"][2]["consumer_job_id"] = json!(1);
        assert!(calls(value, BTreeSet::from([(1, 1), (1, 2)])).is_err());
        let mut value = call_fixture();
        value["events"][1]["producer_job_id"] = json!(3);
        assert!(calls(value, BTreeSet::from([(1, 1), (1, 2)])).is_err());
    }
    #[test]
    fn cache_calls_multiple_requests_preserve_rows_but_count_distinct_job_relations() {
        for mode in ["get", "ensure_ok", "ensure_result"] {
            let mut value = call_fixture();
            for i in [0, 2, 3] {
                value["events"][i]["mode"] = json!(mode);
            }
            value["events"][3]["outcome"] = json!(if mode == "get" { "value" } else { "ensured" });
            let additional: Vec<_> = [0, 2, 3]
                .into_iter()
                .map(|i| {
                    let mut event = value["events"][i].clone();
                    event["request_id"] = json!(2);
                    event
                })
                .collect();
            value["events"].as_array_mut().unwrap().extend(additional);
            value["expected_requests"] = json!(2);
            value["request_high_water"] = json!(3);
            let report = calls(value, BTreeSet::from([(1, 1), (1, 2)])).unwrap();
            call_partition(&report);
            assert_eq!(report.resolved_requests, 2);
            assert_eq!(report.distinct_publishing_job_relations, 1);
            assert_eq!(report.relations[1].request_id, 2);
            assert_eq!(report.relations[1].publication_event_index, 1);
            assert_eq!(report.relations[1].cache_hit_event_index, 5);
        }
    }
    #[test]
    fn cache_calls_orphan_hits_and_policy_bounds_fail_closed() {
        let mut value = call_fixture();
        value["events"].as_array_mut().unwrap().remove(0);
        value["records_complete"] = json!(false);
        value["lost_events"] = json!(1);
        let report = calls(value, BTreeSet::from([(1, 1), (1, 2)])).unwrap();
        call_partition(&report);
        assert_eq!(report.orphan_cache_hits, 1);
        assert_eq!(report.observed_requests, 0);
        assert!(report.relations.is_empty());
        let capture = serde_json::from_value(call_fixture()).unwrap();
        for budget in [0, 3, 1_000_001] {
            assert!(
                assess_cache_calls_with_inventory_limit(
                    &capture,
                    &BTreeSet::from([(1, 1), (1, 2)]),
                    1,
                    &BTreeMap::from([(1, Some(3))]),
                    budget
                )
                .is_err()
            );
        }
        assert!(
            assess_cache_calls_with_inventory_limit(
                &capture,
                &BTreeSet::from([(1, 1), (1, 2)]),
                1,
                &BTreeMap::from([(1, Some(3))]),
                4
            )
            .is_ok()
        );
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
