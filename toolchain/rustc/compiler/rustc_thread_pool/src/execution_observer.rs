//! Opt-in, bounded logical fork/join receipts, independent of rustc_session.
//! Only explicitly instrumented joins are covered. Dequeue order is not a causal
//! edge, and these receipts make no CPU, span, or full-pipeline coverage claim.

pub use crate::execution_timing::QueryExecutionGuard;
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::admission_resource::{
    AttachmentEndorsement, BindFailure, EndorsementGate, EndorsementRole, PoolEndorsementToken,
    PreparedDescriptor, RegistrationSnapshot,
};

const MAX_RECORDS: usize = 100_000;

static NEXT_OBSERVER: AtomicU64 = AtomicU64::new(1);
static NEXT_ENDORSEMENT: AtomicU64 = AtomicU64::new(1);

fn allocate_id(counter: &AtomicU64, limit: u64) -> Option<u64> {
    let mut id = counter.load(Ordering::Relaxed);
    loop {
        if id > limit {
            return None;
        }
        let next = id.checked_add(1)?;
        match counter.compare_exchange_weak(id, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Some(id),
            Err(current) => id = current,
        }
    }
}
type Active = Rc<RefCell<Option<TaskContext>>>;
thread_local! {
    static CURRENT: RefCell<Option<Active>> = const { RefCell::new(None) };
    // Independent of CURRENT: opaque dispatch and an actually empty slot both
    // report NoActiveTask, but only the latter can admit a new frontend root.
    static DISPATCH_MASKED: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskId {
    pub observer: u64,
    pub task: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolPoint {
    pub task: u64,
    pub at_pool_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolPointUnbound {
    NoActiveTask,
    SuspendedTask,
    ForeignObserver,
    ClockOverflow,
}

fn safe_capture_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Normal,
    Panic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Root { task: TaskId },
    Fork { parent: TaskId, child: TaskId, continuation: TaskId },
    Start { task: TaskId },
    Finish { task: TaskId, outcome: Outcome },
    Join { branch: TaskId, child: TaskId, continuation: TaskId },
    Resume { continuation: TaskId },
}

#[derive(Clone, Debug)]
pub struct Receipt {
    pub observer: u64,
    pub events: Vec<Event>,
    pub dropped_events: u64,
    pub full_pipeline: bool,
    pub sealed: bool,
    pub interface_outcome: InterfaceOutcome,
    pub task_allocator_high_water: u64,
    pub expected_tasks: u64,
    pub timestamps: Vec<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceOutcome {
    Returned,
    Unwound,
    Unknown,
}

struct Buffer {
    events: Vec<Event>,
    timestamps: Vec<u64>,
    dropped: u64,
    sealed: bool,
    outcome: InterfaceOutcome,
}

/// Create only when enabled, then attach to exactly one Registry. An observer
/// namespace is never reused, even if a Registry or job address is reused.
pub struct ExecutionObserver {
    id: u64,
    attached: AtomicBool,
    next_task: AtomicU64,
    capacity: usize,
    buffer: Mutex<Buffer>,
    epoch: Instant,
    capture_id: String,
    query_capture_id: Option<String>,
    resource_attachment: OnceLock<PoolEndorsementToken>,
    resource_registration: EndorsementGate,
    timing: Option<Arc<crate::execution_timing::Recorder>>,
}

impl ExecutionObserver {
    /// Identity exhaustion disables recording, never wraps or affects work.
    pub fn new(capacity: usize) -> Option<Arc<Self>> {
        Self::create(capacity, None, None)
    }

    /// Establish immutable, reciprocal capture linkage without inferring query context.
    pub fn new_for_query(capacity: usize, query_capture_id: &str) -> Option<Arc<Self>> {
        if !safe_capture_id(query_capture_id) {
            return None;
        }
        Self::create(capacity, Some(query_capture_id), None)
    }

    /// Programmatic opt-in. Both native clocks use the supplied query Instant.
    /// Legacy constructors retain their exact schema and do not sample CPU time.
    pub fn new_for_query_with_timing(
        capacity: usize,
        query_capture_id: &str,
        query_epoch: Instant,
    ) -> Option<Arc<Self>> {
        if !safe_capture_id(query_capture_id) || query_epoch > Instant::now() {
            return None;
        }
        Self::create(capacity, Some(query_capture_id), Some(query_epoch))
    }

    pub fn timing_enabled(&self) -> bool {
        self.timing.is_some()
    }

    /// Separate opt-in: retain legacy pool3/timing1 for existing timed captures.
    pub fn new_for_query_with_cpu_ownership(
        capacity: usize,
        query_capture_id: &str,
        query_epoch: Instant,
    ) -> Option<Arc<Self>> {
        let mut observer =
            Self::new_for_query_with_timing(capacity, query_capture_id, query_epoch)?;
        Arc::get_mut(&mut observer)?.timing = Some(
            crate::execution_timing::Recorder::new_with_query_ownership(query_epoch, capacity),
        );
        Some(observer)
    }

    pub fn enter_query_execution(
        &self,
        context: u64,
        job: u64,
        name: &'static str,
    ) -> Option<crate::execution_timing::QueryExecutionGuard> {
        let recorder = self.timing.as_ref()?;
        let task = current_context()
            .filter(|task| std::ptr::eq(task.observer.as_ref(), self))
            .map(|task| task.id.task);
        crate::execution_timing::enter_query(recorder, context, job, name, task)
    }

    fn create(
        capacity: usize,
        query_capture_id: Option<&str>,
        timing_epoch: Option<Instant>,
    ) -> Option<Arc<Self>> {
        let id = allocate_id(&NEXT_OBSERVER, u64::MAX)?;
        let capture_id = format!(
            "{}-{}-{id}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
        );
        if query_capture_id == Some(capture_id.as_str()) {
            return None;
        }
        let capacity = capacity.min(MAX_RECORDS);
        let mut events = Vec::new();
        let mut timestamps = Vec::new();
        events.try_reserve_exact(capacity).ok()?;
        timestamps.try_reserve_exact(capacity).ok()?;
        let epoch = timing_epoch.unwrap_or_else(Instant::now);
        Some(Arc::new(Self {
            id,
            attached: AtomicBool::new(false),
            next_task: AtomicU64::new(1),
            capacity,
            epoch,
            timing: timing_epoch.map(|_| crate::execution_timing::Recorder::new(epoch, capacity)),
            capture_id,
            query_capture_id: query_capture_id.map(str::to_owned),
            resource_attachment: OnceLock::new(),
            resource_registration: EndorsementGate::default(),
            buffer: Mutex::new(Buffer {
                events,
                timestamps,
                dropped: 0,
                sealed: false,
                outcome: InterfaceOutcome::Unknown,
            }),
        }))
    }

    pub fn capture_id(&self) -> &str {
        &self.capture_id
    }

    pub fn observer_id(&self) -> u64 {
        self.id
    }

    /// Register one independently validated pool endorsement. Call only from
    /// owner preparation outside native/query locks. Receipt bytes and native
    /// scheduling remain unaffected by failure, including repeated calls.
    pub fn register_resource_attachment(
        &self,
        descriptor: PreparedDescriptor,
    ) -> Result<PoolEndorsementToken, BindFailure> {
        self.resource_registration.begin_attempt()?;
        let result = self.register_resource_attachment_once(descriptor);
        if result.is_ok() {
            self.resource_registration.succeeded();
        } else {
            self.resource_registration.failed();
        }
        result
    }

    fn register_resource_attachment_once(
        &self,
        descriptor: PreparedDescriptor,
    ) -> Result<PoolEndorsementToken, BindFailure> {
        if descriptor.pool_capture_id().as_ref() != self.capture_id {
            return Err(BindFailure::CaptureMismatch);
        }
        if descriptor.pool_observer().get() != self.id {
            return Err(BindFailure::ObserverMismatch);
        }
        if self.query_capture_id.as_deref() != Some(descriptor.query_capture_id().as_ref()) {
            return Err(BindFailure::QueryCaptureMismatch);
        }
        if self.resource_attachment.get().is_some() {
            return Err(BindFailure::Conflict);
        }
        let id = allocate_id(&NEXT_ENDORSEMENT, u64::MAX).ok_or(BindFailure::IdentityExhausted)?;
        // Distinct purpose prefix plus process-local checked allocator. Neither
        // filenames nor pool capture strings are parsed to discover identity.
        let capture_id = Arc::from(format!(
            "pool-endorsement-{}-{}-{id}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
        ));
        let token = PoolEndorsementToken {
            endorsement: AttachmentEndorsement::new(EndorsementRole::Pool, capture_id, descriptor)?,
            gate: self.resource_registration.clone(),
        };
        self.resource_attachment.set(token.clone()).map_err(|_| BindFailure::Conflict)?;
        Ok(token)
    }

    pub fn resource_attachment_registration(&self) -> RegistrationSnapshot {
        self.resource_registration.snapshot()
    }

    /// Serialize only this owner's immutable validated snapshot, if present.
    pub fn write_resource_attachment_json(
        &self,
        out: &mut impl std::io::Write,
    ) -> std::io::Result<bool> {
        if !self.resource_registration.is_complete() {
            return Ok(false);
        }
        let Some(token) = self.resource_attachment.get() else { return Ok(false) };
        token.write_json(out)?;
        Ok(true)
    }

    /// Borrow the worker's active slot without cloning contexts or locking receipts.
    pub fn current_point(self: &Arc<Self>) -> Result<PoolPoint, PoolPointUnbound> {
        CURRENT.with(|current| {
            let current = current.borrow();
            let active = current.as_ref().ok_or(PoolPointUnbound::NoActiveTask)?;
            let active = active.borrow();
            let task = active.as_ref().ok_or(PoolPointUnbound::SuspendedTask)?;
            if !Arc::ptr_eq(self, &task.observer) {
                return Err(PoolPointUnbound::ForeignObserver);
            }
            let at_pool_ns = u64::try_from(self.epoch.elapsed().as_nanos())
                .map_err(|_| PoolPointUnbound::ClockOverflow)?;
            Ok(PoolPoint { task: task.id.task, at_pool_ns })
        })
    }

    fn event(&self, event: Event) {
        let mut buffer = self.receipt_buffer();
        if !buffer.sealed
            && buffer.events.len() < self.capacity
            && buffer.events.try_reserve(1).is_ok()
            && buffer.timestamps.try_reserve(1).is_ok()
        {
            buffer.timestamps.push(self.epoch.elapsed().as_nanos().min(u64::MAX as u128) as u64);
            buffer.events.push(event);
        } else {
            buffer.dropped = buffer.dropped.saturating_add(1);
        }
    }

    fn task(self: &Arc<Self>) -> Option<TaskContext> {
        let task = allocate_id(&self.next_task, MAX_RECORDS as u64);
        let Some(task) = task else {
            self.loss();
            return None;
        };
        Some(TaskContext {
            observer: Arc::clone(self),
            id: TaskId { observer: self.id, task },
            state: Arc::new(Mutex::new(TaskState::Created)),
            completion: Arc::new(Mutex::new(None)),
        })
    }

    pub fn root(self: &Arc<Self>) -> Option<TaskContext> {
        let task = self.task()?;
        self.event(Event::Root { task: task.id });
        Some(task)
    }

    pub fn snapshot(&self) -> Receipt {
        let buffer = self.receipt_buffer();
        let high_water = self.next_task.load(Ordering::Relaxed);
        Receipt {
            observer: self.id,
            events: buffer.events.clone(),
            dropped_events: buffer.dropped,
            full_pipeline: false,
            sealed: buffer.sealed,
            interface_outcome: buffer.outcome,
            timestamps: buffer.timestamps.clone(),
            task_allocator_high_water: high_water,
            expected_tasks: high_water - 1,
        }
    }

    fn loss(&self) {
        let mut buffer = self.receipt_buffer();
        buffer.dropped = buffer.dropped.saturating_add(1);
    }

    fn receipt_buffer(&self) -> std::sync::MutexGuard<'_, Buffer> {
        self.buffer.lock().unwrap_or_else(|error| {
            let mut buffer = error.into_inner();
            // Poison can interrupt event/timestamp publication. Never authorize
            // bindings from a receipt whose buffer integrity is uncertain.
            buffer.dropped = buffer.dropped.saturating_add(1);
            buffer
        })
    }

    /// Seal only after the root interface and pool work finish. Never implicit.
    pub fn seal(&self, outcome: InterfaceOutcome) {
        let mut buffer = self.receipt_buffer();
        if !buffer.sealed {
            buffer.outcome = outcome;
            buffer.sealed = true;
        }
        drop(buffer);
        if let Some(timing) = &self.timing {
            timing.seal();
        }
    }

    /// Separate receipt protocol, intentionally not a query-admission schema.
    /// IDs in events are task-local numbers qualified by the header observer.
    pub fn write_json(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        let receipt = self.snapshot();
        let schema = if self.timing.as_ref().is_some_and(|t| t.query_ownership_enabled()) {
            4
        } else if self.timing.is_some() {
            3
        } else if self.query_capture_id.is_some() {
            2
        } else {
            1
        };
        let reciprocal = self
            .query_capture_id
            .as_ref()
            .map_or_else(String::new, |id| format!(",\"query_capture_id\":\"{id}\""));
        write!(
            out,
            "{{\"schema\":{schema},\"producer\":\"rustc-thread-pool-join\",\"scope\":\"native-join-continuations\",\"capture_id\":\"{}\"{reciprocal},\"observer\":{},\"full_pipeline_complete\":false,\"cpu_work_ns\":null,\"execution_span_ns\":null,\"scope_coverage\":false,\"scheduler_coverage\":false,\"pre_attach_coverage\":false,\"sealed\":{},\"interface_outcome\":\"{}\",\"task_allocator_baseline\":1,\"task_allocator_high_water\":{},\"expected_tasks\":{},\"dropped_events\":{},\"events\":[",
            self.capture_id,
            receipt.observer,
            receipt.sealed,
            match receipt.interface_outcome {
                InterfaceOutcome::Returned => "returned",
                InterfaceOutcome::Unwound => "unwound",
                InterfaceOutcome::Unknown => "unknown",
            },
            receipt.task_allocator_high_water,
            receipt.expected_tasks,
            receipt.dropped_events
        )?;
        for (index, event) in receipt.events.iter().enumerate() {
            if index != 0 {
                write!(out, ",")?;
            }
            let at_ns = receipt.timestamps[index];
            match event {
                Event::Root { task } => {
                    write!(out, "{{\"kind\":\"root\",\"task\":{},\"at_ns\":{at_ns}}}", task.task)?
                }
                Event::Fork { parent, child, continuation } => write!(
                    out,
                    "{{\"kind\":\"fork\",\"parent\":{},\"child\":{},\"continuation\":{},\"at_ns\":{at_ns}}}",
                    parent.task, child.task, continuation.task
                )?,
                Event::Start { task } => {
                    write!(out, "{{\"kind\":\"start\",\"task\":{},\"at_ns\":{at_ns}}}", task.task)?
                }
                Event::Finish { task, outcome } => write!(
                    out,
                    "{{\"kind\":\"finish\",\"task\":{},\"outcome\":\"{}\",\"at_ns\":{at_ns}}}",
                    task.task,
                    match outcome {
                        Outcome::Normal => "normal",
                        Outcome::Panic => "panic",
                    }
                )?,
                Event::Join { branch, child, continuation } => write!(
                    out,
                    "{{\"kind\":\"join\",\"branch\":{},\"child\":{},\"continuation\":{},\"at_ns\":{at_ns}}}",
                    branch.task, child.task, continuation.task
                )?,
                Event::Resume { continuation } => write!(
                    out,
                    "{{\"kind\":\"resume\",\"continuation\":{},\"at_ns\":{at_ns}}}",
                    continuation.task
                )?,
            }
        }
        write!(out, "]")?;
        if let Some(timing) = &self.timing {
            timing.write_json(
                out,
                self.query_capture_id.as_deref().unwrap_or(""),
                &self.capture_id,
                self.id,
            )?;
        }
        writeln!(out, "}}")
    }

    pub(crate) fn claim_registry(&self) -> bool {
        self.attached.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed).is_ok()
    }

    pub fn owns(&self, task: &TaskContext) -> bool {
        self.id == task.id.observer
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskState {
    Created,
    Running,
    Finished,
}

#[derive(Clone)]
pub struct TaskContext {
    observer: Arc<ExecutionObserver>,
    id: TaskId,
    state: Arc<Mutex<TaskState>>,
    // The terminal segment of this logical invocation, set only by its guard.
    completion: Arc<Mutex<Option<TaskId>>>,
}

pub fn current_task() -> Option<TaskId> {
    current_context().map(|task| task.id)
}

pub fn current_context() -> Option<TaskContext> {
    CURRENT.with(|current| current.borrow().as_ref().and_then(|active| active.borrow().clone()))
}

/// Do not attribute an unobserved/legacy receiving query to an outer observed
/// query merely because it executes synchronously on the same worker.
pub fn suspend_query_execution() -> Option<QueryExecutionGuard> {
    crate::execution_timing::suspend_query_execution()
}

/// Whether generic dispatch currently masks the worker's lineage. Instrumented
/// closures may enter their own tasks, but frontend entry must not invent a root
/// for opaque scheduler work. This does not amend point receipt outcomes.
pub fn is_dispatch_masked() -> bool {
    DISPATCH_MASKED.with(Cell::get)
}

/// Generic dispatch is not a lineage edge. Hide the helping invocation's exact
/// active slot until the dispatched job returns. Instrumented closures enter
/// their own contexts, while opaque work stays unbound. Never finish or replace
/// the suspended slot's contents, including an in-progress fork's None state.
pub(crate) fn suspend_for_dispatch() -> DispatchGuard {
    let timing = crate::execution_timing::pause();
    let query = crate::execution_timing::suspend_query();
    let previous = CURRENT.with(|current| current.replace(None));
    let previous_mask = DISPATCH_MASKED.with(|masked| masked.replace(true));
    crate::execution_timing::resume(timing.as_ref(), None);
    DispatchGuard { previous, previous_mask, timing, query, _not_send: PhantomData }
}

/// Worker-local restoration, including when a dispatched closure unwinds.
pub(crate) struct DispatchGuard {
    previous: Option<Active>,
    previous_mask: bool,
    timing: Option<Arc<crate::execution_timing::Recorder>>,
    query: Option<crate::execution_timing::QueryOwnership>,
    _not_send: PhantomData<Rc<()>>,
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        crate::execution_timing::pause();
        CURRENT.with(|current| current.replace(self.previous.take()));
        DISPATCH_MASKED.with(|masked| masked.set(self.previous_mask));
        crate::execution_timing::restore_query(self.query.take());
        resume_current_timing(self.timing.as_ref());
    }
}

pub(crate) fn resume_current_timing(fallback: Option<&Arc<crate::execution_timing::Recorder>>) {
    if let Some(recorder) = crate::execution_timing::current_query_recorder() {
        let task = current_context()
            .filter(|context| {
                context
                    .observer
                    .timing
                    .as_ref()
                    .is_some_and(|timing| Arc::ptr_eq(timing, &recorder))
            })
            .map(|context| context.id.task);
        crate::execution_timing::resume(Some(&recorder), task);
    } else if let Some(context) = current_context() {
        crate::execution_timing::resume(context.observer.timing.as_ref(), Some(context.id.task));
    } else {
        crate::execution_timing::resume(fallback, None);
    }
}

impl TaskContext {
    pub fn id(&self) -> TaskId {
        self.id
    }

    fn start(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if *state != TaskState::Created {
            return false;
        }
        *state = TaskState::Running;
        self.observer.event(Event::Start { task: self.id });
        true
    }

    fn finish(&self, outcome: Outcome) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if *state != TaskState::Running {
            return;
        }
        self.observer.event(Event::Finish { task: self.id, outcome });
        *state = TaskState::Finished;
    }

    /// Never execute user work conditionally on recording success.
    pub fn enter(&self) -> Option<TaskGuard> {
        let timing = crate::execution_timing::pause();
        if !self.start() {
            resume_current_timing(timing.as_ref());
            return None;
        }
        let query = crate::execution_timing::suspend_query();
        let active = Rc::new(RefCell::new(Some(self.clone())));
        let previous = CURRENT.with(|current| current.replace(Some(Rc::clone(&active))));
        crate::execution_timing::resume(self.observer.timing.as_ref(), Some(self.id.task));
        Some(TaskGuard {
            active,
            previous,
            timing,
            query,
            outcome: Outcome::Normal,
            _not_send: PhantomData,
        })
    }

    /// Ends the current segment, creates parallel children, and reserves a
    /// distinct continuation. There is deliberately no child-to-sibling edge.
    /// Restrict fanout to the receipt capacity so instrumentation stays bounded.
    pub fn fork(&self, count: usize) -> Option<Fork> {
        if count != 2 || count > self.observer.capacity {
            self.observer.loss();
            return None;
        }
        let active = CURRENT.with(|current| current.borrow().clone())?;
        if active.borrow().as_ref().map(|task| task.id) != Some(self.id) {
            return None;
        }
        let mut children = Vec::new();
        if children.try_reserve_exact(count).is_err() {
            self.observer.loss();
            return None;
        }
        for _ in 0..count {
            children.push(self.observer.task()?);
        }
        let mut continuation = self.observer.task()?;
        continuation.completion = Arc::clone(&self.completion);
        crate::execution_timing::pause();
        let query = crate::execution_timing::suspend_query();
        for child in &children {
            self.observer.event(Event::Fork {
                parent: self.id,
                child: child.id,
                continuation: continuation.id,
            });
        }
        self.finish(Outcome::Normal);
        active.replace(None);
        crate::execution_timing::resume(self.observer.timing.as_ref(), None);
        Some(Fork {
            observer: Arc::clone(&self.observer),
            children,
            continuation,
            origin: active,
            query,
        })
    }
}

/// Not Send: restores the executing worker's context, including after unwind.
pub struct TaskGuard {
    active: Active,
    previous: Option<Active>,
    timing: Option<Arc<crate::execution_timing::Recorder>>,
    query: Option<crate::execution_timing::QueryOwnership>,
    outcome: Outcome,
    _not_send: PhantomData<Rc<()>>,
}

impl TaskGuard {
    /// Use if a catch_unwind inside the guarded region consumed the unwind.
    pub fn panicked(&mut self) {
        self.outcome = Outcome::Panic;
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        crate::execution_timing::pause();
        let outcome = if std::thread::panicking() { Outcome::Panic } else { self.outcome };
        if let Some(task) = self.active.borrow_mut().take() {
            task.finish(outcome);
            *task.completion.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.id);
        }
        CURRENT.with(|current| {
            current.replace(self.previous.take());
        });
        crate::execution_timing::restore_query(self.query.take());
        resume_current_timing(self.timing.as_ref());
    }
}

