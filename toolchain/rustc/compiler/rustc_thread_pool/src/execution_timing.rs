//! Opt-in exclusive worker CPU intervals. Gaps around instrumentation are unknown,
//! not evidence of zero work. A single TLS cursor prevents nested/help double counts.
use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

static NEXT_THREAD: AtomicU64 = AtomicU64::new(1);
static THREAD_EXHAUSTED: AtomicBool = AtomicBool::new(false);
static TLS_FAILED: AtomicBool = AtomicBool::new(false);
thread_local! {
    static THREAD_ID: Cell<u64> = const { Cell::new(0) };
    static CURSOR: RefCell<Option<Cursor>> = const { RefCell::new(None) };
    static QUERY_OWNER: RefCell<Option<QueryOwnership>> = const { RefCell::new(None) };
}
#[derive(Clone)]
pub(crate) struct QueryOwnership {
    recorder: Arc<Recorder>,
    context: u64,
    job: u64,
    name: &'static str,
}
/// Worker-local ownership, never inherited by stolen work or reconstructed from query TLS.
pub(crate) fn suspend_query() -> Option<QueryOwnership> {
    QUERY_OWNER
        .try_with(|owner| owner.try_borrow_mut().map(|mut slot| slot.take()))
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| {
            TLS_FAILED.store(true, Ordering::Relaxed);
            None
        })
}
pub(crate) fn restore_query(owner: Option<QueryOwnership>) {
    if QUERY_OWNER
        .try_with(|slot| slot.try_borrow_mut().map(|mut slot| *slot = owner))
        .ok()
        .and_then(Result::ok)
        .is_none()
    {
        TLS_FAILED.store(true, Ordering::Relaxed);
    }
}
pub(crate) fn current_query_recorder() -> Option<Arc<Recorder>> {
    QUERY_OWNER
        .try_with(|slot| {
            slot.try_borrow().map(|slot| slot.as_ref().map(|owner| Arc::clone(&owner.recorder)))
        })
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| {
            TLS_FAILED.store(true, Ordering::Relaxed);
            None
        })
}
#[cfg(test)]
pub(crate) fn current_query_identity() -> Option<(u64, u64)> {
    QUERY_OWNER.with(|slot| slot.borrow().as_ref().map(|owner| (owner.context, owner.job)))
}
pub struct QueryExecutionGuard {
    previous: Option<QueryOwnership>,
    previous_recorder: Option<Arc<Recorder>>,
    _not_send: PhantomData<Rc<()>>,
}
impl Drop for QueryExecutionGuard {
    fn drop(&mut self) {
        pause();
        restore_query(self.previous.take());
        crate::execution_observer::resume_current_timing(self.previous_recorder.as_ref());
    }
}
pub(crate) fn enter_query(
    recorder: &Arc<Recorder>,
    context: u64,
    job: u64,
    name: &'static str,
    task: Option<u64>,
) -> Option<QueryExecutionGuard> {
    if !recorder.query_ownership || context == 0 || job == 0 {
        return None;
    }
    // QueryVTable names are compile-time Rust identifiers, never query keys,
    // formatted values, symbols or source paths. Reject rather than escape an
    // unexpected caller-provided category into the private capture protocol.
    if name.is_empty()
        || name.len() > 128
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || name.as_bytes()[0].is_ascii_digit()
    {
        recorder.loss();
        return None;
    }
    let previous_recorder = pause();
    let previous = suspend_query();
    restore_query(Some(QueryOwnership { recorder: Arc::clone(recorder), context, job, name }));
    resume(Some(recorder), task);
    Some(QueryExecutionGuard { previous, previous_recorder, _not_send: PhantomData })
}
fn allocate(counter: &AtomicU64) -> Option<u64> {
    counter.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1)).ok()
}
fn thread_id() -> Option<u64> {
    THREAD_ID
        .try_with(|slot| match slot.get() {
            0 => match allocate(&NEXT_THREAD) {
                Some(id) => {
                    slot.set(id);
                    Some(id)
                }
                None => {
                    THREAD_EXHAUSTED.store(true, Ordering::Relaxed);
                    slot.set(u64::MAX);
                    None
                }
            },
            u64::MAX => None,
            id => Some(id),
        })
        .unwrap_or_else(|_| {
            TLS_FAILED.store(true, Ordering::Relaxed);
            None
        })
}
#[derive(Clone, Copy)]
pub(crate) struct Sample {
    before: Option<u64>,
    cpu: Option<u64>,
    after: Option<u64>,
}
trait Clock: Send + Sync + std::panic::RefUnwindSafe {
    fn sample(&self, epoch: Instant) -> Sample;
}
struct NativeClock;
impl Clock for NativeClock {
    fn sample(&self, epoch: Instant) -> Sample {
        let before = u64::try_from(epoch.elapsed().as_nanos()).ok();
        let cpu = thread_cpu();
        let after = u64::try_from(epoch.elapsed().as_nanos()).ok();
        Sample { before, cpu, after }
    }
}
#[cfg(target_os = "linux")]
fn thread_cpu() -> Option<u64> {
    let mut value = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: the pointer is writable and initialized only on a successful call.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, value.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: clock_gettime initialized the timespec after returning success.
    let value = unsafe { value.assume_init() };
    if value.tv_sec < 0 || !(0..1_000_000_000).contains(&value.tv_nsec) {
        return None;
    }
    u64::try_from(value.tv_sec)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u64::try_from(value.tv_nsec).ok()?)
}
#[cfg(not(target_os = "linux"))]
fn thread_cpu() -> Option<u64> {
    None
}

