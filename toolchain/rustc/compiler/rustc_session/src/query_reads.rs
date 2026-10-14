//! Bounded, opt-in direct query reads. Publication must occur outside query/dep locks.
//! Request IDs are explicit, never inferred from OS TLS or admission nesting.
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_EVENTS: usize = 100_000;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RequestMode {
    Get,
    EnsureOk,
    EnsureResult,
    Feed,
}
impl RequestMode {
    fn name(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::EnsureOk => "ensure_ok",
            Self::EnsureResult => "ensure_result",
            Self::Feed => "feed",
        }
    }
}
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RequestOutcome {
    Value,
    Ensured,
    GreenEnsure,
    Fed,
    Interrupted,
}
impl RequestOutcome {
    fn name(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Ensured => "ensured",
            Self::GreenEnsure => "green_ensure",
            Self::Fed => "fed",
            Self::Interrupted => "interrupted",
        }
    }
}
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DepReadOutcome {
    TrackedNew,
    TrackedDuplicate,
    EvalAlways,
    Ignore,
    Disabled,
    MissingContext,
}
impl DepReadOutcome {
    fn name(self) -> &'static str {
        match self {
            Self::TrackedNew => "tracked_new",
            Self::TrackedDuplicate => "tracked_duplicate",
            Self::EvalAlways => "eval_always",
            Self::Ignore => "ignore",
            Self::Disabled => "disabled",
            Self::MissingContext => "missing_context",
        }
    }
}
#[derive(Copy, Clone)]
struct Event {
    kind: &'static str,
    request: Option<u64>,
    read: Option<u64>,
    result: Option<u64>,
    producer: Option<u64>,
    context: Option<u64>,
    consumer: Option<u64>,
    mode: Option<RequestMode>,
    outcome: Option<&'static str>,
    index: Option<u32>,
}

pub struct QueryReadObserver {
    requests: AtomicU64,
    reads: AtomicU64,
    results: AtomicU64,
    lost: AtomicU64,
    events: Mutex<Vec<Event>>,
}
impl Default for QueryReadObserver {
    fn default() -> Self {
        Self {
            requests: AtomicU64::new(1),
            reads: AtomicU64::new(1),
            results: AtomicU64::new(1),
            lost: AtomicU64::new(0),
            events: Mutex::new(Vec::new()),
        }
    }
}
impl QueryReadObserver {
    fn event(&self, event: Event) {
        let mut events = match self.events.lock() {
            Ok(events) => events,
            Err(error) => {
                self.lost.fetch_add(1, Ordering::Relaxed);
                error.into_inner()
            }
        };
        if events.len() < MAX_EVENTS && events.try_reserve(1).is_ok() {
            events.push(event);
        } else {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn begin(
        &self,
        context: Option<u64>,
        consumer: Option<u64>,
        mode: RequestMode,
    ) -> QueryReadRequest<'_> {
        let id = self.requests.fetch_add(1, Ordering::Relaxed);
        let event = Event {
            kind: "request_started",
            request: Some(id),
            read: None,
            result: None,
            producer: None,
            context,
            consumer,
            mode: Some(mode),
            outcome: None,
            index: None,
        };
        self.event(event);
        QueryReadRequest { observer: self, event, terminal: false }
    }
    pub fn read(
        &self,
        context: Option<u64>,
        consumer: Option<u64>,
        request: Option<u64>,
        index: u32,
        outcome: DepReadOutcome,
    ) {
        let id = self.reads.fetch_add(1, Ordering::Relaxed);
        self.event(Event {
            kind: "dep_read",
            request,
            read: Some(id),
            result: None,
            producer: None,
            context,
            consumer,
            mode: None,
            outcome: Some(outcome.name()),
            index: Some(index),
        });
    }
    pub fn result_produced(&self, context: u64, job: u64, index: u32) {
        let id = self.results.fetch_add(1, Ordering::Relaxed);
        self.event(Event {
            kind: "result_produced",
            request: None,
            read: None,
            result: Some(id),
            producer: Some(job),
            context: Some(context),
            consumer: None,
            mode: None,
            outcome: None,
            index: Some(index),
        });
    }
    /// Append a sibling object before the root receipt's closing brace. Called at root Drop,
    /// after all query context handles are gone, not while a compiler lock is held.
    pub fn append_json(&self, out: &mut String) {
        let high_requests = self.requests.load(Ordering::Relaxed);
        let high_reads = self.reads.load(Ordering::Relaxed);
        let high_results = self.results.load(Ordering::Relaxed);
        let events = self.events.lock();
        let poisoned = events.is_err();
        let events = events.unwrap_or_else(|error| error.into_inner());
        let lost = self.lost.load(Ordering::Relaxed).saturating_add(u64::from(poisoned));
        let complete =
            lost == 0 && inventory_matches(&events, high_requests, high_reads, high_results);
        let _ = write!(
            out,
            ",\"query_reads\":{{\"schema\":1,\"records_complete\":{complete},\"semantic_complete\":false,\"full_pipeline_complete\":false,\"dep_node_namespace\":\"current-context\",\"request_high_water\":{high_requests},\"read_high_water\":{high_reads},\"result_high_water\":{high_results},\"expected_results\":{},\"expected_requests\":{},\"expected_reads\":{},\"lost_events\":{lost},\"opaque_boundaries\":[\"execution-request-read-binding\",\"previous-session-promotion\",\"feed-producer-provenance\",\"missing-consumer-context\",\"no-implicit-context-reads\",\"nonquery\"],\"events\":[",
            high_results.saturating_sub(1),
            high_requests.saturating_sub(1),
            high_reads.saturating_sub(1)
        );
        for (i, event) in events.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"kind\":\"{}\",\"request_id\":{},\"read_id\":{},\"context_id\":{},\"consumer_job_id\":{},\"dep_node_index\":{}",
                event.kind,
                number(event.request),
                number(event.read),
                number(event.context),
                number(event.consumer),
                number(event.index.map(u64::from))
            );
            let _ = write!(
                out,
                ",\"result_id\":{},\"producer_job_id\":{}",
                number(event.result),
                number(event.producer)
            );
            let mode =
                event.mode.map_or_else(|| "null".into(), |mode| format!("\"{}\"", mode.name()));
            let outcome =
                event.outcome.map_or_else(|| "null".into(), |outcome| format!("\"{outcome}\""));
            let _ = write!(out, ",\"mode\":{mode},\"outcome\":{outcome}");
            out.push('}');
        }
        out.push_str("]}");
    }
}
fn number(value: Option<u64>) -> String {
    value.map_or_else(|| "null".into(), |v| v.to_string())
}

