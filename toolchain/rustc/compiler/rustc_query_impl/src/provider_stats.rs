//! Opt-in, request-scoped provider telemetry. This observes execution only and
//! never participates in query results, incremental decisions, or cache keys.
//!
//! The driver must serialize requests and keep the guard alive until all query
//! workers have joined. Nested scopes restore the outer collector. Snapshots
//! contain completed calls (including unwound calls), not in-flight calls.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use parking_lot::Mutex;

/// At most this many query names plus one overflow row are retained.
pub const MAX_QUERY_ROWS: usize = 512;
const OVERFLOW_NAME: &str = "<other queries>";

static ENABLED: AtomicBool = AtomicBool::new(false);
static ACTIVE: Mutex<Option<Arc<Collector>>> = Mutex::new(None);

thread_local! {
    // Sum of exclusive durations of completed providers on this OS thread.
    // A parent subtracts the change since entry, avoiding an unbounded stack.
    static COMPLETED_NANOS: Cell<u128> = const { Cell::new(0) };
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub name: &'static str,
    pub provider_count: u64,
    /// Inclusive wall time around actual provider invocation only.
    pub provider_nanos: u64,
    pub green_recompute_count: u64,
    /// Subset of provider_nanos: green nodes whose value was not reusable.
    pub green_recompute_nanos: u64,
    /// Wall time excluding nested, instrumented providers on the same thread.
    /// Includes blocking and does not claim to measure CPU time. Parallel
    /// children on other threads are not subtracted.
    pub provider_exclusive_nanos: u64,
    pub green_recompute_exclusive_nanos: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhaseRow {
    pub name: &'static str,
    pub count: u64,
    /// Inclusive wall time, separate from provider exclusive accounting.
    pub nanos: u64,
}

#[derive(Default)]
struct Collector {
    rows: Mutex<BTreeMap<&'static str, Row>>,
    phases: Mutex<BTreeMap<&'static str, PhaseRow>>,
}

impl Collector {
    fn record(&self, name: &'static str, green: bool, elapsed: u64, exclusive: u64) {
        let mut rows = self.rows.lock();
        let name = if rows.contains_key(name) || rows.len() < MAX_QUERY_ROWS {
            name
        } else {
            OVERFLOW_NAME
        };
        let row = rows.entry(name).or_insert_with(|| Row { name, ..Row::default() });
        row.provider_count = row.provider_count.saturating_add(1);
        row.provider_nanos = row.provider_nanos.saturating_add(elapsed);
        row.provider_exclusive_nanos = row.provider_exclusive_nanos.saturating_add(exclusive);
        if green {
            row.green_recompute_count = row.green_recompute_count.saturating_add(1);
            row.green_recompute_nanos = row.green_recompute_nanos.saturating_add(elapsed);
            row.green_recompute_exclusive_nanos =
                row.green_recompute_exclusive_nanos.saturating_add(exclusive);
        }
    }
}

/// A fresh request collector. Dropping restores the previous scope, even on unwind.
/// Disabled scopes suppress an enclosing enabled scope without taking timestamps.
#[must_use]
pub struct Guard {
    collector: Option<Arc<Collector>>,
    previous: Option<Arc<Collector>>,
    // Request scopes must be dropped on the controlling thread in stack order.
    _not_send: PhantomData<Rc<()>>,
}

/// Begin a fresh bounded capture, or explicitly disable capture for this scope.
/// Default is off. The caller, not this module, selects any flags or environment.
pub fn begin(enabled: bool) -> Guard {
    let collector = enabled.then(|| Arc::new(Collector::default()));
    let mut active = ACTIVE.lock();
    let previous = std::mem::replace(&mut *active, collector.clone());
    ENABLED.store(enabled, Ordering::Relaxed);
    Guard { collector, previous, _not_send: PhantomData }
}

impl Guard {
    pub fn phases(&self) -> Vec<PhaseRow> {
        self.collector
            .as_ref()
            .map_or_else(Vec::new, |c| c.phases.lock().values().cloned().collect())
    }

    /// Query-name sorted rows for this request, including after a nested scope.
    pub fn snapshot(&self) -> Vec<Row> {
        self.collector.as_ref().map_or_else(Vec::new, |c| c.rows.lock().values().cloned().collect())
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let mut active = ACTIVE.lock();
        *active = self.previous.take();
        ENABLED.store(active.is_some(), Ordering::Relaxed);
    }
}

/// Snapshot the currently active request. Prefer Guard::snapshot when available.
pub fn snapshot() -> Vec<Row> {
    let collector = ACTIVE.lock().clone();
    collector.map_or_else(Vec::new, |c| c.rows.lock().values().cloned().collect())
}

/// Independent inclusive phase timer. It never changes provider nesting counters.
/// Disabled capture performs no timestamp or allocation.
#[must_use]
pub struct PhaseTimer {
    collector: Arc<Collector>,
    name: &'static str,
    start: Instant,
}

pub fn phase(name: &'static str) -> Option<PhaseTimer> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    let collector = ACTIVE.lock().clone()?;
    Some(PhaseTimer { collector, name, start: Instant::now() })
}

impl PhaseTimer {
    /// Complete an interval whose fixed label is selected at its boundary.
    pub fn finish_as(mut self, name: &'static str) {
        self.name = name;
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        let elapsed = nanos(self.start.elapsed().as_nanos());
        let mut rows = self.collector.phases.lock();
        let name = if rows.contains_key(self.name) || rows.len() < 64 {
            self.name
        } else {
            "<other phases>"
        };
        let row = rows.entry(name).or_insert_with(|| PhaseRow { name, ..PhaseRow::default() });
        row.count = row.count.saturating_add(1);
        row.nanos = row.nanos.saturating_add(elapsed);
    }
}

pub(crate) struct ProviderTimer {
    collector: Arc<Collector>,
    name: &'static str,
    green: bool,
    start: Instant,
    completed_at_start: u128,
    _not_send: PhantomData<Rc<()>>,
}

/// The disabled path is one relaxed load, with no clock, TLS, lock or allocation.
#[inline(always)]
pub(crate) fn provider(name: &'static str, green: bool) -> Option<ProviderTimer> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    provider_enabled(name, green)
}

#[cold]
#[inline(never)]
fn provider_enabled(name: &'static str, green: bool) -> Option<ProviderTimer> {
    let collector = ACTIVE.lock().clone()?;
    Some(ProviderTimer {
        collector,
        name,
        green,
        completed_at_start: COMPLETED_NANOS.with(Cell::get),
        start: Instant::now(),
        _not_send: PhantomData,
    })
}

fn exclusive_duration(elapsed: u128, before: u128, completed: &Cell<u128>) -> u128 {
    let now = completed.get();
    let exclusive = elapsed.saturating_sub(now.saturating_sub(before));
    completed.set(now.saturating_add(exclusive));
    exclusive
}

fn nanos(value: u128) -> u64 {
    value.min(u64::MAX as u128) as u64
}

impl Drop for ProviderTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_nanos();
        let exclusive = COMPLETED_NANOS
            .with(|completed| exclusive_duration(elapsed, self.completed_at_start, completed));
        self.collector.record(self.name, self.green, nanos(elapsed), nanos(exclusive));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Process-wide capture requires serialization, just like driver requests.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn phases_are_separate_bounded_and_reset() {
        let _serial = TEST_LOCK.lock();
        assert!(phase("disabled").is_none());
        let request = begin(true);
        {
            let _provider = provider("provider", false);
            drop(phase("frontend"));
        }
        assert_eq!(request.snapshot().len(), 1);
        assert_eq!(request.phases()[0].name, "frontend");
        assert_eq!(request.phases()[0].count, 1);
        for i in 0..70 {
            let name = Box::leak(format!("phase_{i}").into_boxed_str());
            drop(phase(name));
        }
        assert_eq!(request.phases().len(), 65);
        assert!(request.phases().iter().any(|row| row.name == "<other phases>"));
        drop(request);
        let fresh = begin(true);
        assert!(fresh.phases().is_empty());
    }

    #[test]
    fn reset_nested_disabled_and_unwind() {
        let _serial = TEST_LOCK.lock();
        assert!(provider("default_off", false).is_none());
        let outer = begin(true);
        drop(provider("outer", false));
        {
            let disabled = begin(false);
            assert!(provider("disabled", true).is_none());
            assert!(disabled.snapshot().is_empty());
            assert!(snapshot().is_empty());
        }
        let result = std::panic::catch_unwind(|| {
            let inner = begin(true);
            assert!(inner.snapshot().is_empty());
            let _timer = provider("unwound", true);
            panic!("test unwind");
        });
        assert!(result.is_err());
        drop(provider("outer", true));
        let rows = outer.snapshot();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider_count, 2);
        assert_eq!(rows[0].green_recompute_count, 1);
        drop(outer);
        assert!(snapshot().is_empty());
        assert!(provider("off_again", false).is_none());
        let fresh = begin(true);
        assert!(fresh.snapshot().is_empty());
        let result = std::panic::catch_unwind(|| {
            let _timer = provider("unwound", true);
            panic!("provider unwind");
        });
        assert!(result.is_err());
        assert_eq!(fresh.snapshot()[0].green_recompute_count, 1);
    }

    #[test]
    fn native_nested_calls_and_worker_threads_share_request() {
        let _serial = TEST_LOCK.lock();
        let request = begin(true);
        {
            let _parent = provider("parent", false);
            drop(provider("child", true));
        }
        std::thread::scope(|scope| {
            scope.spawn(|| drop(provider("worker", true))).join().unwrap();
        });
        let rows = request.snapshot();
        assert_eq!(rows.len(), 3);
        let parent = rows.iter().find(|row| row.name == "parent").unwrap();
        let child = rows.iter().find(|row| row.name == "child").unwrap();
        assert_eq!(parent.provider_count, 1);
        assert_eq!(parent.green_recompute_count, 0);
        assert_eq!(child.green_recompute_count, 1);
        assert!(parent.provider_exclusive_nanos <= parent.provider_nanos);
        assert_eq!(
            parent.provider_nanos.saturating_sub(parent.provider_exclusive_nanos),
            child.provider_nanos
        );
        assert_eq!(rows.iter().find(|row| row.name == "worker").unwrap().provider_count, 1);
    }

    #[test]
    fn nested_exclusive_accounting_is_stack_free() {
        let completed = Cell::new(0);
        let parent_start = completed.get();
        let child_start = completed.get();
        assert_eq!(exclusive_duration(20, completed.get(), &completed), 20);
        assert_eq!(exclusive_duration(50, child_start, &completed), 30);
        assert_eq!(exclusive_duration(100, parent_start, &completed), 50);
        assert_eq!(completed.get(), 100);
        assert_eq!(nanos(u128::MAX), u64::MAX);
    }

    #[test]
    fn aggregation_is_bounded_and_saturating() {
        let collector = Collector::default();
        // Seed the fixed limit without leaking dynamically allocated names.
        for index in 0..MAX_QUERY_ROWS {
            // BTreeMap keys must be distinct static strings. Suffixes of this
            // static string provide enough distinct names without allocation.
            const NAMES: &str = concat!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            );
            collector.record(&NAMES[index..], false, 1, 1);
        }
        collector.record("overflow", true, u64::MAX, u64::MAX);
        collector.record("another overflow", true, 1, 1);
        let mut rows = collector.rows.lock();
        assert_eq!(rows.len(), MAX_QUERY_ROWS + 1);
        let overflow = rows.get_mut(OVERFLOW_NAME).unwrap();
        assert_eq!(overflow.provider_nanos, u64::MAX);
        assert_eq!(overflow.green_recompute_exclusive_nanos, u64::MAX);
        overflow.provider_count = u64::MAX;
        overflow.green_recompute_count = u64::MAX;
        drop(rows);
        collector.record("overflow", true, 1, 1);
        let rows = collector.rows.lock();
        assert_eq!(rows[OVERFLOW_NAME].provider_count, u64::MAX);
        assert_eq!(rows[OVERFLOW_NAME].green_recompute_count, u64::MAX);
    }
}
