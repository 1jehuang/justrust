//! Opt-in bounded admission and registered-wait diagnostics, not a complete compiler graph.
//! Never call publication methods while holding query shard or latch locks.
use crate::query_task_bindings::{Sample, Source, TaskBindings};
#[cfg(not(test))]
use crate::query_task_bindings::{Sampler, Unbound};
use rustc_thread_pool::admission::{
    AdmissionAttachmentId, AdmissionBinding, AdmissionContext, AdmissionPhase, QualifiedWaitId,
    QueryRootNamespace,
};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Write as _;
use std::num::NonZeroU64;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(test))]
use rustc_data_structures::sync::AtomicU64 as JobAtomicU64;
#[cfg(test)]
use std::sync::atomic::AtomicU64 as JobAtomicU64;

#[cfg(not(test))]
use rustc_thread_pool::execution_observer::{ExecutionObserver, InterfaceOutcome};

const MAX_EVENTS: usize = 100_000;
const MAX_CONTEXTS: usize = 1024;
static FILE_ID: AtomicU64 = AtomicU64::new(1);
fn mint_capture_id() -> String {
    let id = FILE_ID.fetch_add(1, Ordering::Relaxed);
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{epoch}-{}-{id}", std::process::id())
}
#[cfg(not(test))]
#[derive(Default)]
struct PoolSampler(OnceLock<Arc<ExecutionObserver>>);
#[cfg(not(test))]
impl Sampler for PoolSampler {
    fn sample(&self) -> Sample {
        use rustc_thread_pool::execution_observer::PoolPointUnbound;
        let pool = self.0.get().ok_or(Unbound::PoolUnavailable)?;
        pool.current_point()
            .map(|p| crate::query_task_bindings::Point { task: p.task, at_pool_ns: p.at_pool_ns })
            .map_err(|reason| match reason {
                PoolPointUnbound::NoActiveTask => Unbound::NoActive,
                PoolPointUnbound::SuspendedTask => Unbound::Suspended,
                PoolPointUnbound::ForeignObserver => Unbound::ForeignObserver,
                PoolPointUnbound::ClockOverflow => Unbound::ClockOverflow,
            })
    }
    fn authority(&self) -> Option<(String, u64)> {
        self.0.get().map(|pool| (pool.capture_id().to_owned(), pool.observer_id()))
    }
}
#[cfg(not(test))]
fn new_bindings() -> (Arc<TaskBindings>, Arc<PoolSampler>) {
    let pool = Arc::new(PoolSampler::default());
    (Arc::new(TaskBindings::new(pool.clone())), pool)
}
#[cfg(test)]
fn new_bindings() -> (Arc<TaskBindings>, ()) {
    (Arc::new(TaskBindings::new(Arc::new(crate::query_task_bindings::UnavailableSampler))), ())
}

#[derive(Clone)]
pub struct QueryAdmissionObserver(Arc<Inner>);
struct Inner {
    // FD-relative publication stays private if the original path is replaced.
    _directory_handle: File,
    capture_id: Option<String>,
    query_root: Option<QueryRootNamespace>,
    admission: OnceLock<AdmissionBinding>,
    epoch: Instant,
    next_context: AtomicU64,
    next_wait: AtomicU64,
    wait_ids_exhausted: AtomicBool,
    #[cfg(test)]
    wait_id_ceiling: u64,
    reads: crate::query_reads::QueryReadObserver,
    returned: AtomicBool,
    bindings: Arc<TaskBindings>,
    #[cfg(not(test))]
    pool: Arc<PoolSampler>,
    data: Mutex<Data>,
}
#[derive(Default)]
struct Data {
    contexts: Vec<(u64, Option<u64>)>,
    events: Vec<Event>,
    lost: u64,
    waits: Vec<WaitRecord>,
}
struct WaitRecord {
    id: u64,
    context: u64,
    waitee: u64,
    consumer: Option<u64>,
    times: [u64; 4],
    reason: u64,
}

/// Facts only while compiler locks are held. Publication is explicit after resume.
/// Dropping this token never acquires the observer mutex, even during unwinding.
#[derive(Clone)]
pub struct QueryWaitToken(Arc<WaitFacts>);
struct WaitFacts {
    context: QueryAdmissionContext,
    id: u64,
    admission: Option<AdmissionBinding>,
    waitee: u64,
    consumer: Option<u64>,
    registered: u64,
    registered_point: Option<Sample>,
    notified: AtomicU64,
    reason: AtomicU64,
}
impl std::fmt::Debug for QueryWaitToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("QueryWaitToken").field(&self.0.id).finish()
    }
}
impl QueryWaitToken {
    pub fn admission_context(&self, phase: AdmissionPhase) -> Option<AdmissionContext> {
        let binding = self.0.admission?;
        Some(AdmissionContext {
            attachment_id: binding.attachment_id,
            qualified_wait_id: QualifiedWaitId {
                query_root: binding.query_root,
                wait_id: NonZeroU64::new(self.0.id)?,
            },
            phase,
        })
    }
    /// Called immediately before notification under the existing synchronization.
    pub fn notify(&self, reason: &'static str) {
        self.0.notified.store(self.0.context.now(), Ordering::Relaxed);
        self.0.reason.store(
            match reason {
                "completed" => 1,
                "poisoned" => 2,
                "cycle" => 3,
                _ => 0,
            },
            Ordering::Release,
        );
    }
    pub fn now(&self) -> u64 {
        self.0.context.now()
    }
    /// Only the waiting thread calls this, after pool allowance reacquisition.
    pub fn resumed(&self, condvar_returned: u64) {
        let bindings = &self.0.context.observer.0.bindings;
        let resumed_point = bindings.sample();
        bindings
            .record_sample(Source::WaitRegistered { wait_id: self.0.id }, self.0.registered_point);
        bindings.record_sample(Source::WaitResumed { wait_id: self.0.id }, resumed_point);
        let resumed = self.now();
        let reason = self.0.reason.load(Ordering::Acquire);
        let record = WaitRecord {
            id: self.0.id,
            context: self.0.context.id,
            waitee: self.0.waitee,
            consumer: self.0.consumer,
            times: [
                self.0.registered,
                self.0.notified.load(Ordering::Relaxed),
                condvar_returned,
                resumed,
            ],
            reason,
        };
        let mut data = self.0.context.observer.0.data();
        if data.waits.len() < MAX_EVENTS / 4 && data.waits.try_reserve(1).is_ok() {
            data.waits.push(record);
        } else {
            data.lost = data.lost.saturating_add(1);
        }
    }
}
struct Event {
    kind: &'static str,
    context: u64,
    job: u64,
    parent: Option<u64>,
    at: u64,
}
#[derive(Clone)]
pub struct QueryAdmissionContext {
    observer: QueryAdmissionObserver,
    id: u64,
}
pub struct QueryAdmissionJob {
    context: QueryAdmissionContext,
    id: u64,
    terminal: bool,
}