pub struct Fork {
    observer: Arc<ExecutionObserver>,
    children: Vec<TaskContext>,
    continuation: TaskContext,
    origin: Active,
    query: Option<crate::execution_timing::QueryOwnership>,
}

impl Fork {
    pub fn children(&self) -> &[TaskContext] {
        &self.children
    }

    /// Resume in the original suspended guard after the scheduler's actual
    /// join. An early call or an unrelated active context creates no edges.
    pub fn resume(mut self) -> Result<TaskId, Self> {
        let active = CURRENT.with(|current| current.borrow().clone());
        let Some(active) = active else { return Err(self) };
        if !Rc::ptr_eq(&active, &self.origin)
            || active.borrow().is_some()
            || self
                .children
                .iter()
                .any(|child| child.completion.lock().unwrap_or_else(|e| e.into_inner()).is_none())
        {
            return Err(self);
        }
        crate::execution_timing::pause();
        for child in &self.children {
            let terminal = *child.completion.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(terminal) = terminal {
                self.observer.event(Event::Join {
                    branch: child.id,
                    child: terminal,
                    continuation: self.continuation.id,
                });
            }
        }
        self.observer.event(Event::Resume { continuation: self.continuation.id });
        self.continuation.start();
        let id = self.continuation.id;
        active.replace(Some(self.continuation));
        crate::execution_timing::restore_query(self.query.take());
        resume_current_timing(self.observer.timing.as_ref());
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor_for(
        _observer: &ExecutionObserver,
        query: &str,
        pool: &str,
        observer_id: u64,
    ) -> PreparedDescriptor {
        PreparedDescriptor::new(
            Arc::from("attachment-test"),
            crate::admission::AdmissionAttachmentId::mint().unwrap(),
            Arc::from("controller-test"),
            Arc::from("resource-test"),
            Arc::from(query),
            crate::admission::QueryRootNamespace::mint().unwrap(),
            Arc::from(pool),
            std::num::NonZeroU64::new(observer_id).unwrap(),
        )
        .unwrap()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn query_cpu_masks_dispatch_forks_and_foreign_tasks_without_pool_ancestry() {
        use crate::execution_timing::current_query_identity;
        let observer =
            ExecutionObserver::new_for_query_with_cpu_ownership(100, "query-owner", Instant::now())
                .unwrap();
        let root = observer.root().unwrap();
        let root_guard = root.enter().unwrap();
        let query = observer.enter_query_execution(1, 5, "test_query").unwrap();
        assert_eq!(current_query_identity(), Some((1, 5)));
        {
            let _dispatch = suspend_for_dispatch();
            assert_eq!(current_query_identity(), None);
            let _inner = observer.enter_query_execution(1, 6, "test_query").unwrap();
            assert_eq!(current_query_identity(), Some((1, 6)));
            assert_eq!(current_task(), None);
        }
        assert_eq!(current_query_identity(), Some((1, 5)));
        {
            let foreign = ExecutionObserver::new(10).unwrap();
            let _guard = foreign.root().unwrap().enter().unwrap();
            assert_eq!(current_query_identity(), None);
        }
        assert_eq!(current_query_identity(), Some((1, 5)));
        let fork = root.fork(2).unwrap();
        assert_eq!(current_query_identity(), None);
        for child in fork.children() {
            let _guard = child.enter().unwrap();
            assert_eq!(current_query_identity(), None);
        }
        let continuation = fork.resume().ok().unwrap();
        assert_eq!(current_query_identity(), Some((1, 5)));
        assert_eq!(current_task(), Some(continuation));
        drop(query);
        assert_eq!(current_query_identity(), None);
        drop(root_guard);
        assert!(!crate::execution_timing::has_active_cursor());
        observer.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        observer.write_json(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.starts_with("{\"schema\":4,"));
        assert!(out.contains("\"timing\":{\"schema\":2,"));
        assert!(out.contains("\"query_context\":1,\"query_job\":5"));
        assert!(out.contains("\"task\":null,\"owner_kind\":\"query_execution\""));
        assert!(out.contains("\"query_context\":null,\"query_job\":null"));
        assert!(out.contains("\"records_complete\":true"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn query_cpu_real_scope_help_and_unobserved_registry_do_not_inherit_query() {
        use crate::execution_timing::current_query_identity;
        let source = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let target = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let observer = ExecutionObserver::new_for_query_with_cpu_ownership(
            100,
            "query-real-owner",
            Instant::now(),
        )
        .unwrap();
        source.install(|| {
            assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
            let root = observer.root().unwrap();
            let guard = root.enter().unwrap();
            let query = observer.enter_query_execution(1, 9, "test_query").unwrap();
            crate::scope(|scope| {
                scope.spawn(|_| {
                    assert!(is_dispatch_masked());
                    assert_eq!(current_query_identity(), None);
                    let _inner = observer.enter_query_execution(1, 10, "test_query").unwrap();
                    assert_eq!(current_query_identity(), Some((1, 10)));
                });
            });
            assert_eq!(current_query_identity(), Some((1, 9)));
            target.install(|| {
                assert_eq!(current_query_identity(), None);
                assert!(crate::Registry::current().execution_observer().is_none());
            });
            assert_eq!(current_query_identity(), Some((1, 9)));
            drop(query);
            drop(guard);
        });
        observer.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        observer.write_json(&mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("\"records_complete\":true"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn query_cpu_real_join_children_have_only_their_explicit_execution_owners() {
        use crate::execution_timing::current_query_identity;
        let pool = crate::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        let observer = ExecutionObserver::new_for_query_with_cpu_ownership(
            100,
            "query-join-owner",
            Instant::now(),
        )
        .unwrap();
        pool.install(|| {
            assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
            let root = observer.root().unwrap();
            let guard = root.enter().unwrap();
            let query = observer.enter_query_execution(1, 20, "test_query").unwrap();
            let barrier = std::sync::Barrier::new(2);
            crate::join(
                || {
                    assert_eq!(current_query_identity(), None);
                    let _child = observer.enter_query_execution(1, 21, "test_query").unwrap();
                    barrier.wait();
                    assert_eq!(current_query_identity(), Some((1, 21)));
                },
                || {
                    assert_eq!(current_query_identity(), None);
                    let _child = observer.enter_query_execution(1, 22, "test_query").unwrap();
                    barrier.wait();
                    assert_eq!(current_query_identity(), Some((1, 22)));
                },
            );
            assert_eq!(current_query_identity(), Some((1, 20)));
            assert_ne!(current_task(), Some(root.id()));
            drop(query);
            drop(guard);
        });
        observer.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        observer.write_json(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("\"query_job\":21"));
        assert!(out.contains("\"query_job\":22"));
        assert!(out.contains("\"records_complete\":true"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn query_cpu_foreign_receiver_retains_its_own_recorder_not_the_pool_tls() {
        use crate::execution_timing::current_query_identity;
        let source = ExecutionObserver::new_for_query_with_cpu_ownership(
            100,
            "query-source-owner",
            Instant::now(),
        )
        .unwrap();
        let receiver = ExecutionObserver::new_for_query_with_cpu_ownership(
            100,
            "query-receiver-owner",
            Instant::now(),
        )
        .unwrap();
        let root = source.root().unwrap();
        let root_guard = root.enter().unwrap();
        let source_query = source.enter_query_execution(1, 30, "test_query").unwrap();
        {
            let _receiver_query = receiver.enter_query_execution(2, 30, "test_query").unwrap();
            {
                let _nested = receiver.enter_query_execution(2, 31, "test_query").unwrap();
            }
            assert_eq!(current_query_identity(), Some((2, 30)));
            assert!(Arc::ptr_eq(
                &crate::execution_timing::current_query_recorder().unwrap(),
                receiver.timing.as_ref().unwrap()
            ));
            let fork = root.fork(2).unwrap();
            for child in fork.children() {
                drop(child.enter().unwrap());
            }
            fork.resume().ok().unwrap();
            assert_eq!(current_query_identity(), Some((2, 30)));
        }
        assert_eq!(current_query_identity(), Some((1, 30)));
        drop(source_query);
        drop(root_guard);
        receiver.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        receiver.write_json(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        // Four actual receiver segments: before/inside/after nested query,
        // and after the foreign pool's fork continuation. TLS identity alone
        // would miss resuming the wrong recorder for the fourth interval.
        assert_eq!(out.matches("\"task\":null,\"owner_kind\":\"query_execution\"").count(), 4);
        assert!(out.contains("\"records_complete\":true"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn query_cpu_absent_and_legacy_receivers_mask_outer_query_but_keep_real_task() {
        use crate::execution_timing::current_query_identity;
        let source = ExecutionObserver::new_for_query_with_cpu_ownership(
            100,
            "query-unknown-receiver",
            Instant::now(),
        )
        .unwrap();
        let legacy = ExecutionObserver::new_for_query_with_timing(
            100,
            "query-legacy-receiver",
            Instant::now(),
        )
        .unwrap();
        let root = source.root().unwrap();
        let root_guard = root.enter().unwrap();
        let query = source.enter_query_execution(1, 40, "outer").unwrap();
        for receiver in [None, Some(&legacy)] {
            {
                let _unobserved = receiver
                    .and_then(|receiver| receiver.enter_query_execution(2, 40, "unobserved"))
                    .or_else(suspend_query_execution)
                    .unwrap();
                assert_eq!(current_query_identity(), None);
                assert_eq!(current_task(), Some(root.id()));
            }
            assert_eq!(current_query_identity(), Some((1, 40)));
        }
        drop(query);
        drop(root_guard);
        source.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        source.write_json(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out.matches("\"owner_kind\":\"query_execution\"").count(), 3);
        assert_eq!(out.matches("\"query_name\":\"outer\"").count(), 3);
        assert!(!out.contains("\"query_name\":\"unobserved\""));
        assert!(out.contains("\"records_complete\":true"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn timed_guards_restore_exclusive_cursor_across_foreign_dispatch_and_fork() {
        let observer =
            ExecutionObserver::new_for_query_with_timing(100, "query-timed", Instant::now())
                .unwrap();
        let root = observer.root().unwrap();
        let root_guard = root.enter().unwrap();
        assert!(crate::execution_timing::has_active_cursor());
        {
            let foreign = ExecutionObserver::new(10).unwrap();
            let foreign_guard = foreign.root().unwrap().enter().unwrap();
            assert!(!crate::execution_timing::has_active_cursor());
            drop(foreign_guard);
        }
        assert!(crate::execution_timing::has_active_cursor());
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _dispatch = suspend_for_dispatch();
            assert!(current_context().is_none());
            assert!(crate::execution_timing::has_active_cursor());
            panic!("restore timed context");
        }));
        assert!(unwind.is_err());
        assert_eq!(current_task(), Some(root.id()));
        let fork = root.fork(2).unwrap();
        for child in fork.children() {
            drop(child.enter().unwrap());
        }
        let continuation = fork.resume().ok().unwrap();
        assert_eq!(current_task(), Some(continuation));
        drop(root_guard);
        assert!(!crate::execution_timing::has_active_cursor());
        observer.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        observer.write_json(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("\"schema\":3"));
        assert!(out.contains("\"owner_kind\":\"opaque\""));
        assert!(out.contains("\"records_complete\":true"));
        assert!(out.contains("\"pool_epoch_in_query_ns\":0"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn timed_pool_restores_source_after_unobserved_install_and_nested_timed_task() {
        let source = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let target = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let observer =
            ExecutionObserver::new_for_query_with_timing(100, "query-source", Instant::now())
                .unwrap();
        source.install(|| {
            assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
            let root = observer.root().unwrap();
            let guard = root.enter().unwrap();
            target.install(|| {
                assert!(crate::Registry::current().execution_observer().is_none());
                assert!(current_task().is_none());
            });
            assert_eq!(current_task(), Some(root.id()));
            let nested =
                ExecutionObserver::new_for_query_with_timing(20, "query-nested", Instant::now())
                    .unwrap();
            let nested_guard = nested.root().unwrap().enter().unwrap();
            drop(nested_guard);
            assert_eq!(current_task(), Some(root.id()));
            nested.seal(InterfaceOutcome::Returned);
            drop(guard);
            assert!(!crate::execution_timing::has_active_cursor());
        });
        observer.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        observer.write_json(&mut out).unwrap();
        assert!(String::from_utf8(out).unwrap().contains("\"records_complete\":true"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn timing_cursor_alone_masks_real_unobserved_registry_dispatch() {
        let pool = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let observer =
            ExecutionObserver::new_for_query_with_timing(100, "query-cursor-only", Instant::now())
                .unwrap();
        pool.install(|| {
            assert!(crate::Registry::current().execution_observer().is_none());
            let root = observer.root().unwrap();
            let guard = root.enter().unwrap();
            crate::scope(|scope| {
                scope.spawn(|_| {
                    assert!(is_dispatch_masked());
                    assert!(current_task().is_none());
                    assert!(crate::execution_timing::has_active_cursor());
                });
            });
            assert_eq!(current_task(), Some(root.id()));
            drop(guard);
            assert!(!crate::execution_timing::has_active_cursor());
        });
        observer.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        observer.write_json(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("\"owner_kind\":\"opaque\""));
        assert!(out.contains("\"records_complete\":true"));
    }

    #[test]
    fn resource_attachment_validates_own_exact_identity_and_preserves_pool_bytes() {
        let observer = ExecutionObserver::new_for_query(20, "query-test").unwrap();
        let mut before = Vec::new();
        observer.write_json(&mut before).unwrap();
        let mut absent = Vec::new();
        assert!(!observer.write_resource_attachment_json(&mut absent).unwrap());
        assert!(absent.is_empty());
        for (query, pool, id, failure) in [
            ("query-test", "wrong-pool", observer.id, BindFailure::CaptureMismatch),
            ("query-test", observer.capture_id(), observer.id + 1, BindFailure::ObserverMismatch),
            ("wrong-query", observer.capture_id(), observer.id, BindFailure::QueryCaptureMismatch),
        ] {
            let invalid = ExecutionObserver::new_for_query(0, "query-test").unwrap();
            let own_pool = if pool == observer.capture_id() { invalid.capture_id() } else { pool };
            let own_id =
                if failure == BindFailure::ObserverMismatch { invalid.id + 1 } else { invalid.id };
            let descriptor = descriptor_for(&invalid, query, own_pool, own_id);
            assert_eq!(invalid.register_resource_attachment(descriptor).unwrap_err(), failure);
            assert_eq!(invalid.resource_attachment_registration().failures, 1);
            let _ = id;
        }
        let descriptor =
            descriptor_for(&observer, "query-test", observer.capture_id(), observer.id);
        let token = observer.register_resource_attachment(descriptor.clone()).unwrap();
        assert_eq!(token.descriptor(), &descriptor);
        assert!(Arc::ptr_eq(token.descriptor().controller_id(), descriptor.controller_id()));
        let mut output = Vec::new();
        assert!(observer.write_resource_attachment_json(&mut output).unwrap());
        let expected = format!(
            "{{\"schema\":1,\"producer\":\"rustc-frontend-admission-attachment\",\"scope\":\"selected-resource-query-pool-attachment\",\"capture_id\":\"{}\",\"role\":\"pool\",\"sealed\":true,\"expected_endorsements\":1,\"endorsement_allocator_baseline\":0,\"endorsement_allocator_high_water\":1,\"lost_events\":0,\"loss_overflowed\":false,\"attachment\":{{\"attachment_capture_id\":\"attachment-test\",\"attachment_id\":{},\"controller_id\":\"controller-test\",\"resource_capture_id\":\"resource-test\",\"query_capture_id\":\"query-test\",\"query_root_namespace\":{},\"pool_capture_id\":\"{}\",\"pool_observer\":{}}}}}\n",
            token.capture_id(),
            descriptor.attachment_id().wire_value(),
            descriptor.query_root_namespace().wire_value(),
            observer.capture_id(),
            observer.id,
        );
        assert_eq!(String::from_utf8(output).unwrap(), expected);
        let mut after = Vec::new();
        observer.write_json(&mut after).unwrap();
        assert_eq!(before, after);
        assert_eq!(
            observer.register_resource_attachment(descriptor.clone()).unwrap_err(),
            BindFailure::Conflict
        );
        let mut rejected = Vec::new();
        assert!(token.write_json(&mut rejected).is_err());
        assert!(!observer.write_resource_attachment_json(&mut rejected).unwrap());
        assert!(rejected.is_empty());
        assert_eq!(observer.resource_attachment_registration().attempts, 2);
        assert_eq!(observer.resource_attachment_registration().successes, 1);
        assert_eq!(observer.resource_attachment_registration().conflicts, 1);
        // A diagnostic failure cannot prevent real pool task execution.
        let root = observer.root().unwrap();
        drop(root.enter().unwrap());
        assert!(
            observer
                .snapshot()
                .events
                .contains(&Event::Finish { task: root.id(), outcome: Outcome::Normal })
        );
    }

    #[test]
    fn concurrent_resource_attachment_has_exactly_one_owner_snapshot() {
        let observer = ExecutionObserver::new_for_query(0, "query-test").unwrap();
        let descriptor =
            descriptor_for(&observer, "query-test", observer.capture_id(), observer.id);
        let winners = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let descriptor = descriptor.clone();
                    let observer = &observer;
                    scope.spawn(move || observer.register_resource_attachment(descriptor))
                })
                .collect();
            handles.into_iter().map(|handle| handle.join().unwrap()).filter(Result::is_ok).count()
        });
        assert_eq!(winners, 1);
        assert_eq!(observer.resource_attachment.get().unwrap().descriptor(), &descriptor);
        assert_eq!(observer.resource_attachment_registration().attempts, 8);
        assert_eq!(observer.resource_attachment_registration().successes, 1);
        assert_eq!(observer.resource_attachment_registration().conflicts, 7);
        let mut output = Vec::new();
        assert!(observer.resource_attachment.get().unwrap().write_json(&mut output).is_err());
        assert!(output.is_empty());
    }

    #[test]
    fn legacy_pool_cannot_endorse_an_unobserved_query_capture() {
        let observer = ExecutionObserver::new(0).unwrap();
        let descriptor =
            descriptor_for(&observer, "query-test", observer.capture_id(), observer.id);
        assert_eq!(
            observer.register_resource_attachment(descriptor).unwrap_err(),
            BindFailure::QueryCaptureMismatch
        );
        assert!(observer.resource_attachment.get().is_none());
    }

    #[test]
    fn poisoned_buffer_permanently_reports_loss() {
        let observer = ExecutionObserver::new_for_query(20, "query-poison").unwrap();
        let root = observer.root().unwrap();
        drop(root.enter());
        assert_eq!(observer.snapshot().dropped_events, 0);
        let poisoned = std::panic::catch_unwind(|| {
            let _buffer = observer.buffer.lock().unwrap();
            panic!("interrupted receipt publication");
        });
        assert!(poisoned.is_err());
        observer.seal(InterfaceOutcome::Returned);
        let first = observer.snapshot();
        assert!(first.sealed);
        assert!(first.dropped_events > 0);
        assert!(observer.snapshot().dropped_events >= first.dropped_events);
        let mut output = Vec::new();
        observer.write_json(&mut output).unwrap();
        assert!(!String::from_utf8(output).unwrap().contains("\"dropped_events\":0"));
    }

    #[test]
    fn dispatch_mask_nesting_and_unwind_restore_exact_worker_state() {
        let observer = ExecutionObserver::new_for_query(100, "query-mask").unwrap();
        assert!(!is_dispatch_masked());
        let root = observer.root().unwrap();
        let guard = root.enter().unwrap();
        let before = observer.current_point().unwrap().task;
        {
            let _outer = suspend_for_dispatch();
            assert!(is_dispatch_masked());
            assert_eq!(observer.current_point(), Err(PoolPointUnbound::NoActiveTask));
            {
                let _inner = suspend_for_dispatch();
                assert!(is_dispatch_masked());
                assert!(current_context().is_none());
            }
            assert!(is_dispatch_masked());
            let panicked = std::panic::catch_unwind(|| {
                let _inner = suspend_for_dispatch();
                assert!(is_dispatch_masked());
                panic!("nested dispatch unwind");
            });
            assert!(panicked.is_err());
            assert!(is_dispatch_masked());
            assert_eq!(observer.current_point(), Err(PoolPointUnbound::NoActiveTask));
        }
        assert!(!is_dispatch_masked());
        assert_eq!(observer.current_point().unwrap().task, before);
        let fork = root.fork(2).unwrap();
        assert_eq!(observer.current_point(), Err(PoolPointUnbound::SuspendedTask));
        let panicked = std::panic::catch_unwind(|| {
            let _dispatch = suspend_for_dispatch();
            assert!(is_dispatch_masked());
            assert_eq!(observer.current_point(), Err(PoolPointUnbound::NoActiveTask));
            panic!("suspended dispatch unwind");
        });
        assert!(panicked.is_err());
        assert!(!is_dispatch_masked());
        assert_eq!(observer.current_point(), Err(PoolPointUnbound::SuspendedTask));
        for child in fork.children() {
            let _child = child.enter().unwrap();
        }
        let continuation = fork.resume().ok().unwrap();
        assert_eq!(observer.current_point().unwrap().task, continuation.task);
        drop(guard);
        assert!(!is_dispatch_masked());
        assert_eq!(observer.current_point(), Err(PoolPointUnbound::NoActiveTask));
    }

    #[test]
    fn dispatch_mask_does_not_prevent_explicit_instrumented_child_entry() {
        let observer = ExecutionObserver::new_for_query(100, "query-child-mask").unwrap();
        let root = observer.root().unwrap();
        let root_guard = root.enter().unwrap();
        let fork = root.fork(2).unwrap();
        {
            let _dispatch = suspend_for_dispatch();
            let before = observer.snapshot().task_allocator_high_water;
            assert!(is_dispatch_masked());
            assert_eq!(observer.current_point(), Err(PoolPointUnbound::NoActiveTask));
            for child in fork.children() {
                let child_guard = child.enter().unwrap();
                assert!(is_dispatch_masked());
                assert_eq!(observer.current_point().unwrap().task, child.id().task);
                drop(child_guard);
                assert!(is_dispatch_masked());
                assert_eq!(observer.current_point(), Err(PoolPointUnbound::NoActiveTask));
            }
            assert_eq!(observer.snapshot().task_allocator_high_water, before);
        }
        assert!(!is_dispatch_masked());
        assert_eq!(observer.current_point(), Err(PoolPointUnbound::SuspendedTask));
        assert!(fork.resume().is_ok());
        drop(root_guard);
    }

    #[test]
    fn points_bind_root_children_and_continuation_without_locks_or_clones() {
        let observer = ExecutionObserver::new_for_query(100, "query-1").unwrap();
        let foreign = ExecutionObserver::new_for_query(100, "query-1").unwrap();
        assert_ne!(observer.observer_id(), foreign.observer_id());
        assert_ne!(observer.capture_id(), foreign.capture_id());
        assert_eq!(observer.current_point(), Err(PoolPointUnbound::NoActiveTask));
        let root = observer.root().unwrap();
        let guard = root.enter().unwrap();
        let references = Arc::strong_count(&observer);
        // Holding both locks demonstrates that point sampling takes neither.
        let buffer = observer.buffer.lock().unwrap();
        let state = root.state.lock().unwrap();
        let first = observer.current_point().unwrap();
        assert_eq!(first.task, root.id().task);
        assert!(observer.current_point().unwrap().at_pool_ns >= first.at_pool_ns);
        assert_eq!(Arc::strong_count(&observer), references);
        assert_eq!(foreign.current_point(), Err(PoolPointUnbound::ForeignObserver));
        drop(state);
        drop(buffer);
        {
            let _dispatch = suspend_for_dispatch();
            assert_eq!(observer.current_point(), Err(PoolPointUnbound::NoActiveTask));
        }
        assert_eq!(observer.current_point().unwrap().task, root.id().task);
        let fork = root.fork(2).unwrap();
        assert_eq!(observer.current_point(), Err(PoolPointUnbound::SuspendedTask));
        for child in fork.children() {
            let _child = child.enter().unwrap();
            assert_eq!(observer.current_point().unwrap().task, child.id().task);
        }
        let continuation = fork.resume().ok().unwrap();
        assert_eq!(observer.current_point().unwrap().task, continuation.task);
        assert_ne!(continuation.task, root.id().task);
        drop(guard);
        assert_eq!(observer.current_point(), Err(PoolPointUnbound::NoActiveTask));
    }

    #[test]
    fn reciprocal_header_and_exact_legacy_header() {
        let legacy = ExecutionObserver::new(0).unwrap();
        let mut output = Vec::new();
        legacy.write_json(&mut output).unwrap();
        let legacy_json = String::from_utf8(output).unwrap();
        let expected = format!(
            "{{\"schema\":1,\"producer\":\"rustc-thread-pool-join\",\"scope\":\"native-join-continuations\",\"capture_id\":\"{}\",\"observer\":{},\"full_pipeline_complete\":false,\"cpu_work_ns\":null,\"execution_span_ns\":null,\"scope_coverage\":false,\"scheduler_coverage\":false,\"pre_attach_coverage\":false,\"sealed\":false,\"interface_outcome\":\"unknown\",\"task_allocator_baseline\":1,\"task_allocator_high_water\":1,\"expected_tasks\":0,\"dropped_events\":0,\"events\":[]}}\n",
            legacy.capture_id(),
            legacy.observer_id()
        );
        assert_eq!(legacy_json, expected);
        let linked = ExecutionObserver::new_for_query(0, "Query_123-abc").unwrap();
        assert!(safe_capture_id(linked.capture_id()));
        assert_ne!(linked.capture_id(), "Query_123-abc");
        let mut output = Vec::new();
        linked.write_json(&mut output).unwrap();
        let linked_json = String::from_utf8(output).unwrap();
        let expected = expected
            .replace("\"schema\":1", "\"schema\":2")
            .replace(legacy.capture_id(), linked.capture_id())
            .replace(
                &format!("\"observer\":{}", legacy.observer_id()),
                &format!(
                    "\"query_capture_id\":\"Query_123-abc\",\"observer\":{}",
                    linked.observer_id()
                ),
            );
        assert_eq!(linked_json, expected);
    }

    #[test]
    fn reciprocal_ids_are_bounded_safe_ascii() {
        for id in ["", "a b", "a/b", "a.b", "a\"b", "a\\b", "é", "\n"] {
            assert!(ExecutionObserver::new_for_query(0, id).is_none(), "{id:?}");
        }
        assert!(ExecutionObserver::new_for_query(0, &"a".repeat(129)).is_none());
        for id in ["a", "-_", &"A".repeat(128)] {
            assert!(ExecutionObserver::new_for_query(0, id).is_some());
        }
    }

    #[test]
    fn allocator_is_bounded_and_never_wraps() {
        let counter = AtomicU64::new(1);
        assert_eq!(allocate_id(&counter, 2), Some(1));
        assert_eq!(allocate_id(&counter, 2), Some(2));
        assert_eq!(allocate_id(&counter, 2), None);
        assert_eq!(counter.load(Ordering::Relaxed), 3);
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_id(&counter, u64::MAX), Some(u64::MAX - 1));
        assert_eq!(allocate_id(&counter, u64::MAX), None);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn fork_has_no_sibling_edge_and_distinct_continuation() {
        let observer = ExecutionObserver::new(100).unwrap();
        let root = observer.root().unwrap();
        let guard = root.enter().unwrap();
        let fork = root.fork(2).unwrap();
        let first = fork.children()[0].clone();
        let second = fork.children()[1].clone();
        let fork = fork.resume().err().unwrap();
        let inline = first.enter().unwrap();
        assert_eq!(current_task(), Some(first.id()));
        drop(inline);
        assert_eq!(current_task(), None);
        let second_id = second.id();
        std::thread::spawn(move || {
            let _guard = second.enter().unwrap();
        })
        .join()
        .unwrap();
        let continuation = fork.resume().ok().unwrap();
        assert_ne!(continuation, root.id());
        assert_ne!(continuation, first.id());
        assert_eq!(current_task(), Some(continuation));
        drop(guard);
        assert_eq!(current_task(), None);
        let receipt = observer.snapshot();
        assert!(!receipt.full_pipeline);
        assert!(!receipt.events.iter().any(|event| matches!(event,
            Event::Join { child, continuation, .. } if *child == first.id() && *continuation == second_id)));
        assert_eq!(receipt.events.iter().filter(|e| matches!(e, Event::Join { .. })).count(), 2);
        assert!(
            receipt
                .events
                .contains(&Event::Finish { task: continuation, outcome: Outcome::Normal })
        );
    }

    #[test]
    fn unwind_records_panic_and_restores_context() {
        let observer = ExecutionObserver::new(20).unwrap();
        let task = observer.root().unwrap();
        let result = std::panic::catch_unwind(|| {
            let _guard = task.enter().unwrap();
            panic!("user panic");
        });
        assert!(result.is_err());
        assert_eq!(current_task(), None);
        assert!(
            observer
                .snapshot()
                .events
                .contains(&Event::Finish { task: task.id(), outcome: Outcome::Panic })
        );
        assert!(task.enter().is_none());
    }

    #[test]
    fn native_inline_join_and_disabled_semantics() {
        let pool = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        pool.install(|| {
            assert_eq!(crate::join(current_task, current_task), (None, None));
        });
        let observer = ExecutionObserver::new(100).unwrap();
        pool.install(|| {
            assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
            let (a, b) = crate::join_context(
                |context| {
                    assert!(!context.migrated());
                    current_task().unwrap()
                },
                |context| {
                    assert!(!context.migrated());
                    current_task().unwrap()
                },
            );
            assert_ne!(a, b);
            assert_eq!(current_task(), None);
        });
        let receipt = observer.snapshot();
        assert_eq!(receipt.events.iter().filter(|e| matches!(e, Event::Fork { .. })).count(), 2);
        assert_eq!(receipt.events.iter().filter(|e| matches!(e, Event::Join { .. })).count(), 2);
        assert_eq!(receipt.events.iter().filter(|e| matches!(e, Event::Finish { .. })).count(), 4);
    }

    #[test]
    fn native_queued_join_has_real_child_context() {
        let pool = crate::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        let observer = ExecutionObserver::new(100).unwrap();
        pool.install(|| {
            assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
            let (send, recv) = std::sync::mpsc::channel();
            let (a, b) = crate::join_context(
                move |_| {
                    recv.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
                    current_task().unwrap()
                },
                move |context| {
                    assert!(context.migrated());
                    send.send(()).unwrap();
                    current_task().unwrap()
                },
            );
            assert_ne!(a, b);
            assert_eq!(a.observer, b.observer);
        });
        assert_eq!(
            observer.snapshot().events.iter().filter(|e| matches!(e, Event::Join { .. })).count(),
            2
        );
    }

    #[test]
    fn native_nested_join_waits_for_terminal_continuation() {
        let pool = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let observer = ExecutionObserver::new(100).unwrap();
        let (terminal, sibling) = pool.install(|| {
            assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
            crate::join(
                || {
                    crate::join(|| (), || ());
                    current_task().unwrap()
                },
                || current_task().unwrap(),
            )
        });
        let receipt = observer.snapshot();
        let outer_continuation = receipt
            .events
            .iter()
            .find_map(|event| match event {
                Event::Join { child, continuation, .. } if *child == sibling => Some(*continuation),
                _ => None,
            })
            .unwrap();
        assert!(receipt.events.iter().any(|event| matches!(event, Event::Join { child, continuation, .. } if *child == terminal && *continuation == outer_continuation)));
        assert_eq!(receipt.events.iter().filter(|e| matches!(e, Event::Join { .. })).count(), 4);
    }

    #[test]
    fn native_panic_preserves_both_closures_and_payload() {
        for threads in [1, 2] {
            let pool = crate::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            let observer = ExecutionObserver::new(100).unwrap();
            let ran_b = std::sync::atomic::AtomicBool::new(false);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pool.install(|| {
                    assert!(
                        crate::Registry::current().attach_execution_observer(Arc::clone(&observer))
                    );
                    crate::join(
                        || panic!("branch-a"),
                        || {
                            ran_b.store(true, Ordering::Relaxed);
                            panic!("branch-b");
                        },
                    );
                })
            }));
            assert!(ran_b.load(Ordering::Relaxed));
            assert_eq!(*result.unwrap_err().downcast::<&str>().unwrap(), "branch-a");
            let receipt = observer.snapshot();
            assert_eq!(
                receipt
                    .events
                    .iter()
                    .filter(|e| matches!(e, Event::Finish { outcome: Outcome::Panic, .. }))
                    .count(),
                3
            );
            assert_eq!(
                receipt.events.iter().filter(|e| matches!(e, Event::Join { .. })).count(),
                2
            );
        }
    }

    #[test]
    fn native_pool_attachment_rejects_namespace_reuse() {
        let a = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let b = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let observer = ExecutionObserver::new(100).unwrap();
        a.install(|| {
            assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)))
        });
        b.install(|| {
            assert!(!crate::Registry::current().attach_execution_observer(Arc::clone(&observer)))
        });
        b.install(|| assert_eq!(crate::join(current_task, current_task), (None, None)));
    }

    #[test]
    fn explicit_seal_and_independent_inventory() {
        let observer = ExecutionObserver::new(0).unwrap();
        let root = observer.root().unwrap();
        drop(root.enter());
        let receipt = observer.snapshot();
        assert!(!receipt.sealed);
        assert_eq!(receipt.interface_outcome, InterfaceOutcome::Unknown);
        assert_eq!(receipt.task_allocator_high_water, 2);
        assert_eq!(receipt.expected_tasks, 1);
        assert_eq!(receipt.dropped_events, 3);
        observer.seal(InterfaceOutcome::Returned);
        assert!(observer.snapshot().sealed);
        observer.seal(InterfaceOutcome::Unwound);
        assert_eq!(observer.snapshot().interface_outcome, InterfaceOutcome::Returned);
        let mut json = Vec::new();
        observer.write_json(&mut json).unwrap();
        let json = String::from_utf8(json).unwrap();
        assert!(json.contains("native-join-continuations"));
        assert!(json.contains("task_allocator_baseline"));
        assert!(json.contains("cpu_work_ns"));
        assert!(json.contains("returned"));
        let bounded = ExecutionObserver::new(usize::MAX).unwrap();
        assert_eq!(bounded.capacity, MAX_RECORDS);
        bounded.next_task.store(MAX_RECORDS as u64 + 1, Ordering::Relaxed);
        assert!(bounded.root().is_none());
        assert_eq!(bounded.snapshot().dropped_events, 1);
    }

    #[test]
    fn bounded_and_pool_namespaced() {
        let first = ExecutionObserver::new(1).unwrap();
        let second = ExecutionObserver::new(0).unwrap();
        let a = first.root().unwrap();
        let b = second.root().unwrap();
        assert_ne!(a.id(), b.id());
        drop(a.enter());
        assert_eq!(first.snapshot().events.len(), 1);
        assert_eq!(first.snapshot().dropped_events, 2);
        assert_eq!(second.snapshot().events.len(), 0);
        assert_eq!(second.snapshot().dropped_events, 1);
    }
}
