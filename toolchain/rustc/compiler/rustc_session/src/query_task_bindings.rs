//! Independent, bounded query-to-pool point receipts. Sampling never takes the record lock.
#[cfg(not(test))]
use rustc_data_structures::marker::IntoDynSyncSend;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
const MAX_EVENTS: usize = 100_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Admitted { context_id: u64, job_id: u64 },
    RequestStarted { request_id: u64 },
    DepRead { read_id: u64 },
    ResultProduced { result_id: u64 },
    WaitRegistered { wait_id: u64 },
    WaitResumed { wait_id: u64 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub task: u64,
    pub at_pool_ns: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unbound {
    PoolUnavailable,
    NoActive,
    Suspended,
    ForeignObserver,
    ClockOverflow,
}
impl Unbound {
    fn name(self) -> &'static str {
        match self {
            Self::PoolUnavailable => "pool_unavailable",
            Self::NoActive => "no_active",
            Self::Suspended => "suspended",
            Self::ForeignObserver => "foreign_observer",
            Self::ClockOverflow => "clock_overflow",
        }
    }
}
pub type Sample = Result<Point, Unbound>;
pub trait Sampler: Send + Sync + std::panic::RefUnwindSafe {
    fn sample(&self) -> Sample;
    fn authority(&self) -> Option<(String, u64)>;
}
#[derive(Default)]
pub struct UnavailableSampler;
impl Sampler for UnavailableSampler {
    fn sample(&self) -> Sample {
        Err(Unbound::PoolUnavailable)
    }
    fn authority(&self) -> Option<(String, u64)> {
        None
    }
}
pub struct TaskBindings {
    // The compiler's dynamic threading markers intentionally do not infer
    // std Send/Sync for trait objects. The sampler contract already requires
    // both, so use the checked standard-to-native marker bridge.
    #[cfg(not(test))]
    sampler: IntoDynSyncSend<Arc<dyn Sampler>>,
    #[cfg(test)]
    sampler: Arc<dyn Sampler>,
    capped: AtomicBool,
    lost: AtomicU64,
    unknown_inventory: AtomicBool,
    overflow: AtomicBool,
    events: Mutex<Vec<(Source, Sample)>>,
}
impl TaskBindings {
    pub fn new(sampler: Arc<dyn Sampler>) -> Self {
        #[cfg(not(test))]
        let sampler = IntoDynSyncSend(sampler);
        Self {
            sampler,
            capped: AtomicBool::new(false),
            lost: AtomicU64::new(0),
            unknown_inventory: AtomicBool::new(false),
            overflow: AtomicBool::new(false),
            events: Mutex::new(Vec::new()),
        }
    }
    /// Mark allocator wrap permanently, even if later allocations restore a plausible high water.
    pub fn allocator_id(&self, id: u64) {
        if id == 0 || id == u64::MAX {
            self.unknown_inventory.store(true, Ordering::Relaxed);
        }
    }
    fn loss(&self) {
        if self.lost.fetch_add(1, Ordering::Relaxed) == u64::MAX {
            self.overflow.store(true, Ordering::Relaxed);
        }
    }
    /// None means a permanently capped recorder. No TLS, clock, or mutex work occurs.
    pub fn sample(&self) -> Option<Sample> {
        if self.capped.load(Ordering::Relaxed) { None } else { Some(self.sampler.sample()) }
    }
    pub fn record(&self, source: Source) {
        self.record_sample(source, self.sample());
    }
    pub fn record_sample(&self, source: Source, point: Option<Sample>) {
        if self.capped.load(Ordering::Relaxed) || point.is_none() {
            self.loss();
            return;
        }
        let mut events = self.events.lock().unwrap_or_else(|error| {
            self.loss();
            error.into_inner()
        });
        if events.len() < MAX_EVENTS && events.try_reserve(1).is_ok() {
            events.push((source, point.unwrap()));
            if events.len() == MAX_EVENTS {
                self.capped.store(true, Ordering::Relaxed);
            }
        } else {
            self.loss();
        }
    }
    pub fn append_json(
        &self,
        out: &mut String,
        contexts: &[(u64, Option<u64>)],
        expected_contexts: Option<u64>,
        highs: [u64; 4],
    ) {
        let expected = if self.unknown_inventory.load(Ordering::Relaxed) {
            None
        } else {
            expected_points(contexts, expected_contexts, highs)
        };
        let events = self.events.lock();
        let poisoned = events.is_err();
        let events = events.unwrap_or_else(|e| e.into_inner());
        let lost = self.lost.load(Ordering::Relaxed);
        let overflow = self.overflow.load(Ordering::Relaxed) || (poisoned && lost == u64::MAX);
        let lost = lost.saturating_add(u64::from(poisoned));
        let complete = lost == 0
            && !overflow
            && inventory(&events, contexts, expected_contexts, highs, expected);
        let pool = self.sampler.authority();
        let pool_text = pool.as_ref().map_or_else(
            || "null".into(),
            |(capture, observer)| {
                format!("{{\"capture_id\":\"{capture}\",\"observer\":{observer}}}")
            },
        );
        let unavailable = if pool.is_none() { "\"pool_unavailable\"" } else { "null" };
        let bound = complete && pool.is_some() && events.iter().all(|(_, point)| point.is_ok());
        let expected = expected.map_or_else(|| "null".into(), |n| n.to_string());
        let _ = write!(
            out,
            ",\"task_bindings\":{{\"schema\":1,\"records_complete\":{complete},\"all_targets_bound\":{bound},\"expected_points\":{expected},\"lost_events\":{lost},\"loss_overflowed\":{overflow},\"pool\":{pool_text},\"unavailable_reason\":{unavailable},\"events\":["
        );
        for (i, (source, point)) in events.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            out.push_str("{\"source\":{");
            match source {
                Source::Admitted { context_id, job_id } => {
                    let _ = write!(
                        out,
                        "\"kind\":\"admitted\",\"context_id\":{context_id},\"job_id\":{job_id}"
                    );
                }
                Source::RequestStarted { request_id } => {
                    let _ = write!(out, "\"kind\":\"request_started\",\"request_id\":{request_id}");
                }
                Source::DepRead { read_id } => {
                    let _ = write!(out, "\"kind\":\"dep_read\",\"read_id\":{read_id}");
                }
                Source::ResultProduced { result_id } => {
                    let _ = write!(out, "\"kind\":\"result_produced\",\"result_id\":{result_id}");
                }
                Source::WaitRegistered { wait_id } => {
                    let _ = write!(out, "\"kind\":\"wait_registered\",\"wait_id\":{wait_id}");
                }
                Source::WaitResumed { wait_id } => {
                    let _ = write!(out, "\"kind\":\"wait_resumed\",\"wait_id\":{wait_id}");
                }
            }
            out.push_str("},\"target\":");
            match point {
                Ok(Point { task, at_pool_ns }) => {
                    let _ = write!(
                        out,
                        "{{\"task\":{task},\"at_pool_ns\":{at_pool_ns}}},\"unbound_reason\":null}}"
                    );
                }
                Err(reason) => {
                    let _ = write!(out, "null,\"unbound_reason\":\"{}\"}}", reason.name());
                }
            }
        }
        out.push_str("]}");
    }
}
fn expected_points(
    contexts: &[(u64, Option<u64>)],
    count: Option<u64>,
    highs: [u64; 4],
) -> Option<u64> {
    if contexts.len() as u64 != count? {
        return None;
    }
    let mut total = 0u64;
    for &(_, high) in contexts {
        total = total.checked_add(high?.checked_sub(1)?)?;
    }
    for high in &highs[..3] {
        total = total.checked_add(high.checked_sub(1)?)?;
    }
    total.checked_add(highs[3].checked_sub(1)?.checked_mul(2)?)
}
fn inventory(
    events: &[(Source, Sample)],
    contexts: &[(u64, Option<u64>)],
    count: Option<u64>,
    highs: [u64; 4],
    expected: Option<u64>,
) -> bool {
    if expected != Some(events.len() as u64) {
        return false;
    }
    let mut context_ids = Vec::new();
    if context_ids.try_reserve(contexts.len()).is_err() {
        return false;
    }
    context_ids.extend(contexts.iter().map(|c| c.0));
    context_ids.sort_unstable();
    if count != Some(context_ids.len() as u64)
        || !context_ids.iter().enumerate().all(|(i, &id)| id == i as u64 + 1)
    {
        return false;
    }
    let mut actual = Vec::new();
    if actual.try_reserve(events.len()).is_err() {
        return false;
    }
    actual.extend(events.iter().map(|e| e.0));
    actual.sort_unstable();
    if actual.windows(2).any(|p| p[0] == p[1]) {
        return false;
    }
    actual.iter().all(|source| match *source {
        Source::Admitted { context_id, job_id } => contexts
            .iter()
            .any(|&(id, high)| id == context_id && job_id > 0 && high.is_some_and(|h| job_id < h)),
        Source::RequestStarted { request_id } => request_id > 0 && request_id < highs[0],
        Source::DepRead { read_id } => read_id > 0 && read_id < highs[1],
        Source::ResultProduced { result_id } => result_id > 0 && result_id < highs[2],
        Source::WaitRegistered { wait_id } | Source::WaitResumed { wait_id } => {
            wait_id > 0 && wait_id < highs[3]
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestSampler {
        attached: AtomicBool,
        calls: AtomicU64,
    }
    impl Sampler for TestSampler {
        fn sample(&self) -> Sample {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.attached.load(Ordering::Relaxed) {
                Ok(Point { task: 9, at_pool_ns: 17 })
            } else {
                Err(Unbound::PoolUnavailable)
            }
        }
        fn authority(&self) -> Option<(String, u64)> {
            self.attached.load(Ordering::Relaxed).then(|| ("query-pool".into(), 7))
        }
    }
    fn recorder() -> (TaskBindings, Arc<TestSampler>) {
        let sampler =
            Arc::new(TestSampler { attached: AtomicBool::new(true), calls: AtomicU64::new(0) });
        (TaskBindings::new(sampler.clone()), sampler)
    }
    fn json(
        recorder: &TaskBindings,
        contexts: &[(u64, Option<u64>)],
        count: Option<u64>,
        highs: [u64; 4],
    ) -> String {
        let mut out = String::new();
        recorder.append_json(&mut out, contexts, count, highs);
        out
    }
    #[test]
    fn checked_expected_inventory_is_not_derived_from_events() {
        assert_eq!(expected_points(&[(1, Some(3)), (2, Some(2))], Some(2), [4, 5, 6, 7]), Some(27));
        assert_eq!(expected_points(&[(1, None)], Some(1), [1; 4]), None);
        assert_eq!(expected_points(&[], Some(1), [1; 4]), None);
        assert_eq!(expected_points(&[], Some(0), [0, 1, 1, 1]), None);
        assert_eq!(expected_points(&[], Some(0), [1, 1, 1, u64::MAX]), None);
        assert_eq!(expected_points(&[(1, Some(u64::MAX))], Some(1), [3, 1, 1, 1]), None);
    }
    #[test]
    fn category_removal_duplicate_or_substitution_fails_inventory() {
        let (recorder, _) = recorder();
        let sources = [
            Source::Admitted { context_id: 1, job_id: 1 },
            Source::RequestStarted { request_id: 1 },
            Source::DepRead { read_id: 1 },
            Source::ResultProduced { result_id: 1 },
            Source::WaitRegistered { wait_id: 1 },
            Source::WaitResumed { wait_id: 1 },
        ];
        for source in sources {
            recorder.record(source);
        }
        let contexts = [(1, Some(2))];
        assert!(json(&recorder, &contexts, Some(1), [2; 4]).contains("\"records_complete\":true"));
        for i in 0..sources.len() {
            let removed = recorder.events.lock().unwrap().remove(i);
            assert!(
                json(&recorder, &contexts, Some(1), [2; 4]).contains("\"records_complete\":false")
            );
            recorder.events.lock().unwrap().insert(i, removed);
        }
        recorder.events.lock().unwrap()[1].0 = sources[2];
        assert!(json(&recorder, &contexts, Some(1), [2; 4]).contains("\"records_complete\":false"));
        assert!(json(&recorder, &[], Some(1), [2; 4]).contains("\"expected_points\":null"));
    }
    #[test]
    fn early_unavailable_stays_null_after_pool_attach() {
        let (recorder, sampler) = recorder();
        sampler.attached.store(false, Ordering::Relaxed);
        recorder.record(Source::DepRead { read_id: 1 });
        assert!(
            json(&recorder, &[], Some(0), [1, 2, 1, 1])
                .contains("\"pool\":null,\"unavailable_reason\":\"pool_unavailable\"")
        );
        sampler.attached.store(true, Ordering::Relaxed);
        let out = json(&recorder, &[], Some(0), [1, 2, 1, 1]);
        assert!(out.contains("\"records_complete\":true,\"all_targets_bound\":false"));
        assert!(out.contains(
            "\"pool\":{\"capture_id\":\"query-pool\",\"observer\":7},\"unavailable_reason\":null"
        ));
        assert!(out.contains("\"target\":null,\"unbound_reason\":\"pool_unavailable\""));
    }
    #[test]
    fn every_unbound_reason_is_explicit_and_not_loss() {
        let (recorder, _) = recorder();
        for (i, reason) in [
            Unbound::PoolUnavailable,
            Unbound::NoActive,
            Unbound::Suspended,
            Unbound::ForeignObserver,
            Unbound::ClockOverflow,
        ]
        .into_iter()
        .enumerate()
        {
            recorder.record_sample(Source::DepRead { read_id: i as u64 + 1 }, Some(Err(reason)));
        }
        let out = json(&recorder, &[], Some(0), [1, 6, 1, 1]);
        assert!(out.contains("\"records_complete\":true,\"all_targets_bound\":false"));
        assert_eq!(out.matches("\"target\":null").count(), 5);
        assert!(out.contains("\"lost_events\":0"));
    }
    #[test]
    fn exact_cap_is_complete_and_permanent_cap_skips_sampler_and_mutex() {
        let (recorder, sampler) = recorder();
        for id in 1..=MAX_EVENTS as u64 {
            recorder.record(Source::DepRead { read_id: id });
        }
        assert!(
            json(&recorder, &[], Some(0), [1, MAX_EVENTS as u64 + 1, 1, 1])
                .contains("\"records_complete\":true,\"all_targets_bound\":true")
        );
        let guard = recorder.events.lock().unwrap();
        recorder.record(Source::DepRead { read_id: MAX_EVENTS as u64 + 1 });
        recorder.record_sample(Source::WaitRegistered { wait_id: 1 }, recorder.sample());
        recorder.record_sample(Source::WaitResumed { wait_id: 1 }, recorder.sample());
        assert_eq!(sampler.calls.load(Ordering::Relaxed), MAX_EVENTS as u64);
        assert_eq!(recorder.lost.load(Ordering::Relaxed), 3);
        drop(guard);
        recorder.events.lock().unwrap().clear();
        recorder.record(Source::DepRead { read_id: 1 });
        assert!(recorder.events.lock().unwrap().is_empty());
        assert_eq!(recorder.lost.load(Ordering::Relaxed), 4);
    }
    #[test]
    fn poison_and_loss_wrap_permanently_fail_incomplete() {
        let (recorder, _) = recorder();
        assert!(
            std::panic::catch_unwind(|| {
                let _guard = recorder.events.lock().unwrap();
                panic!("poison");
            })
            .is_err()
        );
        recorder.record(Source::DepRead { read_id: 1 });
        assert!(json(&recorder, &[], Some(0), [1, 2, 1, 1]).contains("\"records_complete\":false"));
        recorder.lost.store(u64::MAX, Ordering::Relaxed);
        recorder.capped.store(true, Ordering::Relaxed);
        recorder.record(Source::DepRead { read_id: 2 });
        let out = json(&recorder, &[], Some(0), [1, 2, 1, 1]);
        assert!(out.contains("\"loss_overflowed\":true"));
        assert!(out.contains("\"records_complete\":false"));
    }
    #[test]
    fn allocator_wrap_stays_unknown_after_plausible_high_water() {
        let (recorder, _) = recorder();
        recorder.allocator_id(u64::MAX);
        recorder.allocator_id(1);
        let out = json(&recorder, &[], Some(0), [1; 4]);
        assert!(out.contains("\"expected_points\":null"));
        assert!(out.contains("\"records_complete\":false"));
    }
    #[test]
    fn wait_registration_sampling_does_not_publish_or_lock() {
        let (recorder, _) = recorder();
        let guard = recorder.events.lock().unwrap();
        let registered = recorder.sample();
        assert_eq!(guard.len(), 0);
        drop(guard);
        assert!(json(&recorder, &[], Some(0), [1, 1, 1, 2]).contains("\"records_complete\":false"));
        recorder.record_sample(Source::WaitRegistered { wait_id: 1 }, registered);
        recorder.record(Source::WaitResumed { wait_id: 1 });
        assert!(json(&recorder, &[], Some(0), [1, 1, 1, 2]).contains("\"records_complete\":true"));
    }
}