pub struct QueryReadRequest<'a> {
    observer: &'a QueryReadObserver,
    event: Event,
    terminal: bool,
}
impl QueryReadRequest<'_> {
    pub fn id(&self) -> u64 {
        self.event.request.unwrap()
    }
    pub fn cache_hit(&self, index: u32) {
        self.observer.event(Event { kind: "cache_hit", index: Some(index), ..self.event });
    }
    pub fn finish(mut self, outcome: RequestOutcome) {
        self.terminal = true;
        self.observer.event(Event {
            kind: "request_finished",
            outcome: Some(outcome.name()),
            ..self.event
        });
    }
}
impl Drop for QueryReadRequest<'_> {
    fn drop(&mut self) {
        if !self.terminal {
            self.observer.event(Event {
                kind: "request_interrupted",
                outcome: Some("interrupted"),
                ..self.event
            });
        }
    }
}

fn inventory_matches(events: &[Event], requests: u64, reads: u64, results: u64) -> bool {
    if requests == 0 || reads == 0 || results == 0 {
        return false;
    }
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    let mut read_ids = Vec::new();
    let mut result_ids = Vec::new();
    if starts.try_reserve(events.len()).is_err()
        || ends.try_reserve(events.len()).is_err()
        || read_ids.try_reserve(events.len()).is_err()
        || result_ids.try_reserve(events.len()).is_err()
    {
        return false;
    }
    for (position, event) in events.iter().enumerate() {
        match event.kind {
            "request_started" => starts.push((event.request.unwrap_or(0), position)),
            "request_finished" | "request_interrupted" => {
                ends.push((event.request.unwrap_or(0), position))
            }
            "dep_read" => read_ids.push(event.read.unwrap_or(0)),
            "result_produced" => result_ids.push(event.result.unwrap_or(0)),
            "cache_hit" => {}
            _ => return false,
        }
    }
    starts.sort_unstable();
    ends.sort_unstable();
    read_ids.sort_unstable();
    result_ids.sort_unstable();
    starts.len() as u64 == requests - 1
        && starts.len() == ends.len()
        && starts
            .iter()
            .zip(&ends)
            .enumerate()
            .all(|(i, (start, end))| start.0 == i as u64 + 1 && start.0 == end.0 && start.1 < end.1)
        && read_ids.len() as u64 == reads - 1
        && read_ids.iter().enumerate().all(|(i, &id)| id == i as u64 + 1)
        && result_ids.len() as u64 == results - 1
        && result_ids.iter().enumerate().all(|(i, &id)| id == i as u64 + 1)
        && events.iter().enumerate().all(|(position, event)| {
            event.request.is_none_or(|id| {
                starts.binary_search_by_key(&id, |start| start.0).ok().is_some_and(|i| {
                    let start = &events[starts[i].1];
                    start.context == event.context
                        && start.consumer == event.consumer
                        && position >= starts[i].1
                        && position <= ends[i].1
                })
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn produced_result_has_independent_inventory() {
        let observer = QueryReadObserver::default();
        observer.result_produced(2, 7, 42);
        observer.read(Some(2), Some(8), None, 42, DepReadOutcome::TrackedNew);
        let out = json(&observer);
        assert!(out.contains("\"records_complete\":true"));
        assert!(out.contains("\"producer_job_id\":7"));
        assert!(out.contains("\"result_high_water\":2"));
        assert!(out.contains("\"expected_results\":1"));
        observer.events.lock().unwrap().retain(|event| event.kind != "result_produced");
        assert!(json(&observer).contains("\"records_complete\":false"));
    }
    #[test]
    fn parallel_contention_does_not_drop_inventory() {
        let observer = QueryReadObserver::default();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..100 {
                        let request = observer.begin(Some(1), None, RequestMode::Get);
                        observer.read(
                            Some(1),
                            None,
                            Some(request.id()),
                            42,
                            DepReadOutcome::TrackedDuplicate,
                        );
                        request.finish(RequestOutcome::Value);
                    }
                });
            }
        });
        let out = json(&observer);
        assert!(out.contains("\"records_complete\":true"));
        assert!(out.contains("\"lost_events\":0"));
        assert!(out.contains("\"expected_requests\":800"));
    }
    #[test]
    fn capacity_overflow_preserves_independent_counts() {
        let observer = QueryReadObserver::default();
        for _ in 0..MAX_EVENTS + 1 {
            observer.result_produced(1, 1, 42);
        }
        assert_eq!(observer.events.lock().unwrap().len(), MAX_EVENTS);
        let out = json(&observer);
        assert!(out.contains("\"records_complete\":false"));
        assert!(out.contains("\"lost_events\":1"));
        assert!(out.contains("\"expected_results\":100001"));
    }
    fn json(observer: &QueryReadObserver) -> String {
        let mut out = String::new();
        observer.append_json(&mut out);
        out
    }
    #[test]
    fn fast_hit_and_native_read_have_separate_identity() {
        let observer = QueryReadObserver::default();
        let request = observer.begin(Some(2), Some(7), RequestMode::Get);
        request.cache_hit(4);
        observer.read(Some(2), Some(7), Some(request.id()), 4, DepReadOutcome::TrackedNew);
        request.finish(RequestOutcome::Value);
        let out = json(&observer);
        assert!(out.contains("\"records_complete\":true"));
        assert!(out.contains("\"semantic_complete\":false"));
        assert!(out.contains("\"expected_requests\":1"));
        assert!(out.contains("\"expected_reads\":1"));
    }
    #[test]
    fn interruption_is_terminal_not_success() {
        let observer = QueryReadObserver::default();
        drop(observer.begin(None, None, RequestMode::EnsureResult));
        assert!(json(&observer).contains("request_interrupted"));
        assert!(json(&observer).contains("\"records_complete\":true"));
    }
    #[test]
    fn untracked_and_unbound_reads_never_claim_semantics() {
        let observer = QueryReadObserver::default();
        for outcome in [
            DepReadOutcome::TrackedDuplicate,
            DepReadOutcome::EvalAlways,
            DepReadOutcome::Ignore,
            DepReadOutcome::Disabled,
            DepReadOutcome::MissingContext,
        ] {
            observer.read(None, None, None, 9, outcome);
        }
        let out = json(&observer);
        assert!(out.contains("\"request_id\":null"));
        assert!(out.contains("\"semantic_complete\":false"));
        assert!(out.contains("\"expected_reads\":5"));
    }
    #[test]
    fn poison_is_failure_open_and_independently_counted() {
        let observer = QueryReadObserver::default();
        let _ = std::panic::catch_unwind(|| {
            let _lock = observer.events.lock().unwrap();
            panic!("poison observer only");
        });
        drop(observer.begin(None, None, RequestMode::Get));
        observer.read(None, None, None, 0, DepReadOutcome::Disabled);
        let out = json(&observer);
        assert!(out.contains("\"lost_events\":4"));
        assert!(out.contains("\"expected_requests\":1"));
        assert!(out.contains("\"expected_reads\":1"));
        assert!(out.contains("\"records_complete\":false"));
    }
    #[test]
    fn removing_entire_entity_fails_inventory() {
        let observer = QueryReadObserver::default();
        observer.begin(Some(1), None, RequestMode::EnsureOk).finish(RequestOutcome::GreenEnsure);
        observer.events.lock().unwrap().clear();
        assert!(json(&observer).contains("\"records_complete\":false"));
    }
    #[test]
    fn foreign_endpoint_and_duplicate_read_fail_inventory() {
        let observer = QueryReadObserver::default();
        let request = observer.begin(Some(1), Some(1), RequestMode::Get);
        observer.read(Some(2), Some(1), Some(request.id()), 1, DepReadOutcome::TrackedNew);
        request.finish(RequestOutcome::Value);
        assert!(json(&observer).contains("\"records_complete\":false"));
        let observer = QueryReadObserver::default();
        observer.read(Some(1), None, None, 1, DepReadOutcome::TrackedDuplicate);
        {
            let mut events = observer.events.lock().unwrap();
            let event = events[0];
            events.push(event);
        }
        assert!(json(&observer).contains("\"records_complete\":false"));
    }
}