struct Cursor {
    recorder: Arc<Recorder>,
    id: Option<u64>,
    thread: Option<u64>,
    task: Option<u64>,
    query: Option<(u64, u64, &'static str)>,
    start: Sample,
}
struct Segment {
    id: u64,
    thread: u64,
    task: Option<u64>,
    query: Option<(u64, u64, &'static str)>,
    start: Sample,
    end: Sample,
    cpu: Option<u64>,
    status: &'static str,
}
struct Buffer {
    rows: Vec<Segment>,
    sealed: bool,
    active_at_cutoff: u64,
}
pub(crate) struct Recorder {
    epoch: Instant,
    capacity: usize,
    query_ownership: bool,
    // Production remains concrete so compiler DynSend/DynSync are structural.
    #[cfg(not(test))]
    clock: NativeClock,
    #[cfg(test)]
    clock: Arc<dyn Clock>,
    next: AtomicU64,
    active: AtomicU64,
    lost: AtomicU64,
    loss_overflowed: AtomicBool,
    identity_exhausted: AtomicBool,
    clock_failures: AtomicU64,
    buffer: Mutex<Buffer>,
}
impl Recorder {
    pub(crate) fn new(epoch: Instant, capacity: usize) -> Arc<Self> {
        #[cfg(test)]
        {
            Self::with_clock(epoch, capacity, Arc::new(NativeClock))
        }
        #[cfg(not(test))]
        {
            Self::with_clock(epoch, capacity)
        }
    }
    pub(crate) fn new_with_query_ownership(epoch: Instant, capacity: usize) -> Arc<Self> {
        let mut recorder = Self::new(epoch, capacity);
        Arc::get_mut(&mut recorder).unwrap().query_ownership = true;
        recorder
    }
    pub(crate) fn query_ownership_enabled(&self) -> bool {
        self.query_ownership
    }
    fn with_clock(
        epoch: Instant,
        capacity: usize,
        #[cfg(test)] clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            epoch,
            capacity,
            query_ownership: false,
            #[cfg(test)]
            clock,
            #[cfg(not(test))]
            clock: NativeClock,
            next: AtomicU64::new(1),
            active: AtomicU64::new(0),
            lost: AtomicU64::new(0),
            loss_overflowed: AtomicBool::new(false),
            identity_exhausted: AtomicBool::new(false),
            clock_failures: AtomicU64::new(0),
            buffer: Mutex::new(Buffer { rows: Vec::new(), sealed: false, active_at_cutoff: 0 }),
        })
    }
    fn loss(&self) {
        if self.lost.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1)).is_err()
        {
            self.loss_overflowed.store(true, Ordering::Relaxed);
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Buffer> {
        self.buffer.lock().unwrap_or_else(|e| {
            self.loss();
            e.into_inner()
        })
    }
    fn begin(self: &Arc<Self>, task: Option<u64>) -> Option<Cursor> {
        let buffer = self.lock();
        if buffer.sealed {
            self.loss();
            return None;
        }
        let id = allocate(&self.next);
        let thread = thread_id();
        if id.is_none() || thread.is_none() {
            self.identity_exhausted.store(true, Ordering::Relaxed);
        }
        if self
            .active
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .is_err()
        {
            self.identity_exhausted.store(true, Ordering::Relaxed);
        }
        drop(buffer);
        Some(Cursor {
            recorder: Arc::clone(self),
            id,
            thread,
            task,
            query: if self.query_ownership {
                QUERY_OWNER
                    .try_with(|slot| {
                        slot.try_borrow().map(|slot| {
                            slot.as_ref()
                                .filter(|owner| Arc::ptr_eq(&owner.recorder, self))
                                .map(|owner| (owner.context, owner.job, owner.name))
                        })
                    })
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_else(|| {
                        TLS_FAILED.store(true, Ordering::Relaxed);
                        None
                    })
            } else {
                None
            },
            start: self.clock.sample(self.epoch),
        })
    }
    fn end(&self, cursor: Cursor) {
        let mut end = self.clock.sample(self.epoch);
        let _ = self.active.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1));
        let valid_wall = cursor
            .start
            .before
            .zip(cursor.start.after)
            .zip(end.before.zip(end.after))
            .is_some_and(|((a, b), (c, d))| a <= b && b <= c && c <= d);
        if !valid_wall {
            end.after = None;
        }
        let cpu_failed = cursor.start.cpu.is_none() || end.cpu.is_none();
        if cpu_failed
            && self
                .clock_failures
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .is_err()
        {
            self.loss_overflowed.store(true, Ordering::Relaxed);
        }
        let status = if !valid_wall {
            "wall_overflow"
        } else if cpu_failed {
            "clock_failed"
        } else if end.cpu < cursor.start.cpu {
            "clock_regressed"
        } else {
            "measured"
        };
        let cpu = if status == "measured" {
            end.cpu.zip(cursor.start.cpu).and_then(|(b, a)| b.checked_sub(a))
        } else {
            None
        };
        let (Some(id), Some(thread)) = (cursor.id, cursor.thread) else {
            self.loss();
            return;
        };
        let mut buffer = self.lock();
        if !buffer.sealed && buffer.rows.len() < self.capacity && buffer.rows.try_reserve(1).is_ok()
        {
            buffer.rows.push(Segment {
                id,
                thread,
                task: cursor.task,
                query: cursor.query,
                start: cursor.start,
                end,
                cpu,
                status,
            });
        } else {
            self.loss();
        }
    }
    pub(crate) fn seal(&self) {
        let mut buffer = self.lock();
        if !buffer.sealed {
            buffer.active_at_cutoff = self.active.load(Ordering::Relaxed);
            buffer.sealed = true;
        }
    }
    pub(crate) fn write_json(
        &self,
        out: &mut impl std::io::Write,
        query: &str,
        pool: &str,
        observer: u64,
    ) -> std::io::Result<()> {
        let buffer = self.lock();
        let exhausted = self.identity_exhausted.load(Ordering::Relaxed)
            || THREAD_EXHAUSTED.load(Ordering::Relaxed)
            || TLS_FAILED.load(Ordering::Relaxed);
        let high = self.next.load(Ordering::Relaxed);
        let active = if buffer.sealed {
            buffer.active_at_cutoff
        } else {
            self.active.load(Ordering::Relaxed)
        };
        let lost = self.lost.load(Ordering::Relaxed);
        let overflow = self.loss_overflowed.load(Ordering::Relaxed);
        let complete = buffer.sealed
            && active == 0
            && lost == 0
            && !overflow
            && !exhausted
            && buffer.rows.len() as u64 == high - 1
            && buffer.rows.iter().all(|row| row.status == "measured");
        let high_text = if exhausted { "null".into() } else { high.to_string() };
        let expected = if exhausted { "null".into() } else { (high - 1).to_string() };
        let threads = if THREAD_EXHAUSTED.load(Ordering::Relaxed) {
            "null".into()
        } else {
            NEXT_THREAD.load(Ordering::Relaxed).to_string()
        };
        write!(
            out,
            ",\"timing\":{{\"schema\":{},\"scope\":\"observed-exclusive-thread-segments\",\"clock_bridge\":{{\"query_capture_id\":\"{query}\",\"pool_capture_id\":\"{pool}\",\"pool_observer\":{observer},\"pool_epoch_in_query_ns\":0}},\"cpu_clock\":\"linux-thread-cpu\",\"segment_allocator_baseline\":1,\"segment_allocator_high_water\":{high_text},\"expected_segments\":{expected},\"thread_allocator_high_water\":{threads},\"lost_segments\":{lost},\"loss_overflowed\":{overflow},\"identity_exhausted\":{exhausted},\"clock_failures\":{},\"active_segments\":{active},\"sealed\":{},\"records_complete\":{complete},\"segments\":[",
            if self.query_ownership { 2 } else { 1 },
            self.clock_failures.load(Ordering::Relaxed),
            buffer.sealed
        )?;
        for (index, row) in buffer.rows.iter().enumerate() {
            if index != 0 {
                write!(out, ",")?;
            }
            let mut text = String::new();
            let _ = write!(
                text,
                "{{\"segment_id\":{},\"thread_id\":{},\"task\":{},\"owner_kind\":\"{}\",\"start_ns\":{},\"end_ns\":{},\"cpu_start_ns\":{},\"cpu_end_ns\":{},\"cpu_ns\":{},\"status\":\"{}\"",
                row.id,
                row.thread,
                number(row.task),
                if row.query.is_some() {
                    "query_execution"
                } else if row.task.is_some() {
                    "task"
                } else {
                    "opaque"
                },
                number(row.start.before),
                number(row.end.after),
                number(row.start.cpu),
                number(row.end.cpu),
                number(row.cpu),
                row.status
            );
            if self.query_ownership {
                let _ = write!(
                    text,
                    ",\"query_context\":{},\"query_job\":{},\"query_name\":{}",
                    number(row.query.map(|q| q.0)),
                    number(row.query.map(|q| q.1)),
                    row.query.map_or_else(|| "null".into(), |q| format!("\"{}\"", q.2))
                );
            }
            text.push('}');
            out.write_all(text.as_bytes())?;
        }
        write!(out, "]}}")
    }
}
fn number(value: Option<u64>) -> String {
    value.map_or_else(|| "null".into(), |n| n.to_string())
}
/// No clocks or allocator counters are read on the disabled path.
pub(crate) fn has_active_cursor() -> bool {
    CURSOR
        .try_with(|cursor| cursor.try_borrow().map_or(true, |slot| slot.is_some()))
        .unwrap_or(false)
}
/// Stop before native lifecycle bookkeeping; never reuse the old CPU baseline.
pub(crate) fn pause() -> Option<Arc<Recorder>> {
    let cursor = CURSOR
        .try_with(|cursor| match cursor.try_borrow_mut() {
            Ok(mut slot) => slot.take(),
            Err(_) => {
                TLS_FAILED.store(true, Ordering::Relaxed);
                None
            }
        })
        .unwrap_or_else(|_| {
            TLS_FAILED.store(true, Ordering::Relaxed);
            None
        })?;
    let recorder = Arc::clone(&cursor.recorder);
    recorder.end(cursor);
    Some(recorder)
}
pub(crate) fn resume(recorder: Option<&Arc<Recorder>>, task: Option<u64>) {
    let _ = pause();
    if let Some(recorder) = recorder {
        if let Some(cursor) = recorder.begin(task) {
            let mut pending = Some(cursor);
            let _ = CURSOR.try_with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    *slot = pending.take();
                }
            });
            if let Some(cursor) = pending {
                recorder.loss();
                recorder.end(cursor);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeClock(Mutex<std::collections::VecDeque<Sample>>);
    impl Clock for FakeClock {
        fn sample(&self, _: Instant) -> Sample {
            self.0.lock().unwrap().pop_front().unwrap()
        }
    }
    fn sample(wall: u64, cpu: Option<u64>) -> Sample {
        Sample { before: Some(wall), after: Some(wall), cpu }
    }
    fn fake(samples: Vec<Sample>, capacity: usize) -> Arc<Recorder> {
        Recorder::with_clock(
            Instant::now(),
            capacity,
            Arc::new(FakeClock(Mutex::new(samples.into()))),
        )
    }
    fn json(recorder: &Recorder) -> String {
        let mut out = Vec::new();
        recorder.write_json(&mut out, "query", "pool", 1).unwrap();
        String::from_utf8(out).unwrap()
    }
    #[test]
    fn query_cpu_nesting_is_exclusive_and_unwind_restores_exact_context() {
        let mut recorder = fake((1..=6).map(|n| sample(n, Some(n))).collect(), 10);
        Arc::get_mut(&mut recorder).unwrap().query_ownership = true;
        {
            let _outer = enter_query(&recorder, 1, 7, "outer", None).unwrap();
            let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _inner = enter_query(&recorder, 2, 7, "inner", None).unwrap();
                assert_eq!(current_query_identity(), Some((2, 7)));
                panic!("restore exact query context");
            }));
            assert!(unwind.is_err());
            assert_eq!(current_query_identity(), Some((1, 7)));
        }
        assert_eq!(current_query_identity(), None);
        assert!(!has_active_cursor());
        recorder.seal();
        let rows = recorder.lock();
        assert_eq!(
            rows.rows.iter().map(|r| r.query).collect::<Vec<_>>(),
            [Some((1, 7, "outer")), Some((2, 7, "inner")), Some((1, 7, "outer"))]
        );
        assert_eq!(rows.rows.iter().map(|r| r.cpu.unwrap()).sum::<u64>(), 3);
        drop(rows);
        let text = json(&recorder);
        assert!(text.contains("\"schema\":2"));
        assert_eq!(text.matches("\"owner_kind\":\"query_execution\"").count(), 3);
        assert!(text.contains("\"records_complete\":true"));
    }
    #[test]
    fn legacy_timing_never_enters_query_ownership_or_samples_for_it() {
        let recorder = fake(Vec::new(), 10);
        assert!(enter_query(&recorder, 1, 1, "query", None).is_none());
        assert!(!has_active_cursor());
        assert!(current_query_identity().is_none());
        let text = json(&recorder);
        assert!(text.contains("\"schema\":1"));
        assert!(!text.contains("query_context"));
        assert!(!text.contains("query_job"));
        assert!(!text.contains("query_name"));
    }
    #[test]
    fn query_cpu_rejects_nonconstant_category_grammar_without_sampling() {
        let mut recorder = fake(Vec::new(), 10);
        Arc::get_mut(&mut recorder).unwrap().query_ownership = true;
        for name in [
            "",
            "1query",
            "path/to/key",
            "query\"injection",
            "non_ascii_é",
            concat!(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ),
        ] {
            assert!(enter_query(&recorder, 1, 1, name, None).is_none());
        }
        assert!(!has_active_cursor());
        assert!(current_query_identity().is_none());
        recorder.seal();
        let text = json(&recorder);
        assert!(text.contains("\"lost_segments\":6"));
        assert!(text.contains("\"records_complete\":false"));
    }
    #[test]
    fn exclusive_switches_do_not_double_count_or_bridge_gaps() {
        let recorder = fake(
            vec![
                sample(1, Some(10)),
                sample(3, Some(12)),
                sample(4, Some(13)),
                sample(8, Some(17)),
                sample(9, Some(18)),
                sample(11, Some(20)),
            ],
            10,
        );
        resume(Some(&recorder), Some(1));
        pause();
        resume(Some(&recorder), Some(2));
        pause();
        resume(Some(&recorder), Some(1));
        pause();
        recorder.seal();
        let buffer = recorder.lock();
        assert_eq!(buffer.rows.iter().map(|row| row.cpu.unwrap()).sum::<u64>(), 8);
        assert!(buffer.rows.windows(2).all(|rows| rows[0].end.after <= rows[1].start.before));
        drop(buffer);
        assert!(json(&recorder).contains("\"records_complete\":true"));
    }
    #[test]
    fn clock_failures_and_regression_are_null_not_zero() {
        let recorder = fake(
            vec![sample(1, None), sample(2, Some(4)), sample(3, Some(8)), sample(4, Some(7))],
            10,
        );
        resume(Some(&recorder), Some(1));
        pause();
        resume(Some(&recorder), None);
        pause();
        recorder.seal();
        let out = json(&recorder);
        assert!(out.contains("\"status\":\"clock_failed\""));
        assert!(out.contains("\"status\":\"clock_regressed\""));
        assert_eq!(out.matches("\"cpu_ns\":null").count(), 2);
        assert!(out.contains("\"records_complete\":false"));
    }
    #[test]
    fn independent_loss_and_open_cutoff_cannot_become_complete() {
        let recorder = fake(
            vec![sample(1, Some(1)), sample(2, Some(2)), sample(3, Some(3)), sample(4, Some(4))],
            1,
        );
        resume(Some(&recorder), Some(1));
        pause();
        resume(Some(&recorder), Some(1));
        recorder.seal();
        assert!(json(&recorder).contains("\"active_segments\":1"));
        pause();
        let out = json(&recorder);
        assert!(out.contains("\"expected_segments\":2"));
        assert!(out.contains("\"lost_segments\":1"));
        assert!(out.contains("\"active_segments\":1"));
        assert!(out.contains("\"records_complete\":false"));
    }
    #[test]
    fn invalid_wall_and_postseal_activity_never_claim_complete() {
        let recorder = fake(vec![sample(5, Some(10)), sample(4, Some(11))], 10);
        resume(Some(&recorder), Some(1));
        pause();
        recorder.seal();
        let out = json(&recorder);
        assert!(out.contains("\"end_ns\":null"));
        assert!(out.contains("\"status\":\"wall_overflow\""));
        assert!(out.contains("\"records_complete\":false"));
        // No fake clock samples remain: post-seal entry must not sample.
        resume(Some(&recorder), None);
        assert!(!has_active_cursor());
        assert!(json(&recorder).contains("\"lost_segments\":1"));
    }
    #[test]
    fn checked_segment_allocator_refuses_wraparound() {
        let recorder = fake(vec![sample(1, Some(1)), sample(2, Some(2))], 10);
        recorder.next.store(u64::MAX, Ordering::Relaxed);
        resume(Some(&recorder), None);
        pause();
        recorder.seal();
        let out = json(&recorder);
        assert!(out.contains("\"segment_allocator_high_water\":null"));
        assert!(out.contains("\"expected_segments\":null"));
        assert!(out.contains("\"identity_exhausted\":true"));
        assert!(out.contains("\"lost_segments\":1"));
    }
    #[test]
    fn process_thread_ids_are_unique_and_cpu_baselines_are_thread_local() {
        let first = thread_id().unwrap();
        let second = std::thread::spawn(|| {
            let id = thread_id().unwrap();
            assert_eq!(thread_id(), Some(id));
            id
        })
        .join()
        .unwrap();
        assert_ne!(first, second);
        assert_eq!(thread_id(), Some(first));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn real_thread_cpu_distinguishes_busy_work_from_sleep() {
        let before = thread_cpu().unwrap();
        let start = Instant::now();
        while thread_cpu().unwrap() - before < 5_000_000 {
            std::hint::black_box(7u64.wrapping_mul(13));
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
        }
        let busy = thread_cpu().unwrap() - before;
        let before = thread_cpu().unwrap();
        let sleep_start = Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(30));
        let sleeping = thread_cpu().unwrap() - before;
        assert!(busy >= 5_000_000);
        // Sleep may be delayed arbitrarily by scheduling, but should consume only
        // a small fraction of its own wall envelope, not the sleeping duration.
        assert!(u128::from(sleeping) * 2 < sleep_start.elapsed().as_nanos());
    }
}
