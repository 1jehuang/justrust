//! Bounded, opt-in direct query reads. Publication must occur outside query/dep locks.
//! Request IDs are explicit, never inferred from OS TLS or admission nesting.
use crate::query_task_bindings::{Source, TaskBindings};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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
    bindings: Option<Arc<TaskBindings>>,
    requests: AtomicU64,
    reads: AtomicU64,
    results: AtomicU64,
    lost: AtomicU64,
    lost_overflowed: AtomicBool,
    // Monotonic: set only once event_capacity events have been accepted under the mutex.
    event_capacity: usize,
    capped: AtomicBool,
    events: Mutex<Vec<Event>>,
}
impl Default for QueryReadObserver {
    fn default() -> Self {
        Self {
            bindings: None,
            requests: AtomicU64::new(1),
            reads: AtomicU64::new(1),
            results: AtomicU64::new(1),
            lost: AtomicU64::new(0),
            lost_overflowed: AtomicBool::new(false),
            event_capacity: MAX_EVENTS,
            capped: AtomicBool::new(false),
            events: Mutex::new(Vec::new()),
        }
    }
}
impl QueryReadObserver {
    /// Opt-in diagnostic capacity only. No allocation occurs until events arrive.
    pub fn with_event_capacity(capacity: usize) -> Option<Self> {
        if !(1..=crate::query_task_bindings::MAX_EVENT_CAPACITY).contains(&capacity) {
            return None;
        }
        Some(Self { event_capacity: capacity, ..Self::default() })
    }
    pub fn with_bindings_and_event_capacity(
        bindings: Arc<TaskBindings>,
        capacity: usize,
    ) -> Option<Self> {
        Some(Self { bindings: Some(bindings), ..Self::with_event_capacity(capacity)? })
    }
    pub fn with_bindings(bindings: Arc<TaskBindings>) -> Self {
        Self { bindings: Some(bindings), ..Self::default() }
    }
    #[cfg(test)]
    pub(crate) fn event_capacity(&self) -> usize {
        self.event_capacity
    }
    pub fn high_waters(&self) -> [u64; 3] {
        [
            self.requests.load(Ordering::Relaxed),
            self.reads.load(Ordering::Relaxed),
            self.results.load(Ordering::Relaxed),
        ]
    }
    fn loss(&self, amount: u64) {
        // Keep the common path to one fetch_add, not a contended CAS retry loop.
        // Counts are exact while representable; wrapping must never restore completeness.
        if self.lost.fetch_add(amount, Ordering::Relaxed) > u64::MAX - amount {
            self.lost_overflowed.store(true, Ordering::Relaxed);
        }
    }
    fn event(&self, event: Event) {
        // IDs are allocated by the callers even when recording is capped. Relaxed is
        // enough: this bit publishes no vector data, and a stale false still takes
        // the mutex and checks its length. Accepted events retain their mutex order.
        if self.capped.load(Ordering::Relaxed) {
            // Preserve the existing loss accounting: one per rejected event, plus
            // one per poisoned-lock observation. is_poisoned does not take the lock.
            // Publication adds its own poison marker, so poison always fails incomplete.
            // A concurrent panic can race this observation; unlike locking, it cannot
            // wait for the holder's poison flag. Each rejection is still counted once.
            self.loss(1 + u64::from(self.events.is_poisoned()));
            return;
        }
        let mut events = match self.events.lock() {
            Ok(events) => events,
            Err(error) => {
                self.loss(1);
                error.into_inner()
            }
        };
        if events.len() < self.event_capacity && events.try_reserve(1).is_ok() {
            events.push(event);
            if events.len() == self.event_capacity {
                self.capped.store(true, Ordering::Relaxed);
            }
        } else {
            // A reservation failure is not a permanent cap: later calls still retry.
            self.loss(1);
        }
    }
    pub fn begin(
        &self,
        context: Option<u64>,
        consumer: Option<u64>,
        mode: RequestMode,
    ) -> QueryReadRequest<'_> {
        let id = self.requests.fetch_add(1, Ordering::Relaxed);
        if let Some(bindings) = &self.bindings {
            bindings.allocator_id(id);
            bindings.record(Source::RequestStarted { request_id: id });
        }
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
        if let Some(bindings) = &self.bindings {
            bindings.allocator_id(id);
            bindings.record(Source::DepRead { read_id: id });
        }
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
        if let Some(bindings) = &self.bindings {
            bindings.allocator_id(id);
            bindings.record(Source::ResultProduced { result_id: id });
        }
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
        let complete = lost == 0
            && !self.lost_overflowed.load(Ordering::Relaxed)
            && inventory_matches(&events, high_requests, high_reads, high_results);
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
    fn explicit_event_capacity_is_bounded_and_defaults_are_unchanged() {
        assert!(QueryReadObserver::with_event_capacity(0).is_none());
        assert!(QueryReadObserver::with_event_capacity(1_000_001).is_none());
        assert_eq!(QueryReadObserver::default().event_capacity, 100_000);
        for capacity in [1, 3, 100_001, 1_000_000] {
            let observer = QueryReadObserver::with_event_capacity(capacity).unwrap();
            assert_eq!(observer.event_capacity, capacity);
            assert!(observer.events.lock().unwrap().is_empty());
        }
        let observer = QueryReadObserver::with_event_capacity(2).unwrap();
        observer.result_produced(1, 1, 1);
        observer.result_produced(1, 1, 2);
        assert!(json(&observer).contains("\"records_complete\":true"));
        observer.result_produced(1, 1, 3);
        assert_eq!(observer.events.lock().unwrap().len(), 2);
        assert_eq!(observer.high_waters(), [1, 1, 4]);
        let out = json(&observer);
        assert!(out.contains("\"lost_events\":1"));
        assert!(out.contains("\"expected_results\":3"));
        assert!(out.contains("\"records_complete\":false"));
        for capacity in [MAX_EVENTS, MAX_EVENTS + 1] {
            let observer = QueryReadObserver::with_event_capacity(capacity).unwrap();
            for index in 0..capacity {
                observer.result_produced(1, 1, index as u32);
            }
            assert_eq!(observer.events.lock().unwrap().len(), capacity);
            assert!(json(&observer).contains("\"records_complete\":true"));
            observer.result_produced(1, 1, capacity as u32);
            assert_eq!(observer.events.lock().unwrap().len(), capacity);
            assert_eq!(observer.high_waters(), [1, 1, capacity as u64 + 2]);
            let out = json(&observer);
            assert!(out.contains("\"lost_events\":1"));
            assert!(out.contains("\"records_complete\":false"));
        }
    }
    #[test]
    fn unequal_capacities_keep_read_and_binding_loss_independent() {
        for (read_capacity, binding_capacity) in [(1, 3), (3, 1)] {
            let bindings = Arc::new(
                TaskBindings::with_event_capacity(
                    Arc::new(crate::query_task_bindings::UnavailableSampler),
                    binding_capacity,
                )
                .unwrap(),
            );
            assert!(
                QueryReadObserver::with_bindings_and_event_capacity(bindings.clone(), 0).is_none()
            );
            let observer = QueryReadObserver::with_bindings_and_event_capacity(
                bindings.clone(),
                read_capacity,
            )
            .unwrap();
            for index in 0..3 {
                observer.read(None, None, None, index, DepReadOutcome::Disabled);
            }
            assert_eq!(observer.high_waters(), [1, 4, 1]);
            assert_eq!(observer.events.lock().unwrap().len(), read_capacity);
            let out = json(&observer);
            assert!(out.contains(&format!("\"lost_events\":{}", 3 - read_capacity)));
            assert!(out.contains("\"expected_reads\":3"));
            let mut binding_out = String::new();
            bindings.append_json(&mut binding_out, &[], Some(0), [1, 4, 1, 1]);
            assert!(binding_out.contains(&format!("\"lost_events\":{}", 3 - binding_capacity)));
            assert!(binding_out.contains("\"expected_points\":3"));
            assert_eq!(binding_out.matches("\"read_id\":").count(), binding_capacity);
        }
    }
    #[test]
    fn task_binding_inventory_is_independent_of_read_record_loss() {
        let bindings =
            Arc::new(TaskBindings::new(Arc::new(crate::query_task_bindings::UnavailableSampler)));
        let observer = QueryReadObserver::with_bindings(bindings.clone());
        observer.begin(None, None, RequestMode::Get).finish(RequestOutcome::Value);
        observer.read(None, None, None, 3, DepReadOutcome::Disabled);
        observer.result_produced(1, 2, 3);
        observer.events.lock().unwrap().clear();
        observer.loss(17);
        assert!(json(&observer).contains("\"records_complete\":false"));
        let [requests, reads, results] = observer.high_waters();
        let mut out = String::new();
        bindings.append_json(&mut out, &[], Some(0), [requests, reads, results, 1]);
        assert!(out.contains("\"records_complete\":true"));
        assert!(out.contains("\"expected_points\":3"));
        assert!(out.contains("\"lost_events\":0"));
        for kind in ["request_started", "dep_read", "result_produced"] {
            assert_eq!(out.matches(&format!("\"kind\":\"{kind}\"")).count(), 1);
        }
        assert!(QueryReadObserver::default().bindings.is_none());
    }
    #[test]
    fn bound_allocator_wrap_cannot_restore_empty_inventory() {
        let bindings =
            Arc::new(TaskBindings::new(Arc::new(crate::query_task_bindings::UnavailableSampler)));
        let observer = QueryReadObserver::with_bindings(bindings.clone());
        observer.reads.store(u64::MAX, Ordering::Relaxed);
        observer.read(None, None, None, 3, DepReadOutcome::Disabled);
        observer.read(None, None, None, 3, DepReadOutcome::Disabled);
        let mut out = String::new();
        bindings.append_json(&mut out, &[], Some(0), [1; 4]);
        assert!(out.contains("\"expected_points\":null"));
        assert!(out.contains("\"records_complete\":false"));
    }
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
    #[test]
    fn exact_cap_is_complete_and_preserves_accepted_order() {
        let observer = QueryReadObserver::default();
        for index in 0..MAX_EVENTS - 1 {
            observer.result_produced(1, 2, index as u32);
        }
        assert!(!observer.capped.load(Ordering::Relaxed));
        observer.result_produced(1, 2, (MAX_EVENTS - 1) as u32);
        assert!(observer.capped.load(Ordering::Relaxed));
        assert_eq!(observer.lost.load(Ordering::Relaxed), 0);
        let events = observer.events.lock().unwrap();
        assert_eq!(events.len(), MAX_EVENTS);
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event.kind, "result_produced");
            assert_eq!(event.result, Some(index as u64 + 1));
            assert_eq!(event.index, Some(index as u32));
        }
        drop(events);
        assert!(json(&observer).contains("\"records_complete\":true"));
    }
    #[test]
    fn capped_events_do_not_lock_and_still_allocate_independent_ids() {
        let observer = QueryReadObserver::default();
        for _ in 0..MAX_EVENTS {
            observer.result_produced(1, 1, 42);
        }
        let events = observer.events.lock().unwrap();
        let capacity = events.capacity();
        std::thread::scope(|scope| {
            let (sender, receiver) = std::sync::mpsc::channel();
            let observer = &observer;
            let handle = scope.spawn(move || {
                let request = observer.begin(Some(1), Some(2), RequestMode::Get);
                assert_eq!(request.id(), 1);
                request.cache_hit(42);
                observer.read(Some(1), Some(2), Some(request.id()), 42, DepReadOutcome::TrackedNew);
                observer.result_produced(1, 2, 42);
                request.finish(RequestOutcome::Value);
                let interrupted = observer.begin(None, None, RequestMode::EnsureOk);
                assert_eq!(interrupted.id(), 2);
                drop(interrupted);
                sender.send(()).unwrap();
            });
            let bypassed = receiver.recv_timeout(std::time::Duration::from_secs(5));
            // Release even on failure so a regression cannot deadlock the test suite.
            drop(events);
            handle.join().unwrap();
            assert!(bypassed.is_ok(), "capped calls blocked on the event mutex");
        });
        let events = observer.events.lock().unwrap();
        assert_eq!(events.len(), MAX_EVENTS);
        assert_eq!(events.capacity(), capacity);
        drop(events);
        assert_eq!(observer.requests.load(Ordering::Relaxed), 3);
        assert_eq!(observer.reads.load(Ordering::Relaxed), 2);
        assert_eq!(observer.results.load(Ordering::Relaxed), MAX_EVENTS as u64 + 2);
        assert_eq!(observer.lost.load(Ordering::Relaxed), 7);
        let out = json(&observer);
        assert!(out.contains("\"records_complete\":false"));
        assert!(out.contains("\"lost_events\":7"));
    }
    #[test]
    fn racing_cap_retains_prefix_and_counts_each_rejected_event_once() {
        let observer = QueryReadObserver::default();
        let prefix = MAX_EVENTS - 17;
        for index in 0..prefix {
            observer.result_produced(1, 1, index as u32);
        }
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    barrier.wait();
                    for iteration in 0..2000 {
                        let request = observer.begin(Some(1), Some(2), RequestMode::Get);
                        request.cache_hit(42);
                        observer.read(
                            Some(1),
                            Some(2),
                            Some(request.id()),
                            42,
                            DepReadOutcome::TrackedDuplicate,
                        );
                        observer.result_produced(1, 2, 42);
                        if iteration % 2 == 0 {
                            request.finish(RequestOutcome::Value);
                        } else {
                            drop(request);
                        }
                    }
                });
            }
        });
        assert!(observer.capped.load(Ordering::Relaxed));
        assert_eq!(observer.requests.load(Ordering::Relaxed), 16_001);
        assert_eq!(observer.reads.load(Ordering::Relaxed), 16_001);
        assert_eq!(observer.results.load(Ordering::Relaxed), prefix as u64 + 16_001);
        assert_eq!(observer.lost.load(Ordering::Relaxed), 80_000 - 17);
        let events = observer.events.lock().unwrap();
        assert_eq!(events.len(), MAX_EVENTS);
        for (index, event) in events[..prefix].iter().enumerate() {
            assert_eq!(event.kind, "result_produced");
            assert_eq!(event.result, Some(index as u64 + 1));
            assert_eq!(event.index, Some(index as u32));
        }
        drop(events);
        let out = json(&observer);
        assert!(out.contains("\"lost_events\":79983"));
        assert!(out.contains("\"records_complete\":false"));
    }
    #[test]
    fn poison_after_cap_preserves_rejection_and_poison_loss_counts() {
        let observer = QueryReadObserver::default();
        for _ in 0..MAX_EVENTS {
            observer.result_produced(1, 1, 42);
        }
        let _ = std::panic::catch_unwind(|| {
            let _lock = observer.events.lock().unwrap();
            panic!("poison capped observer only");
        });
        let request = observer.begin(None, None, RequestMode::Get);
        request.cache_hit(42);
        observer.read(None, None, None, 42, DepReadOutcome::Disabled);
        observer.result_produced(1, 1, 42);
        drop(request);
        // Five rejections plus five poison observations, then the publication marker.
        assert_eq!(observer.lost.load(Ordering::Relaxed), 10);
        assert_eq!(observer.requests.load(Ordering::Relaxed), 2);
        assert_eq!(observer.reads.load(Ordering::Relaxed), 2);
        assert_eq!(observer.results.load(Ordering::Relaxed), MAX_EVENTS as u64 + 2);
        let out = json(&observer);
        assert!(out.contains("\"lost_events\":11"));
        assert!(out.contains("\"records_complete\":false"));
        assert_eq!(out.matches("{\"kind\":").count(), MAX_EVENTS);
        assert_eq!(json(&observer), out);
    }
    #[test]
    fn identifier_wrap_is_independent_and_fails_inventory() {
        let observer = QueryReadObserver::default();
        observer.requests.store(u64::MAX, Ordering::Relaxed);
        observer.reads.store(u64::MAX, Ordering::Relaxed);
        observer.results.store(u64::MAX, Ordering::Relaxed);
        let request = observer.begin(None, None, RequestMode::Get);
        assert_eq!(request.id(), u64::MAX);
        request.finish(RequestOutcome::Value);
        observer.read(None, None, None, 42, DepReadOutcome::Disabled);
        observer.result_produced(1, 1, 42);
        assert_eq!(observer.requests.load(Ordering::Relaxed), 0);
        assert_eq!(observer.reads.load(Ordering::Relaxed), 0);
        assert_eq!(observer.results.load(Ordering::Relaxed), 0);
        assert!(json(&observer).contains("\"records_complete\":false"));
        let request = observer.begin(None, None, RequestMode::Get);
        assert_eq!(request.id(), 0);
        drop(request);
        observer.read(None, None, None, 42, DepReadOutcome::Disabled);
        observer.result_produced(1, 1, 42);
        assert_eq!(observer.requests.load(Ordering::Relaxed), 1);
        assert_eq!(observer.reads.load(Ordering::Relaxed), 1);
        assert_eq!(observer.results.load(Ordering::Relaxed), 1);
        assert_eq!(observer.lost.load(Ordering::Relaxed), 0);
        assert!(json(&observer).contains("\"records_complete\":false"));
    }
    #[test]
    fn loss_counter_wrap_cannot_restore_complete_retained_inventory() {
        let observer = QueryReadObserver::default();
        for _ in 0..MAX_EVENTS {
            observer.result_produced(1, 1, 42);
        }
        let event = {
            let events = observer.events.lock().unwrap();
            assert!(inventory_matches(&events, 1, 1, MAX_EVENTS as u64 + 1));
            events[0]
        };
        observer.lost.store(u64::MAX, Ordering::Relaxed);
        assert!(!observer.lost_overflowed.load(Ordering::Relaxed));
        // Reject an event without changing IDs, so the retained inventory still matches.
        observer.event(event);
        assert_eq!(observer.lost.load(Ordering::Relaxed), 0);
        assert!(observer.lost_overflowed.load(Ordering::Relaxed));
        let out = json(&observer);
        assert!(out.contains("\"lost_events\":0"));
        assert!(out.contains("\"records_complete\":false"));
        // The latch is permanent, including the two-count poisoned-rejection increment.
        observer.loss(u64::MAX);
        observer.loss(2);
        assert_eq!(observer.lost.load(Ordering::Relaxed), 1);
        assert!(observer.lost_overflowed.load(Ordering::Relaxed));
        assert!(json(&observer).contains("\"records_complete\":false"));
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
        assert!(!observer.capped.load(Ordering::Relaxed));
        assert!(observer.events.is_poisoned());
        assert_eq!(observer.events.lock().unwrap_or_else(|error| error.into_inner()).len(), 3);
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
