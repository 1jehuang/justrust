//! Opt-in, bounded logical fork/join receipts, independent of rustc_session.
//! Only explicitly instrumented joins are covered. Dequeue order is not a causal
//! edge, and these receipts make no CPU, span, or full-pipeline coverage claim.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MAX_RECORDS: usize = 100_000;

static NEXT_OBSERVER: AtomicU64 = AtomicU64::new(1);

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskId {
    pub observer: u64,
    pub task: u64,
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
}

impl ExecutionObserver {
    /// Identity exhaustion disables recording, never wraps or affects work.
    pub fn new(capacity: usize) -> Option<Arc<Self>> {
        let id = allocate_id(&NEXT_OBSERVER, u64::MAX)?;
        let capacity = capacity.min(MAX_RECORDS);
        let mut events = Vec::new();
        let mut timestamps = Vec::new();
        events.try_reserve_exact(capacity).ok()?;
        timestamps.try_reserve_exact(capacity).ok()?;
        Some(Arc::new(Self {
            id,
            attached: AtomicBool::new(false),
            next_task: AtomicU64::new(1),
            capacity,
            epoch: Instant::now(),
            capture_id: format!(
                "{}-{}-{id}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
            ),
            buffer: Mutex::new(Buffer {
                events,
                timestamps,
                dropped: 0,
                sealed: false,
                outcome: InterfaceOutcome::Unknown,
            }),
        }))
    }

    fn event(&self, event: Event) {
        let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
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
        let buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
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
        let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        buffer.dropped = buffer.dropped.saturating_add(1);
    }

    /// Seal only after the root interface and pool work finish. Never implicit.
    pub fn seal(&self, outcome: InterfaceOutcome) {
        let mut buffer = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        if !buffer.sealed {
            buffer.outcome = outcome;
            buffer.sealed = true;
        }
    }

    /// Separate receipt protocol, intentionally not a query-admission schema.
    /// IDs in events are task-local numbers qualified by the header observer.
    pub fn write_json(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        let receipt = self.snapshot();
        write!(
            out,
            "{{\"schema\":1,\"producer\":\"rustc-thread-pool-join\",\"scope\":\"native-join-continuations\",\"capture_id\":\"{}\",\"observer\":{},\"full_pipeline_complete\":false,\"cpu_work_ns\":null,\"execution_span_ns\":null,\"scope_coverage\":false,\"scheduler_coverage\":false,\"pre_attach_coverage\":false,\"sealed\":{},\"interface_outcome\":\"{}\",\"task_allocator_baseline\":1,\"task_allocator_high_water\":{},\"expected_tasks\":{},\"dropped_events\":{},\"events\":[",
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
        writeln!(out, "]}}")
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

/// Generic dispatch is not a lineage edge. Hide the helping invocation's exact
/// active slot until the dispatched job returns. Instrumented closures enter
/// their own contexts, while opaque work stays unbound. Never finish or replace
/// the suspended slot's contents, including an in-progress fork's None state.
pub(crate) fn suspend_for_dispatch() -> DispatchGuard {
    let previous = CURRENT.with(|current| current.replace(None));
    DispatchGuard { previous, _not_send: PhantomData }
}

/// Worker-local restoration, including when a dispatched closure unwinds.
pub(crate) struct DispatchGuard {
    previous: Option<Active>,
    _not_send: PhantomData<Rc<()>>,
}

impl Drop for DispatchGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.previous.take()));
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
        if !self.start() {
            return None;
        }
        let active = Rc::new(RefCell::new(Some(self.clone())));
        let previous = CURRENT.with(|current| current.replace(Some(Rc::clone(&active))));
        Some(TaskGuard { active, previous, outcome: Outcome::Normal, _not_send: PhantomData })
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
        for child in &children {
            self.observer.event(Event::Fork {
                parent: self.id,
                child: child.id,
                continuation: continuation.id,
            });
        }
        self.finish(Outcome::Normal);
        active.replace(None);
        Some(Fork { observer: Arc::clone(&self.observer), children, continuation, origin: active })
    }
}

/// Not Send: restores the executing worker's context, including after unwind.
pub struct TaskGuard {
    active: Active,
    previous: Option<Active>,
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
        let outcome = if std::thread::panicking() { Outcome::Panic } else { self.outcome };
        if let Some(task) = self.active.borrow_mut().take() {
            task.finish(outcome);
            *task.completion.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.id);
        }
        CURRENT.with(|current| {
            current.replace(self.previous.take());
        });
    }
}

pub struct Fork {
    observer: Arc<ExecutionObserver>,
    children: Vec<TaskContext>,
    continuation: TaskContext,
    origin: Active,
}

impl Fork {
    pub fn children(&self) -> &[TaskContext] {
        &self.children
    }

    /// Resume in the original suspended guard after the scheduler's actual
    /// join. An early call or an unrelated active context creates no edges.
    pub fn resume(self) -> Result<TaskId, Self> {
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
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