/// Non-generic allocator owner. Its Drop cannot tighten dropck for tcx arena references.
/// With observation disabled this is only an inline atomic and None, with no allocation.
pub struct QueryJobAllocator {
    counter: JobAtomicU64,
    observer: Option<QueryAdmissionContext>,
}

impl QueryJobAllocator {
    pub fn new(observer: Option<&QueryAdmissionObserver>) -> Self {
        let counter = JobAtomicU64::new(1);
        debug_assert_eq!(counter.load(Ordering::Relaxed), 1);
        Self { counter, observer: observer.map(QueryAdmissionObserver::context) }
    }

    pub fn observer(&self) -> Option<&QueryAdmissionContext> {
        self.observer.as_ref()
    }
}

impl std::ops::Deref for QueryJobAllocator {
    type Target = JobAtomicU64;

    fn deref(&self) -> &Self::Target {
        &self.counter
    }
}

impl Drop for QueryJobAllocator {
    fn drop(&mut self) {
        if let Some(observer) = &self.observer {
            observer.snapshot(self.counter.load(Ordering::Relaxed));
        }
    }
}

impl Inner {
    fn data(&self) -> MutexGuard<'_, Data> {
        match self.data.lock() {
            Ok(data) => data,
            Err(error) => {
                let mut data = error.into_inner();
                data.lost = data.lost.saturating_add(1);
                data
            }
        }
    }
}

impl QueryAdmissionObserver {
    #[cfg(test)]
    pub(crate) fn set_wait_id_ceiling_for_test(&mut self, ceiling: u64) {
        assert!(ceiling < u64::MAX);
        Arc::get_mut(&mut self.0).unwrap().wait_id_ceiling = ceiling;
    }
    /// Sample outside query, latch, and pool State locks. A conflict never replaces authority.
    pub fn bind_admission_attachment(
        &self,
        attachment: Option<AdmissionAttachmentId>,
    ) -> Option<AdmissionBinding> {
        let binding =
            AdmissionBinding { attachment_id: attachment?, query_root: self.0.query_root? };
        let actual = self.0.admission.get_or_init(|| binding);
        (*actual == binding).then_some(*actual)
    }

    pub fn admission_binding(&self) -> Option<AdmissionBinding> {
        self.0.admission.get().copied()
    }
    /// Called only from inside the actual frontend pool, never from session setup.
    /// Failure disables pool capture without changing compilation or query capture.
    pub fn attach_current_pool(&self) {
        #[cfg(not(test))]
        {
            if self.0.pool.0.get().is_some() {
                return;
            }
            let Some(observer) =
                ExecutionObserver::new_for_query(MAX_EVENTS, self.0.capture_id.as_deref().unwrap())
            else {
                return;
            };
            if rustc_thread_pool::Registry::current()
                .attach_execution_observer(Arc::clone(&observer))
            {
                let _ = self.0.pool.0.set(observer);
            }
        }
    }

    /// Requires a private directory. Unsupported platforms fail open by disabling capture.
    pub fn new(directory: &Path) -> Option<Self> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = directory;
            return None;
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            if let Err(error) = builder.create(directory) {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return None;
                }
            }
            if !fs::symlink_metadata(directory).ok()?.is_dir() {
                return None;
            }
            // Linux O_NOFOLLOW | O_DIRECTORY closes the leaf symlink/replacement race.
            let handle = fs::OpenOptions::new()
                .read(true)
                .custom_flags(131072 | 65536)
                .open(directory)
                .ok()?;
            let metadata = handle.metadata().ok()?;
            if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
                return None;
            }
            unsafe extern "C" {
                fn geteuid() -> u32;
            }
            // SAFETY: geteuid has no arguments or memory preconditions on Linux.
            let owner = unsafe { geteuid() };
            if metadata.uid() != owner {
                return None;
            }
            let capture_id = Some(mint_capture_id());
            let (bindings, _pool) = new_bindings();
            Some(Self(Arc::new(Inner {
                _directory_handle: handle,
                capture_id,
                query_root: QueryRootNamespace::mint(),
                admission: OnceLock::new(),
                epoch: Instant::now(),
                next_context: AtomicU64::new(1),
                next_wait: AtomicU64::new(1),
                wait_ids_exhausted: AtomicBool::new(false),
                #[cfg(test)]
                wait_id_ceiling: u64::MAX - 1,
                reads: crate::query_reads::QueryReadObserver::with_bindings(Arc::clone(&bindings)),
                bindings,
                returned: AtomicBool::new(false),
                #[cfg(not(test))]
                pool: _pool,
                data: Mutex::new(Data::default()),
            })))
        }
    }
    /// Uses only a validated owned private directory and a transport-assigned identity.
    /// The schema and query accounting remain identical to standalone observation.
    pub fn new_from_directory(directory: File, capture_id: String) -> Option<Self> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (directory, capture_id);
            None
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            unsafe extern "C" {
                fn geteuid() -> u32;
            }
            let metadata = directory.metadata().ok()?;
            if !metadata.is_dir()
                || metadata.mode() & 0o077 != 0
                || metadata.uid() != unsafe { geteuid() }
                || capture_id.is_empty()
                || capture_id.len() > 128
                || !capture_id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            {
                return None;
            }
            let (bindings, _pool) = new_bindings();
            Some(Self(Arc::new(Inner {
                _directory_handle: directory,
                capture_id: Some(capture_id),
                query_root: QueryRootNamespace::mint(),
                admission: OnceLock::new(),
                epoch: Instant::now(),
                next_context: AtomicU64::new(1),
                next_wait: AtomicU64::new(1),
                wait_ids_exhausted: AtomicBool::new(false),
                #[cfg(test)]
                wait_id_ceiling: u64::MAX - 1,
                reads: crate::query_reads::QueryReadObserver::with_bindings(Arc::clone(&bindings)),
                bindings,
                returned: AtomicBool::new(false),
                #[cfg(not(test))]
                pool: _pool,
                data: Mutex::new(Data::default()),
            })))
        }
    }
    pub fn context(&self) -> QueryAdmissionContext {
        let id = self.0.next_context.fetch_add(1, Ordering::Relaxed);
        self.0.bindings.allocator_id(id);
        let mut data = self.0.data();
        if data.contexts.len() < MAX_CONTEXTS && data.contexts.try_reserve(1).is_ok() {
            data.contexts.push((id, None));
        } else {
            data.lost = data.lost.saturating_add(1);
        }
        QueryAdmissionContext { observer: self.clone(), id }
    }
    pub fn reads(&self) -> &crate::query_reads::QueryReadObserver {
        &self.0.reads
    }
    /// This is an interface callback outcome, never a process/driver outcome.
    pub fn finalize_interface_returned(&self) {
        self.0.returned.store(true, Ordering::Relaxed);
    }
}
impl QueryAdmissionContext {
    pub fn admission_binding(&self) -> Option<AdmissionBinding> {
        self.observer.admission_binding()
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    fn now(&self) -> u64 {
        u64::try_from(self.observer.0.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
    /// Lock-free observer facts allocation, called only for an actual registration.
    pub fn register_wait(
        &self,
        waitee: u64,
        consumer: Option<u64>,
        binding: Option<AdmissionBinding>,
    ) -> Option<QueryWaitToken> {
        let inner = &self.observer.0;
        #[cfg(test)]
        let ceiling = inner.wait_id_ceiling;
        #[cfg(not(test))]
        let ceiling = u64::MAX - 1;
        let mut id = inner.next_wait.load(Ordering::Relaxed);
        loop {
            if id == 0 || id > ceiling {
                inner.wait_ids_exhausted.store(true, Ordering::Relaxed);
                return None;
            }
            match inner.next_wait.compare_exchange_weak(
                id,
                id + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => id = actual,
            }
        }
        self.observer.0.bindings.allocator_id(id);
        Some(QueryWaitToken(Arc::new(WaitFacts {
            context: self.clone(),
            id,
            admission: binding
                .filter(|binding| Some(*binding) == self.observer.admission_binding()),
            waitee,
            consumer,
            registered_point: self.observer.0.bindings.sample(),
            registered: self.now(),
            notified: AtomicU64::new(0),
            reason: AtomicU64::new(0),
        })))
    }
    /// Lock-free identity check. Numeric context IDs alone are not unique across roots.
    pub fn same_context(&self, other: &Self) -> bool {
        self.id == other.id && Arc::ptr_eq(&self.observer.0, &other.observer.0)
    }

    /// Publish only outside query shard/latch locks, including namespace mismatch loss.
    pub fn mark_loss(&self) {
        let mut data = self.observer.0.data();
        data.lost = data.lost.saturating_add(1);
    }

    pub fn admit(&self, id: u64, parent: Option<u64>, at: Instant) -> QueryAdmissionJob {
        self.observer.0.bindings.allocator_id(id);
        self.observer.0.bindings.record(Source::Admitted { context_id: self.id, job_id: id });
        self.event("admitted", id, parent, at);
        QueryAdmissionJob { context: self.clone(), id, terminal: false }
    }
    pub fn snapshot(&self, high_water: u64) {
        let mut data = self.observer.0.data();
        if let Some(context) = data.contexts.iter_mut().find(|c| c.0 == self.id) {
            context.1 = Some(high_water);
        }
    }
    fn event(&self, kind: &'static str, job: u64, parent: Option<u64>, at: Instant) {
        let elapsed = at.saturating_duration_since(self.observer.0.epoch).as_nanos();
        let at = u64::try_from(elapsed).unwrap_or(u64::MAX);
        let mut data = self.observer.0.data();
        if data.events.len() < MAX_EVENTS && data.events.try_reserve(1).is_ok() {
            data.events.push(Event { kind, context: self.id, job, parent, at });
        } else {
            data.lost = data.lost.saturating_add(1);
        }
    }
}
impl QueryAdmissionJob {
    /// Identifies the admitted job publishing this result, not provider execution.
    /// The cache's own critical section must have returned before calling this.
    pub fn result_produced(&self, index: u32) {
        self.context.observer.reads().result_produced(self.context.id, self.id, index);
    }
    pub fn complete(mut self) {
        self.terminal = true;
        self.context.event("completed", self.id, None, Instant::now());
    }
}
impl Drop for QueryAdmissionJob {
    fn drop(&mut self) {
        if !self.terminal {
            self.terminal = true;
            self.context.event("poisoned", self.id, None, Instant::now());
        }
    }
}
impl Data {
    fn waits_complete(&self, expected: u64) -> bool {
        if self.lost != 0 || self.waits.len() as u64 != expected {
            return false;
        }
        let mut ids = Vec::new();
        if ids.try_reserve(self.waits.len()).is_err() {
            return false;
        }
        let mut admissions = Vec::new();
        let mut terminals = Vec::new();
        if admissions.try_reserve(self.events.len()).is_err()
            || terminals.try_reserve(self.events.len()).is_err()
        {
            return false;
        }
        for event in &self.events {
            if event.kind == "admitted" {
                admissions.push((event.context, event.job, event.at));
            } else {
                terminals.push((event.context, event.job, event.kind));
            }
        }
        admissions.sort_unstable();
        terminals.sort_unstable();
        ids.extend(self.waits.iter().map(|wait| wait.id));
        ids.sort_unstable();
        ids.iter().enumerate().all(|(i, &id)| id == i as u64 + 1)
            && self.waits.iter().all(|wait| {
                let admitted = |job| {
                    admissions
                        .binary_search_by_key(&(wait.context, job), |&(context, job, _)| {
                            (context, job)
                        })
                        .ok()
                        .is_some_and(|index| admissions[index].2 <= wait.times[0])
                };
                admitted(wait.waitee)
                    && wait.consumer.is_none_or(admitted)
                    && wait.times.windows(2).all(|pair| pair[0] <= pair[1])
                    && matches!(wait.reason, 1..=3)
                    && (wait.reason == 3
                        || terminals
                            .binary_search(&(
                                wait.context,
                                wait.waitee,
                                if wait.reason == 1 { "completed" } else { "poisoned" },
                            ))
                            .is_ok())
            })
    }

    fn append_wait_json(&self, out: &mut String, high: u64, returned: bool) {
        let expected = high.saturating_sub(1);
        let _ = write!(
            out,
            ",\"wait_allocator_baseline\":1,\"wait_allocator_high_water\":{high},\"expected_waits\":{expected},\"wait_capture_complete\":{},\"wait_events\":[",
            returned && self.waits_complete(expected)
        );
        let mut first = true;
        for wait in &self.waits {
            for (stage, kind) in
                ["registered", "notified", "condvar_returned", "resumed"].iter().enumerate()
            {
                if !first {
                    out.push(',');
                }
                first = false;
                let consumer = wait.consumer.map_or_else(|| "null".into(), |id| id.to_string());
                let _ = write!(
                    out,
                    "{{\"kind\":\"{kind}\",\"wait_id\":{},\"context_id\":{},\"waitee_job_id\":{},\"consumer_job_id\":{consumer},\"at_ns\":{}",
                    wait.id, wait.context, wait.waitee, wait.times[stage]
                );
                if stage == 1 {
                    let reason = match wait.reason {
                        1 => "completed",
                        2 => "poisoned",
                        3 => "cycle",
                        _ => "unknown",
                    };
                    let _ = write!(out, ",\"reason\":\"{reason}\"");
                }
                out.push('}');
            }
        }
        out.push(']');
    }
    fn context_inventory_matches(&self, expected_contexts: u64) -> bool {
        // This count comes from the root allocator, never from the captured context list.
        // Removing an entire context and all of its events must still fail reconciliation.
        if self.contexts.len() as u64 != expected_contexts {
            return false;
        }
        let mut contexts = Vec::new();
        if contexts.try_reserve(self.contexts.len()).is_err() {
            return false;
        }
        contexts.extend(self.contexts.iter().map(|context| context.0));
        contexts.sort_unstable();
        contexts.iter().enumerate().all(|(index, &id)| id == index as u64 + 1)
    }

    fn complete(&self, expected_contexts: u64) -> bool {
        if self.lost != 0 || !self.context_inventory_matches(expected_contexts) {
            return false;
        }
        if self
            .events
            .iter()
            .any(|event| !matches!(event.kind, "admitted" | "completed" | "poisoned"))
        {
            return false;
        }
        self.contexts.iter().all(|&(context, high)| {
            let Some(high) = high else {
                return false;
            };
            if high < 1 {
                return false;
            }
            let admissions =
                self.events.iter().filter(|e| e.context == context && e.kind == "admitted").count()
                    as u64;
            if admissions != high - 1 {
                return false;
            }
            // Sorted IDs check uniqueness, contiguous allocator coverage, and one terminal each.
            let mut admitted = Vec::new();
            let mut terminal = Vec::new();
            if admitted.try_reserve(self.events.len()).is_err()
                || terminal.try_reserve(self.events.len()).is_err()
            {
                return false;
            }
            admitted.extend(
                self.events
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.context == context && e.kind == "admitted")
                    .map(|(index, e)| (e.job, e.at, e.parent, index)),
            );
            terminal.extend(
                self.events
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.context == context && e.kind != "admitted")
                    .map(|(index, e)| (e.job, e.at, index)),
            );
            admitted.sort_unstable();
            terminal.sort_unstable();
            admitted.len() == terminal.len()
                && admitted.iter().zip(&terminal).enumerate().all(|(i, (admission, terminal))| {
                    admission.0 == i as u64 + 1
                        && admission.0 == terminal.0
                        && terminal.1 >= admission.1
                        && terminal.2 > admission.3
                        && admission.2.is_none_or(|parent| parent >= 1 && parent < admission.0)
                        && admission.2.is_none_or(|parent| {
                            admitted
                                .get((parent - 1) as usize)
                                .is_some_and(|parent| parent.1 <= admission.1)
                        })
                })
        }) && self.events.iter().all(|e| self.contexts.iter().any(|c| c.0 == e.context))
    }
    fn json(&self, returned: bool, capture_id: &str, expected_contexts: u64) -> String {
        let mut out = format!(
            "{{\"schema\":4,\"capture_id\":\"{capture_id}\",\"producer\":\"rustc-query-admission\",\"scope\":\"query-job-admission-lifecycle\",\"full_pipeline_complete\":false,\"cpu_work_ns\":null,\"execution_span_ns\":null,\"interface_outcome\":\"{}\",\"capture_complete\":{},\"lost_events\":{},\"expected_contexts\":{expected_contexts},\"opaque_boundaries\":[\"fast-cache-hits\",\"race-cache-hits\",\"green-ensure-no-job\",\"feeds\",\"direct-paths\",\"nonquery\",\"resource-internals\",\"driver-and-process-outcome\",\"cargo-parent-binding\",\"cache-read-dependencies\",\"wait-and-resource-dependencies\"],\"contexts\":[",
            if returned { "returned" } else { "interrupted" },
            returned && self.complete(expected_contexts),
            self.lost
        );
        for (i, &(id, high)) in self.contexts.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            let high_text = high.map_or_else(|| "null".into(), |h| h.to_string());
            let expected = high
                .and_then(|h| h.checked_sub(1))
                .map_or_else(|| "null".into(), |h| h.to_string());
            let _ = write!(
                out,
                "{{\"context_id\":{id},\"allocator_baseline\":1,\"allocator_high_water\":{high_text},\"expected_jobs\":{expected}}}"
            );
        }
        out.push_str("],\"events\":[");
        for (i, event) in self.events.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"kind\":\"{}\",\"context_id\":{},\"job_id\":{},\"at_ns\":{}",
                event.kind, event.context, event.job, event.at
            );
            if event.kind == "admitted" {
                let parent = event.parent.map_or_else(|| "null".into(), |p| p.to_string());
                let _ = write!(out, ",\"parent_job_id\":{parent}");
            }
            out.push('}');
        }
        out.push_str("]}\n");
        out
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        let data = match self.data.get_mut() {
            Ok(data) => data,
            Err(error) => {
                let data = error.into_inner();
                data.lost = data.lost.saturating_add(1);
                data
            }
        };
        let name = self.capture_id.clone().unwrap();
        if self.wait_ids_exhausted.load(Ordering::Relaxed) {
            data.lost = data.lost.saturating_add(1);
        }
        // All context/job handles own this root Arc. Its final Drop observes a stable
        // independent allocator highwater, including contexts whose receipts were lost.
        let expected_contexts = self.next_context.load(Ordering::Relaxed).saturating_sub(1);
        if !data.context_inventory_matches(expected_contexts) {
            data.lost = data.lost.saturating_add(1);
        }
        let returned = self.returned.load(Ordering::Relaxed);
        let wait_high = self.next_wait.load(Ordering::Relaxed);
        let missing_waits = wait_high.saturating_sub(1).saturating_sub(data.waits.len() as u64);
        data.lost = data.lost.saturating_add(missing_waits);
        let mut json = data.json(returned, &name, expected_contexts);
        // Preserve admission completeness as its own scope, not full causal coverage.
        json.truncate(json.len().saturating_sub(2));
        data.append_wait_json(&mut json, wait_high, returned);
        self.reads.append_json(&mut json);
        let [requests, reads, results] = self.reads.high_waters();
        self.bindings.append_json(
            &mut json,
            &data.contexts,
            self.next_context.load(Ordering::Relaxed).checked_sub(1),
            [requests, reads, results, wait_high],
        );
        json.push_str("}\n");
        #[cfg(target_os = "linux")]
        let _ = publish(&self._directory_handle, &name, json.as_bytes());
        #[cfg(not(target_os = "linux"))]
        let _ = json;
        // Final root drop follows frontend return and pool teardown. Keep the pool
        // protocol separate: its clock and task IDs are not query bindings.
        #[cfg(all(not(test), target_os = "linux"))]
        if let Some(pool) = self.pool.0.get() {
            if returned {
                pool.seal(InterfaceOutcome::Returned);
            } else {
                pool.seal(InterfaceOutcome::Unwound);
            }
            let mut receipt = Vec::new();
            if pool.write_json(&mut receipt).is_ok() {
                let _ = publish(&self._directory_handle, &format!("{name}-pool"), &receipt);
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn publish(directory: &File, name: &str, bytes: &[u8]) -> bool {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::PermissionsExt;
    // Refuse publication if the directory was made public while capture ran.
    if !directory.metadata().is_ok_and(|m| m.permissions().mode() & 0o077 == 0) {
        return false;
    }
    let Ok(temp) = CString::new(format!(".{name}.tmp")) else {
        return false;
    };
    let Ok(receipt) = CString::new(format!("{name}.json")) else {
        return false;
    };
    unsafe extern "C" {
        fn openat(fd: i32, path: *const std::ffi::c_char, flags: i32, mode: u32) -> i32;
        fn linkat(
            oldfd: i32,
            old: *const std::ffi::c_char,
            newfd: i32,
            new: *const std::ffi::c_char,
            flags: i32,
        ) -> i32;
        fn unlinkat(fd: i32, path: *const std::ffi::c_char, flags: i32) -> i32;
    }
    let fd = directory.as_raw_fd();
    // SAFETY: valid held directory FD and NUL-terminated anonymous filename. Linux flags
    // O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC refuse existing files and symlinks.
    let output = unsafe { openat(fd, temp.as_ptr(), 1 | 64 | 128 | 524288, 0o600) };
    if output < 0 {
        return false;
    }
    // SAFETY: openat returned a newly owned FD, transferred exactly once to File.
    let mut file = unsafe { File::from_raw_fd(output) };
    let mut published = false;
    if file.write_all(bytes).is_ok() && file.sync_all().is_ok() {
        // SAFETY: both names remain valid for the call, FD is held. linkat is atomic
        // and cannot replace an existing destination, unlike rename.
        published = unsafe { linkat(fd, temp.as_ptr(), fd, receipt.as_ptr(), 0) } == 0;
        let _ = directory.sync_all();
    }
    // SAFETY: only our successfully created temporary anonymous file is unlinked.
    let _ = unsafe { unlinkat(fd, temp.as_ptr(), 0) };
    published
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn admission_binding_is_immutable_and_namespaces_are_not_capture_text() {
        let path = private_directory();
        let other_path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        fs::create_dir(&other_path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&other_path, fs::Permissions::from_mode(0o700)).unwrap();
        let other = QueryAdmissionObserver::new_from_directory(
            File::open(&other_path).unwrap(),
            observer.0.capture_id.clone().unwrap(),
        )
        .unwrap();
        assert_ne!(observer.0.query_root, other.0.query_root);
        assert_eq!(observer.admission_binding(), None);
        assert_eq!(observer.bind_admission_attachment(None), None);
        let attachment = AdmissionAttachmentId::mint().unwrap();
        let binding = observer.bind_admission_attachment(Some(attachment)).unwrap();
        assert_eq!(observer.clone().bind_admission_attachment(Some(attachment)), Some(binding));
        assert_eq!(
            observer.bind_admission_attachment(Some(AdmissionAttachmentId::mint().unwrap())),
            None
        );
        assert_eq!(observer.bind_admission_attachment(None), None);
        assert_eq!(observer.admission_binding(), Some(binding));
        let other_binding = other.bind_admission_attachment(Some(attachment)).unwrap();
        let first = observer.context().register_wait(1, None, Some(binding)).unwrap();
        let second = other.context().register_wait(1, None, Some(other_binding)).unwrap();
        let phase = AdmissionPhase::QueryWaitRelease;
        let a = first.admission_context(phase).unwrap();
        let b = second.admission_context(phase).unwrap();
        assert_eq!(a.qualified_wait_id.wait_id, b.qualified_wait_id.wait_id);
        assert_ne!(a.qualified_wait_id, b.qualified_wait_id);
        assert_eq!(
            first.admission_context(AdmissionPhase::QueryWaitReacquire).unwrap().qualified_wait_id,
            a.qualified_wait_id
        );
        let false_binding = observer.context().register_wait(1, None, Some(other_binding)).unwrap();
        assert_eq!(false_binding.admission_context(phase), None);
        let absent = observer.context().register_wait(1, None, None).unwrap();
        assert_eq!(absent.admission_context(phase), None);
        let false_attachment = observer
            .context()
            .register_wait(
                1,
                None,
                Some(AdmissionBinding {
                    query_root: binding.query_root,
                    attachment_id: AdmissionAttachmentId::mint().unwrap(),
                }),
            )
            .unwrap();
        assert_eq!(false_attachment.admission_context(phase), None);
        drop(false_attachment);
        drop((first, second, false_binding, absent, observer, other));
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(other_path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn injected_wait_ceiling_fails_open_without_locking_or_reusing_ids() {
        let path = private_directory();
        let mut observer = QueryAdmissionObserver::new(&path).unwrap();
        observer.set_wait_id_ceiling_for_test(1);
        let context = observer.context();
        let binding = observer.bind_admission_attachment(AdmissionAttachmentId::mint()).unwrap();
        let data = observer.0.data.lock().unwrap();
        let first = context.register_wait(1, None, Some(binding)).unwrap();
        assert_eq!(first.0.id, 1);
        for _ in 0..3 {
            assert!(context.register_wait(1, None, Some(binding)).is_none());
        }
        assert_eq!(observer.0.next_wait.load(Ordering::Relaxed), 2);
        assert!(observer.0.wait_ids_exhausted.load(Ordering::Relaxed));
        assert!(data.waits.is_empty());
        drop(data);
        drop(first);
        context.snapshot(1);
        observer.finalize_interface_returned();
        drop((context, observer));
        let text = receipt(&path);
        assert!(text.contains("\"capture_complete\":false"));
        assert!(text.contains("\"wait_capture_complete\":false"));
        assert!(text.contains("\"wait_allocator_high_water\":2"));
        assert!(!text.contains("query_root"));
        assert!(!text.contains("attachment_id"));
        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exhausted_namespace_leaves_diagnostic_waits_unqualified() {
        let path = private_directory();
        let mut observer = QueryAdmissionObserver::new(&path).unwrap();
        // Inject only into this root, never mutate the process-wide mint.
        Arc::get_mut(&mut observer.0).unwrap().query_root = None;
        assert_eq!(observer.bind_admission_attachment(AdmissionAttachmentId::mint()), None);
        assert_eq!(observer.admission_binding(), None);
        let context = observer.context();
        let wait = context.register_wait(1, None, None).unwrap();
        assert_eq!(wait.admission_context(AdmissionPhase::QueryWaitRelease), None);
        assert_eq!(wait.0.id, 1);
        drop((wait, context, observer));
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    fn private_directory() -> std::path::PathBuf {
        let id = FILE_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!("query-admission-test-{}-{id}", std::process::id()))
    }
    #[cfg(target_os = "linux")]
    fn receipt(directory: &Path) -> String {
        let files: Vec<_> = fs::read_dir(directory).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].extension().unwrap(), "json");
        let text = fs::read_to_string(&files[0]).unwrap();
        assert!(text.contains(&format!(
            "\"capture_id\":\"{}\"",
            files[0].file_stem().unwrap().to_str().unwrap()
        )));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777, 0o600);
        text
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn lifetime_terminal_once_and_root_outlives_session() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let root = observer.clone();
        let first = observer.context();
        let second = observer.context();
        first.admit(1, None, Instant::now()).complete();
        let parent = first.admit(2, Some(1), Instant::now());
        drop(parent);
        second.admit(1, None, Instant::now()).complete();
        first.snapshot(3);
        second.snapshot(2);
        drop(first);
        drop(second);
        drop(observer);
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        root.finalize_interface_returned();
        drop(root);
        let text = receipt(&path);
        assert!(text.contains("\"capture_complete\":true"));
        assert_eq!(text.matches("\"kind\":\"completed\"").count(), 2);
        assert_eq!(text.matches("\"kind\":\"poisoned\"").count(), 1);
        assert!(text.contains("\"context_id\":2"));
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn allocator_off_is_inline_and_retains_native_baseline() {
        let jobs = QueryJobAllocator::new(None);
        assert!(jobs.observer().is_none());
        assert_eq!(jobs.load(Ordering::Relaxed), 1);
        assert_eq!(jobs.fetch_add(1, Ordering::Relaxed), 1);
        assert_eq!(jobs.fetch_add(1, Ordering::Relaxed), 2);
        assert_eq!(jobs.load(Ordering::Relaxed), 3);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn allocator_drop_snapshots_highwater_without_generic_owner_drop() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let jobs = QueryJobAllocator::new(Some(&observer));
        let id = jobs.fetch_add(1, Ordering::Relaxed);
        jobs.observer().unwrap().admit(id, None, Instant::now()).complete();
        assert!(observer.0.data().contexts[0].1.is_none());
        drop(jobs);
        assert_eq!(observer.0.data().contexts[0].1, Some(2));
        observer.finalize_interface_returned();
        drop(observer);
        let text = receipt(&path);
        assert!(text.contains("\"allocator_high_water\":2"));
        assert!(text.contains("\"expected_jobs\":1"));
        assert!(text.contains("\"capture_complete\":true"));
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn context_identity_uses_context_id_and_root_identity() {
        let path = private_directory();
        let other_path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let other_observer = QueryAdmissionObserver::new(&other_path).unwrap();
        let context = observer.context();
        let same = context.clone();
        let foreign_context = observer.context();
        let foreign_root = other_observer.context();
        assert!(context.same_context(&context));
        assert!(context.same_context(&same));
        assert!(!context.same_context(&foreign_context));
        assert_eq!(context.id, foreign_root.id);
        assert!(!context.same_context(&foreign_root));
        context.admit(1, None, Instant::now()).complete();
        foreign_context.admit(1, None, Instant::now()).complete();
        foreign_root.admit(1, None, Instant::now()).complete();
        context.snapshot(2);
        foreign_context.snapshot(2);
        foreign_root.snapshot(2);
        drop(context);
        drop(same);
        drop(foreign_context);
        drop(foreign_root);
        observer.finalize_interface_returned();
        other_observer.finalize_interface_returned();
        drop(observer);
        drop(other_observer);
        assert!(receipt(&path).contains("\"capture_complete\":true"));
        assert!(receipt(&other_path).contains("\"capture_complete\":true"));
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(other_path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn foreign_parent_loss_avoids_fabricated_local_numeric_parent() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let context = observer.context();
        let foreign = observer.context();
        context.admit(1, None, Instant::now()).complete();
        foreign.admit(1, None, Instant::now()).complete();
        // A foreign job1 must not silently name the unrelated local job1 for local job2.
        let parent = if context.same_context(&foreign) {
            Some(1)
        } else {
            context.mark_loss();
            None
        };
        context.admit(2, parent, Instant::now()).complete();
        context.snapshot(3);
        foreign.snapshot(2);
        drop(context);
        drop(foreign);
        observer.finalize_interface_returned();
        drop(observer);
        let text = receipt(&path);
        assert!(text.contains("\"capture_complete\":false"));
        assert!(text.contains("\"lost_events\":1"));
        assert_eq!(text.matches("\"parent_job_id\":null").count(), 3);
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn unwind_is_interrupted_even_with_complete_jobs() {
        let path = private_directory();
        let result = std::panic::catch_unwind(|| {
            let observer = QueryAdmissionObserver::new(&path).unwrap();
            let context = observer.context();
            context.snapshot(2);
            let _job = context.admit(1, None, Instant::now());
            panic!("simulated query panic");
        });
        assert!(result.is_err());
        let text = receipt(&path);
        assert!(text.contains("\"capture_complete\":false"));
        assert!(text.contains("\"interface_outcome\":\"interrupted\""));
        assert_eq!(text.matches("\"kind\":\"poisoned\"").count(), 1);
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn publication_no_replace_and_path_replacement_stays_anchored() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        publish(&observer.0._directory_handle, "1-2-3", b"first");
        publish(&observer.0._directory_handle, "1-2-3", b"second");
        assert_eq!(fs::read(path.join("1-2-3.json")).unwrap(), b"first");
        let moved = path.with_extension("moved");
        fs::rename(&path, &moved).unwrap();
        fs::create_dir(&path).unwrap();
        observer.finalize_interface_returned();
        drop(observer);
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 2);
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(moved).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_public_and_symlink_directories_and_fails_open() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let path = private_directory();
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(QueryAdmissionObserver::new(&path).is_none());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let link = path.with_extension("link");
        symlink(&path, &link).unwrap();
        assert!(QueryAdmissionObserver::new(&link).is_none());
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        observer.finalize_interface_returned();
        drop(observer);
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        fs::remove_file(link).unwrap();
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn bounds_mark_capture_incomplete() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let context = observer.context();
        for id in 1..=(MAX_EVENTS as u64 / 2 + 1) {
            context.admit(id, None, Instant::now()).complete();
        }
        context.snapshot(MAX_EVENTS as u64 / 2 + 2);
        let data = observer.0.data.lock().unwrap();
        assert_eq!(data.events.len(), MAX_EVENTS);
        assert_eq!(data.lost, 2);
        assert!(!data.complete(1));
        drop(data);
        for _ in 1..=MAX_CONTEXTS {
            drop(observer.context());
        }
        assert_eq!(observer.0.data.lock().unwrap().contexts.len(), MAX_CONTEXTS);
        drop(context);
        drop(observer);
        assert!(receipt(&path).contains("\"capture_complete\":false"));
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn independent_allocator_detects_loss_and_duplicates() {
        let mut data = Data { contexts: vec![(1, Some(2))], ..Data::default() };
        assert!(!data.complete(1));
        data.events.push(Event { kind: "admitted", context: 1, job: 1, parent: None, at: 0 });
        data.events.push(Event { kind: "completed", context: 1, job: 1, parent: None, at: 1 });
        assert!(data.complete(1));
        data.events.push(Event { kind: "poisoned", context: 1, job: 1, parent: None, at: 2 });
        assert!(!data.complete(1));
    }
    #[test]
    fn independent_root_allocator_detects_deleted_and_invalid_contexts() {
        let mut data = Data { contexts: vec![(1, Some(1)), (2, Some(1))], ..Data::default() };
        assert!(data.complete(2));
        data.contexts.pop();
        assert!(!data.complete(2));
        data.contexts.push((1, Some(1)));
        assert!(!data.complete(2));
        data.contexts[1].0 = 3;
        assert!(!data.complete(2));
        data.contexts[1].0 = 0;
        assert!(!data.complete(2));
        data.contexts[1].0 = 2;
        data.contexts.swap(0, 1);
        assert!(data.complete(2));
        assert!(Data::default().complete(0));
        assert!(!Data::default().complete(1));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn deleting_whole_context_and_jobs_marks_root_loss() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let first = observer.context();
        let second = observer.context();
        first.admit(1, None, Instant::now()).complete();
        first.snapshot(2);
        second.admit(1, None, Instant::now()).complete();
        second.snapshot(2);
        {
            let mut data = observer.0.data();
            data.contexts.retain(|context| context.0 != second.id);
            data.events.retain(|event| event.context != second.id);
            assert!(!data.complete(2));
        }
        drop(first);
        drop(second);
        observer.finalize_interface_returned();
        drop(observer);
        let text = receipt(&path);
        assert!(text.contains("\"expected_contexts\":2"));
        assert!(text.contains("\"lost_events\":1"));
        assert!(text.contains("\"capture_complete\":false"));
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn parent_admission_cannot_follow_child_admission() {
        let mut data = Data {
            contexts: vec![(1, Some(3))],
            events: vec![
                Event { kind: "admitted", context: 1, job: 1, parent: None, at: 20 },
                Event { kind: "admitted", context: 1, job: 2, parent: Some(1), at: 10 },
                Event { kind: "completed", context: 1, job: 2, parent: None, at: 30 },
                Event { kind: "completed", context: 1, job: 1, parent: None, at: 40 },
            ],
            lost: 0,
            waits: Vec::new(),
        };
        assert!(!data.complete(1));
        data.events[1].at = 20;
        assert!(data.complete(1));
        data.events[1].at = 25;
        assert!(data.complete(1));
    }
    #[test]
    fn invalid_parent_time_kind_and_order_are_incomplete() {
        let mut data = Data {
            contexts: vec![(1, Some(2))],
            events: vec![
                Event { kind: "admitted", context: 1, job: 1, parent: None, at: 10 },
                Event { kind: "completed", context: 1, job: 1, parent: None, at: 11 },
            ],
            lost: 0,
            waits: Vec::new(),
        };
        assert!(data.complete(1));
        data.events[0].parent = Some(1);
        assert!(!data.complete(1));
        data.events[0].parent = Some(0);
        assert!(!data.complete(1));
        data.events[0].parent = None;
        data.events[1].at = 9;
        assert!(!data.complete(1));
        data.events[1].at = 11;
        data.events[1].kind = "invalid";
        assert!(!data.complete(1));
        data.events[1].kind = "completed";
        data.events.swap(0, 1);
        assert!(!data.complete(1));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn poisoned_observer_mutex_does_not_poison_compilation() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let result = std::panic::catch_unwind(|| {
            let _data = observer.0.data.lock().unwrap();
            panic!("simulated recorder failure");
        });
        assert!(result.is_err());
        let context = observer.context();
        context.admit(1, None, Instant::now()).complete();
        context.snapshot(2);
        drop(context);
        observer.finalize_interface_returned();
        drop(observer);
        assert!(receipt(&path).contains("\"capture_complete\":false"));
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(target_os = "linux")]
    fn wait_receipt(f: impl FnOnce(&QueryAdmissionContext)) -> String {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let context = observer.context();
        let waitee = context.admit(1, None, Instant::now());
        let consumer = context.admit(2, None, Instant::now());
        f(&context);
        waitee.result_produced(7);
        waitee.complete();
        consumer.complete();
        context.snapshot(3);
        drop(context);
        observer.finalize_interface_returned();
        drop(observer);
        let text = receipt(&path);
        fs::remove_dir_all(path).unwrap();
        text
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn standalone_capture_identity_exists_before_any_pool_attach_or_drop() {
        let path = private_directory();
        let observer = QueryAdmissionObserver::new(&path).unwrap();
        let id = observer.0.capture_id.clone().unwrap();
        assert!(!id.is_empty());
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        drop(observer);
        assert!(path.join(format!("{id}.json")).is_file());
        assert!(receipt(&path).contains(&format!("\"capture_id\":\"{id}\"")));
        fs::remove_dir_all(path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wait_registration_under_latch_does_not_lock_or_publish_records() {
        let text = wait_receipt(|context| {
            let data = context.observer.0.data.lock().unwrap();
            let wait = context.register_wait(1, Some(2), None).unwrap();
            assert!(data.waits.is_empty());
            drop(wait);
            assert!(data.waits.is_empty());
            drop(data);
        });
        let bindings = text.split("\"task_bindings\":").nth(1).unwrap();
        assert!(bindings.contains("\"records_complete\":false"));
        assert!(bindings.contains("\"expected_points\":5"));
        assert!(!bindings.contains("\"kind\":\"wait_registered\""));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn registered_wait_resume_before_terminal_is_valid() {
        let text = wait_receipt(|context| {
            let wait = context.register_wait(1, Some(2), None).unwrap();
            wait.notify("completed");
            wait.resumed(wait.now());
        });
        assert!(text.contains("\"schema\":4"));
        assert!(text.contains("\"wait_capture_complete\":true"));
        let bindings = text.split("\"task_bindings\":").nth(1).unwrap();
        assert!(bindings.contains("\"records_complete\":true"));
        assert!(bindings.contains("\"expected_points\":5"));
        assert!(bindings.contains("\"all_targets_bound\":false"));
        for kind in ["wait_registered", "wait_resumed"] {
            assert_eq!(bindings.matches(&format!("\"kind\":\"{kind}\"")).count(), 1);
        }
        for kind in ["registered", "notified", "condvar_returned", "resumed"] {
            assert_eq!(text.matches(&format!("\"kind\":\"{kind}\"")).count(), 1);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn registered_wait_abandonment_duplicate_and_foreign_endpoint_are_incomplete() {
        let text = wait_receipt(|context| {
            let _wait = context.register_wait(1, Some(2), None).unwrap();
        });
        assert!(text.contains("\"expected_waits\":1"));
        let bindings = text.split("\"task_bindings\":").nth(1).unwrap();
        assert!(bindings.contains("\"expected_points\":5"));
        assert!(bindings.contains("\"records_complete\":false"));
        assert!(bindings.contains("\"lost_events\":0"));
        assert!(!bindings.contains("\"kind\":\"wait_registered\""));
        assert!(text.contains("\"wait_capture_complete\":false"));
        assert!(text.contains("\"lost_events\":1"));
        let text = wait_receipt(|context| {
            let wait = context.register_wait(1, Some(2), None).unwrap();
            wait.notify("completed");
            let returned = wait.now();
            wait.resumed(returned);
            wait.resumed(returned);
        });
        assert!(text.contains("\"wait_capture_complete\":false"));
        let text = wait_receipt(|context| {
            let wait = context.register_wait(3, Some(2), None).unwrap();
            wait.notify("completed");
            wait.resumed(wait.now());
        });
        assert!(text.contains("\"wait_capture_complete\":false"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn registered_wait_cycle_and_wrong_terminal_reason() {
        let text = wait_receipt(|context| {
            let wait = context.register_wait(1, Some(2), None).unwrap();
            wait.notify("cycle");
            wait.resumed(wait.now());
        });
        assert!(text.contains("\"wait_capture_complete\":true"));
        assert!(text.contains("\"reason\":\"cycle\""));
        let text = wait_receipt(|context| {
            let wait = context.register_wait(1, Some(2), None).unwrap();
            wait.notify("poisoned");
            wait.resumed(wait.now());
        });
        assert!(text.contains("\"wait_capture_complete\":false"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn registered_wait_bound_marks_loss() {
        let text = wait_receipt(|context| {
            for _ in 0..25_001 {
                let wait = context.register_wait(1, Some(2), None).unwrap();
                wait.notify("completed");
                wait.resumed(wait.now());
            }
        });
        assert!(text.contains("\"expected_waits\":25001"));
        let bindings = text.split("\"task_bindings\":").nth(1).unwrap();
        assert!(bindings.contains("\"expected_points\":50005"));
        assert!(bindings.contains("\"records_complete\":true"));
        assert!(bindings.contains("\"lost_events\":0"));
        assert!(text.contains("\"wait_capture_complete\":false"));
        assert_eq!(text.matches("\"kind\":\"resumed\"").count(), 25_000);
    }

    #[test]
    fn missing_snapshot_and_empty_baseline() {
        let mut data = Data { contexts: vec![(1, None)], ..Data::default() };
        assert!(!data.complete(1));
        data.contexts[0].1 = Some(1);
        assert!(data.complete(1));
        assert!(data.json(false, "1-1", 1).contains("\"interface_outcome\":\"interrupted\""));
        data.lost = 1;
        assert!(!data.complete(1));
    }
}
