//! A single immutable-snapshot graph with independently published `u64` facts.
//!
//! Interning is partitioned and compares complete keys, including when hashes
//! deliberately collide. Scheduling uses one global ready queue as a correctness
//! baseline, not a selected scalability-optimal default. Dependency waits release
//! workers. Only engine-internal wakes are supported: quiescent external waits
//! fail explicitly rather than keeping a build alive indefinitely.
//!
//! An engine runs once. Demand cancellation detaches just that demand and does
//! not stop orphan work. Published outputs survive later producer failure, but a
//! successful output alone does not establish success of all producer branches.
//! `Report::node_errors` exposes those later failures. Capture buffers are bounded
//! and overflow makes `capture_complete` false. Structural edges remain retained.

use std::any::Any;
use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque, hash_map::DefaultHasher};
use std::fmt;
use std::future::{Future, poll_fn};
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::task::{Context as PollContext, Poll, Wake, Waker};
use std::time::Instant;

const SHARDS: usize = 64;
const EVENT_CAPACITY: usize = 65_536;
type PendingWaits = (u64, u64, HashSet<(u64, usize)>);
static NEXT_GRAPH: AtomicU64 = AtomicU64::new(1);

thread_local! {
    // The currently polled branch, not the Context that a fork captured.
    static CURRENT_TASK: Cell<Option<(u64, u64)>> = const { Cell::new(None) };
}

/// Canonical identity inside one graph's immutable input snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub kind: u32,
    pub version: u32,
    pub params: Vec<u64>,
}

impl Key {
    pub fn new(kind: u32, params: Vec<u64>) -> Self {
        Self {
            kind,
            version: 1,
            params,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Error {}

pub type TaskFuture = Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'static>>;

pub trait Provider: Send + Sync + 'static {
    fn outputs(&self, key: &Key) -> usize;
    fn start(&self, cx: Context, key: Key) -> TaskFuture;
}

/// An always-on, bounded transition record. IDs have separate namespaces.
#[derive(Clone, Debug)]
pub struct Event {
    pub at_ns: u64,
    pub kind: String,
    pub node: Option<u64>,
    pub task: Option<u64>,
    pub demand: Option<u64>,
    pub detail: String,
}

/// Queue, execution, and wait times are summed branch intervals, not wall time.
/// They are monotonic-clock measurements, not sampled CPU time.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub nodes: usize,
    pub intern_hits: usize,
    pub tasks: usize,
    pub polls: usize,
    pub publications: usize,
    pub edges: usize,
    pub failures: usize,
    pub events_dropped: usize,
    pub queue_ns: u64,
    pub execution_ns: u64,
    pub wait_ns: u64,
    pub elapsed_ns: u64,
    pub workers: usize,
    pub capture_complete: bool,
    pub node_evaluations: Vec<(Key, usize)>,
    pub events: Vec<Event>,
    /// `(consumer branch/task, producer node, output slot)`.
    pub dependency_edges: Vec<(u64, u64, usize)>,
    pub retained_outputs: usize,
    /// Producer failures include failures after an early output was published.
    pub node_errors: Vec<(u64, Error)>,
    pub demands: usize,
    pub cancellations: usize,
    /// CPU samples and allocator/peak-RSS measurements are not implemented.
    pub resource_sampling_available: bool,
}

#[derive(Clone)]
pub struct Engine {
    shared: Arc<Shared>,
}

#[derive(Clone)]
pub struct Context {
    shared: Arc<Shared>,
    node: Arc<Node>,
}

#[derive(Clone)]
pub struct Outputs {
    shared: Arc<Shared>,
    node: Arc<Node>,
}

/// Carries graph identity, so reading a foreign graph cannot silently suspend.
#[derive(Clone)]
pub struct Handle {
    graph: u64,
    node: Arc<Node>,
    index: usize,
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("graph", &self.graph)
            .field("node", &self.node.id)
            .field("index", &self.index)
            .finish()
    }
}

pub struct Demand {
    outputs: Outputs,
    id: u64,
    cancelled: AtomicBool,
}

struct Shared {
    id: u64,
    provider: Arc<dyn Provider>,
    bits: u8,
    index: Vec<Mutex<HashMap<u64, Vec<Arc<Node>>>>>,
    nodes: Mutex<Vec<Arc<Node>>>,
    next_node: AtomicU64,
    next_task: AtomicU64,
    next_demand: AtomicU64,
    next_read: AtomicU64,
    scheduler: Mutex<Scheduler>,
    ready: Condvar,
    accounting: Mutex<Accounting>,
    epoch: Instant,
    #[cfg(test)]
    worker_fault_on_pending_restore: AtomicBool,
    #[cfg(test)]
    worker_abort_instead: AtomicBool,
}

struct Node {
    id: u64,
    key: Key,
    slots: Vec<Mutex<Slot>>,
    state: Mutex<NodeState>,
}

struct NodeState {
    branches: usize,
    completed: bool,
    error: Option<Error>,
}

#[derive(Default)]
struct Slot {
    result: Option<Result<u64, Error>>,
    waiters: HashMap<(u64, u64), Waker>,
}

struct Task {
    id: u64,
    node: Arc<Node>,
    future: Mutex<Option<TaskFuture>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskState {
    Queued,
    Running,
    Suspended,
    Done,
}

struct TaskRecord {
    task: Arc<Task>,
    state: TaskState,
    notified: bool,
    queued_at: Instant,
    wait_since: Option<Instant>,
    waiting: HashMap<(u64, usize), HashSet<u64>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Building,
    Running,
    Draining,
    Finished,
}

struct Scheduler {
    phase: Phase,
    queue: VecDeque<u64>,
    tasks: HashMap<u64, TaskRecord>,
    active: usize,
    live: usize,
}

struct Accounting {
    report: Report,
    edge_set: HashSet<(u64, u64, usize)>,
    evaluations: HashMap<u64, usize>,
    capacity: usize,
}

struct TaskWake {
    shared: Weak<Shared>,
    id: u64,
}

impl Wake for TaskWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if let Some(shared) = self.shared.upgrade() {
            shared.wake_task(self.id);
        }
    }
}

struct PollGuard(Option<(u64, u64)>);
impl Drop for PollGuard {
    fn drop(&mut self) {
        CURRENT_TASK.with(|current| current.set(self.0));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Provider code is never called with an engine lock held. Poison recovery
    // avoids converting an otherwise reportable fault into stranded demands.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn nanos(start: Instant) -> u64 {
    start.elapsed().as_nanos().min(u64::MAX as u128) as u64
}

fn panic_error(where_: &str, payload: Box<dyn Any + Send>) -> Error {
    let text = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic");
    Error::new(format!("{where_} panicked: {text}"))
}

impl Engine {
    pub fn new(provider: impl Provider) -> Self {
        Self::configured(provider, 64, EVENT_CAPACITY)
    }

    /// Truncates the lookup hash, not the key comparison. Zero forces collisions.
    pub fn with_hash_bits(provider: impl Provider, bits: u8) -> Self {
        Self::configured(provider, bits.min(64), EVENT_CAPACITY)
    }

    /// A small capacity is useful for testing truthful capture-overflow handling.
    pub fn with_event_capacity(provider: impl Provider, capacity: usize) -> Self {
        Self::configured(provider, 64, capacity)
    }

    fn configured(provider: impl Provider, bits: u8, capacity: usize) -> Self {
        Self {
            shared: Arc::new(Shared {
                id: NEXT_GRAPH.fetch_add(1, Ordering::Relaxed),
                provider: Arc::new(provider),
                bits,
                index: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
                nodes: Mutex::new(Vec::new()),
                next_node: AtomicU64::new(1),
                next_task: AtomicU64::new(1),
                next_demand: AtomicU64::new(1),
                next_read: AtomicU64::new(1),
                scheduler: Mutex::new(Scheduler {
                    phase: Phase::Building,
                    queue: VecDeque::new(),
                    tasks: HashMap::new(),
                    active: 0,
                    live: 0,
                }),
                ready: Condvar::new(),
                accounting: Mutex::new(Accounting {
                    report: Report {
                        capture_complete: true,
                        ..Report::default()
                    },
                    edge_set: HashSet::new(),
                    evaluations: HashMap::new(),
                    capacity,
                }),
                epoch: Instant::now(),
                #[cfg(test)]
                worker_fault_on_pending_restore: AtomicBool::new(false),
                #[cfg(test)]
                worker_abort_instead: AtomicBool::new(false),
            }),
        }
    }

    pub fn request(&self, key: Key) -> Demand {
        let node = self.shared.intern(key);
        let id = self.shared.next_demand.fetch_add(1, Ordering::Relaxed);
        lock(&self.shared.accounting).report.demands += 1;
        self.shared
            .event("demand", Some(node.id), None, Some(id), "attached".into());
        Demand {
            outputs: Outputs {
                shared: self.shared.clone(),
                node,
            },
            id,
            cancelled: AtomicBool::new(false),
        }
    }

    /// Run exactly once. Provider failures are represented in the returned report
    /// and demand results. Configuration/execution misuse returns `Err` directly.
    pub fn run(&self, workers: usize) -> Result<Report, Error> {
        if workers == 0 || workers > 4096 {
            return Err(Error::new("worker count must be in 1..=4096"));
        }
        {
            let mut scheduler = lock(&self.shared.scheduler);
            if scheduler.phase != Phase::Building {
                return Err(Error::new(
                    "concurrent or repeated Engine::run is unsupported",
                ));
            }
            scheduler.phase = Phase::Running;
        }
        let start = Instant::now();
        let mut infrastructure_error = None;
        std::thread::scope(|scope| {
            let mut threads = Vec::new();
            for worker in 0..workers {
                let shared = self.shared.clone();
                match std::thread::Builder::new()
                    .name(format!("graph-{worker}"))
                    .spawn_scoped(scope, move || {
                        match catch_unwind(AssertUnwindSafe(|| shared.worker())) {
                            Ok(()) => Ok(()),
                            Err(payload) => {
                                let error = panic_error("engine worker", payload);
                                shared.abort(error.clone());
                                Err(error)
                            }
                        }
                    }) {
                    Ok(thread) => threads.push(thread),
                    Err(error) => {
                        infrastructure_error =
                            Some(Error::new(format!("cannot spawn worker: {error}")));
                        break;
                    }
                }
            }
            if let Some(error) = &infrastructure_error {
                self.shared.abort(error.clone());
            }
            for thread in threads {
                match thread.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => infrastructure_error = Some(error),
                    Err(payload) => {
                        let error = panic_error("engine worker", payload);
                        self.shared.abort(error.clone());
                        infrastructure_error = Some(error);
                    }
                }
            }
        });
        // An abort can race with the final Pending transition of an active
        // poll. Only after every worker has joined is it safe to reclaim even
        // records left Running by a worker fault. Never leave a future/Context
        // ownership cycle or an unfinished producer branch behind.
        let stranded = {
            let mut scheduler = lock(&self.shared.scheduler);
            scheduler.phase = Phase::Draining;
            scheduler.active = 0;
            scheduler
                .tasks
                .values()
                .any(|record| record.state != TaskState::Done)
        };
        if stranded {
            let error = infrastructure_error.clone().unwrap_or_else(|| {
                Error::new("engine stopped with unfinished continuations after worker shutdown")
            });
            lock(&self.shared.accounting).report.capture_complete = false;
            self.shared.terminate_quiescent(error.clone(), true);
            infrastructure_error.get_or_insert(error);
        }
        lock(&self.shared.scheduler).phase = Phase::Finished;
        let nodes = lock(&self.shared.nodes).clone();
        let retained_outputs = nodes
            .iter()
            .map(|node| {
                node.slots
                    .iter()
                    .filter(|slot| matches!(lock(slot).result, Some(Ok(_))))
                    .count()
            })
            .sum();
        let node_errors = nodes
            .iter()
            .filter_map(|node| {
                lock(&node.state)
                    .error
                    .clone()
                    .map(|error| (node.id, error))
            })
            .collect();
        let mut accounting = lock(&self.shared.accounting);
        accounting.report.nodes = nodes.len();
        accounting.report.workers = workers;
        accounting.report.elapsed_ns = nanos(start);
        accounting.report.node_evaluations = nodes
            .iter()
            .map(|node| {
                (
                    node.key.clone(),
                    accounting.evaluations.get(&node.id).copied().unwrap_or(0),
                )
            })
            .collect();
        accounting.report.retained_outputs = retained_outputs;
        accounting.report.node_errors = node_errors;
        let report = accounting.report.clone();
        drop(accounting);
        if let Some(error) = infrastructure_error {
            Err(error)
        } else {
            Ok(report)
        }
    }
}

impl Outputs {
    /// An invalid index becomes an explicit read/result error, never a panic.
    pub fn get(&self, index: usize) -> Handle {
        Handle {
            graph: self.shared.id,
            node: self.node.clone(),
            index,
        }
    }
    pub fn node_id(&self) -> u64 {
        self.node.id
    }
    pub fn len(&self) -> usize {
        self.node.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.node.slots.is_empty()
    }
}

impl Demand {
    pub fn outputs(&self) -> Outputs {
        self.outputs.clone()
    }

    /// Detaches only this demand. Orphan work is deliberately not stopped.
    pub fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            lock(&self.outputs.shared.accounting).report.cancellations += 1;
            self.outputs.shared.event(
                "cancel",
                Some(self.outputs.node.id),
                None,
                Some(self.id),
                "demand detached; producer retained".into(),
            );
        }
    }

    /// Nonblocking producer status, separate from independently valid facts.
    /// Every branch and required output must finish before this returns `Ok`.
    pub fn completion(&self) -> Result<(), Error> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Error::new("demand cancelled"));
        }
        let state = lock(&self.outputs.node.state);
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if !state.completed {
            return Err(Error::new("producer not ready"));
        }
        Ok(())
    }

    /// Nonblocking, including before `run`. Early valid facts survive failure.
    pub fn result(&self, index: usize) -> Result<u64, Error> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Error::new("demand cancelled"));
        }
        self.outputs.node.result(index)
    }
}

impl Node {
    fn result(&self, index: usize) -> Result<u64, Error> {
        match self.slots.get(index) {
            Some(slot) => lock(slot)
                .result
                .clone()
                .unwrap_or_else(|| Err(Error::new("output not ready"))),
            None => Err(lock(&self.state).error.clone().unwrap_or_else(|| {
                Error::new(format!(
                    "output index {index} out of range for node {}",
                    self.id
                ))
            })),
        }
    }
}

impl Context {
    pub fn request(&self, key: Key) -> Outputs {
        Outputs {
            node: self.shared.intern(key),
            shared: self.shared.clone(),
        }
    }

    pub async fn read(&self, handle: Handle) -> Result<u64, Error> {
        if handle.graph != self.shared.id {
            return Err(Error::new(
                "output handle belongs to a different engine/graph",
            ));
        }
        let Some(slot) = handle.node.slots.get(handle.index) else {
            return handle.node.result(handle.index);
        };
        let mut registration = ReadRegistration {
            shared: self.shared.clone(),
            node: handle.node.clone(),
            index: handle.index,
            id: self.shared.next_read.fetch_add(1, Ordering::Relaxed),
            task: None,
        };
        poll_fn(|cx| {
            let task = match CURRENT_TASK.with(Cell::get) {
                Some((graph, task)) if graph == self.shared.id => task,
                _ => {
                    return Poll::Ready(Err(Error::new(
                        "Context::read must be polled by its engine task",
                    )));
                }
            };
            if registration.task != Some(task) {
                registration.clear();
                registration.task = Some(task);
                self.shared.edge(task, handle.node.id, handle.index);
            }
            // Registration and the result recheck use the publication lock.
            // Publishing takes this same lock then wakes outside it.
            let mut slot = lock(slot);
            if let Some(result) = &slot.result {
                let result = result.clone();
                slot.waiters.remove(&(task, registration.id));
                drop(slot);
                self.shared
                    .unwait(task, handle.node.id, handle.index, registration.id);
                return Poll::Ready(result);
            }
            slot.waiters
                .insert((task, registration.id), cx.waker().clone());
            self.shared
                .wait(task, handle.node.id, handle.index, registration.id);
            Poll::Pending
        })
        .await
    }

    pub fn publish(&self, index: usize, value: u64) -> Result<(), Error> {
        let state = lock(&self.node.state);
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if state.completed {
            return Err(Error::new("cannot publish after producer completion"));
        }
        let Some(slot) = self.node.slots.get(index) else {
            return Err(Error::new(format!("output index {index} out of range")));
        };
        let mut slot = lock(slot);
        if slot.result.is_some() {
            return Err(Error::new(format!("output {index} already finalized")));
        }
        slot.result = Some(Ok(value));
        let waiters: Vec<_> = slot.waiters.drain().map(|(_, waker)| waker).collect();
        drop(slot);
        drop(state);
        lock(&self.shared.accounting).report.publications += 1;
        self.shared.event(
            "publish",
            Some(self.node.id),
            None,
            None,
            format!("slot {index} = {value}"),
        );
        for waker in waiters {
            waker.wake();
        }
        Ok(())
    }

    /// The future is an independently scheduled continuation of this producer.
    /// A captured Context still records reads against the actual polled branch.
    pub fn fork(&self, future: TaskFuture) -> Result<(), Error> {
        let mut state = lock(&self.node.state);
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if state.completed {
            return Err(Error::new("cannot fork a completed producer"));
        }
        let result = self.shared.schedule(self.node.clone(), future);
        if result.is_ok() {
            state.branches += 1;
        }
        result.map(|_| ())
    }

    pub fn fail(&self, error: Error) {
        self.shared.fail_node(&self.node, error);
    }
}

struct ReadRegistration {
    shared: Arc<Shared>,
    node: Arc<Node>,
    index: usize,
    id: u64,
    task: Option<u64>,
}

impl ReadRegistration {
    fn clear(&mut self) {
        if let Some(task) = self.task.take() {
            if let Some(slot) = self.node.slots.get(self.index) {
                lock(slot).waiters.remove(&(task, self.id));
            }
            self.shared.unwait(task, self.node.id, self.index, self.id);
        }
    }
}
impl Drop for ReadRegistration {
    fn drop(&mut self) {
        self.clear();
    }
}

impl Shared {
    fn event(
        &self,
        kind: &str,
        node: Option<u64>,
        task: Option<u64>,
        demand: Option<u64>,
        detail: String,
    ) {
        let mut accounting = lock(&self.accounting);
        if accounting.report.events.len() < accounting.capacity {
            accounting.report.events.push(Event {
                at_ns: nanos(self.epoch),
                kind: kind.into(),
                node,
                task,
                demand,
                detail,
            });
        } else {
            accounting.report.events_dropped += 1;
            accounting.report.capture_complete = false;
        }
    }

    fn hash(&self, key: &Key) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        match self.bits {
            0 => 0,
            64 => hasher.finish(),
            bits => hasher.finish() & ((1_u64 << bits) - 1),
        }
    }

    fn intern(self: &Arc<Self>, key: Key) -> Arc<Node> {
        let hash = self.hash(&key);
        let shard = &self.index[hash as usize % SHARDS];
        {
            let index = lock(shard);
            if let Some(node) = index
                .get(&hash)
                .and_then(|bucket| bucket.iter().find(|node| node.key == key))
            {
                let node = node.clone();
                drop(index);
                lock(&self.accounting).report.intern_hits += 1;
                self.event("intern-hit", Some(node.id), None, None, String::new());
                return node;
            }
        }
        // outputs() is metadata, not evaluation, and may be called by competing
        // requests. It runs outside every engine lock, including the shard lock.
        let output_count = catch_unwind(AssertUnwindSafe(|| self.provider.outputs(&key)));
        let (count, output_error) = match output_count {
            Ok(count) => (count, None),
            Err(payload) => (0, Some(panic_error("provider outputs", payload))),
        };
        let mut index = lock(shard);
        let bucket = index.entry(hash).or_default();
        if let Some(node) = bucket.iter().find(|node| node.key == key) {
            let node = node.clone();
            drop(index);
            lock(&self.accounting).report.intern_hits += 1;
            self.event("intern-hit", Some(node.id), None, None, String::new());
            return node;
        }
        let node = Arc::new(Node {
            id: self.next_node.fetch_add(1, Ordering::Relaxed),
            key: key.clone(),
            slots: (0..count).map(|_| Mutex::new(Slot::default())).collect(),
            state: Mutex::new(NodeState {
                branches: 1,
                completed: false,
                error: None,
            }),
        });
        bucket.push(node.clone());
        lock(&self.nodes).push(node.clone());
        self.event(
            "discovered",
            Some(node.id),
            None,
            None,
            format!("{:?}", node.key),
        );
        if let Some(error) = output_error {
            self.fail_node(&node, error);
            self.finish_branch(&node);
        } else {
            let weak = Arc::downgrade(self);
            let provider = self.provider.clone();
            let producer = node.clone();
            let future: TaskFuture = Box::pin(async move {
                let shared = weak
                    .upgrade()
                    .ok_or_else(|| Error::new("graph dropped before evaluation"))?;
                *lock(&shared.accounting)
                    .evaluations
                    .entry(producer.id)
                    .or_default() += 1;
                shared.event("evaluate", Some(producer.id), None, None, String::new());
                let cx = Context {
                    shared,
                    node: producer,
                };
                let future = catch_unwind(AssertUnwindSafe(|| provider.start(cx, key)))
                    .map_err(|payload| panic_error("provider start", payload))?;
                future.await
            });
            if let Err(error) = self.schedule(node.clone(), future) {
                self.fail_node(&node, error);
                self.finish_branch(&node);
            }
        }
        drop(index);
        node
    }

    fn schedule(self: &Arc<Self>, node: Arc<Node>, future: TaskFuture) -> Result<u64, Error> {
        let mut scheduler = lock(&self.scheduler);
        if matches!(scheduler.phase, Phase::Draining | Phase::Finished) {
            return Err(Error::new(
                "cannot schedule new work after engine quiescence/run completion",
            ));
        }
        let id = self.next_task.fetch_add(1, Ordering::Relaxed);
        let node_id = node.id;
        let task = Arc::new(Task {
            id,
            node,
            future: Mutex::new(Some(future)),
        });
        scheduler.tasks.insert(
            id,
            TaskRecord {
                task,
                state: TaskState::Queued,
                notified: false,
                queued_at: Instant::now(),
                wait_since: None,
                waiting: HashMap::new(),
            },
        );
        scheduler.queue.push_back(id);
        scheduler.live += 1;
        lock(&self.accounting).report.tasks += 1;
        self.event(
            "queued",
            Some(node_id),
            Some(id),
            None,
            "new continuation".into(),
        );
        drop(scheduler);
        self.ready.notify_one();
        Ok(id)
    }

    fn edge(&self, task: u64, node: u64, index: usize) {
        let mut accounting = lock(&self.accounting);
        if accounting.edge_set.insert((task, node, index)) {
            accounting.report.dependency_edges.push((task, node, index));
            accounting.report.edges += 1;
        }
    }

    fn wait(&self, task: u64, node: u64, index: usize, registration: u64) {
        if let Some(record) = lock(&self.scheduler).tasks.get_mut(&task) {
            record
                .waiting
                .entry((node, index))
                .or_default()
                .insert(registration);
        }
    }

    fn unwait(&self, task: u64, node: u64, index: usize, registration: u64) {
        let mut scheduler = lock(&self.scheduler);
        let Some(record) = scheduler.tasks.get_mut(&task) else {
            return;
        };
        let Some(registrations) = record.waiting.get_mut(&(node, index)) else {
            return;
        };
        registrations.remove(&registration);
        if registrations.is_empty() {
            record.waiting.remove(&(node, index));
        }
    }

    fn wake_task(&self, id: u64) {
        let mut scheduler = lock(&self.scheduler);
        if scheduler.phase != Phase::Running && scheduler.phase != Phase::Building {
            return;
        }
        let Some(record) = scheduler.tasks.get_mut(&id) else {
            return;
        };
        let node = record.task.node.id;
        let mut waited = 0;
        match record.state {
            TaskState::Running => {
                // A wake while poll() is active must not enqueue a concurrent
                // poll. The poll owner performs the deferred enqueue.
                record.notified = true;
                return;
            }
            TaskState::Suspended => {
                record.state = TaskState::Queued;
                record.queued_at = Instant::now();
                if let Some(start) = record.wait_since.take() {
                    waited = nanos(start);
                }
                scheduler.queue.push_back(id);
            }
            TaskState::Queued | TaskState::Done => return,
        }
        {
            let mut accounting = lock(&self.accounting);
            accounting.report.wait_ns = accounting.report.wait_ns.saturating_add(waited);
        }
        self.event(
            "queued",
            Some(node),
            Some(id),
            None,
            "dependency/waker notification".into(),
        );
        drop(scheduler);
        self.ready.notify_one();
    }

    fn fail_node(&self, node: &Arc<Node>, error: Error) {
        let mut state = lock(&node.state);
        if state.error.is_some() {
            return;
        }
        state.error = Some(error.clone());
        let mut waiters = Vec::new();
        for slot in &node.slots {
            let mut slot = lock(slot);
            if slot.result.is_none() {
                slot.result = Some(Err(error.clone()));
            }
            waiters.extend(slot.waiters.drain().map(|(_, waker)| waker));
        }
        drop(state);
        lock(&self.accounting).report.failures += 1;
        self.event("failed", Some(node.id), None, None, error.0);
        for waker in waiters {
            waker.wake();
        }
    }

    fn finish_branch(&self, node: &Arc<Node>) {
        let mut state = lock(&node.state);
        if state.branches == 0 {
            return;
        }
        state.branches -= 1;
        if state.branches != 0 {
            return;
        }
        state.completed = true;
        let missing: Vec<_> = node
            .slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                if lock(slot).result.is_none() {
                    Some(index)
                } else {
                    None
                }
            })
            .collect();
        let error = if !missing.is_empty() && state.error.is_none() {
            Some(Error::new(format!(
                "producer node {} finished without finalizing required outputs {missing:?}",
                node.id
            )))
        } else {
            None
        };
        let mut waiters = Vec::new();
        if let Some(error) = &error {
            // Status and missing-output terminalization share one critical
            // section. completion() must never observe an intermediate success.
            state.error = Some(error.clone());
            for slot in &node.slots {
                let mut slot = lock(slot);
                if slot.result.is_none() {
                    slot.result = Some(Err(error.clone()));
                }
                waiters.extend(slot.waiters.drain().map(|(_, waker)| waker));
            }
        }
        let failed = state.error.is_some();
        drop(state);
        if let Some(error) = error {
            lock(&self.accounting).report.failures += 1;
            self.event("failed", Some(node.id), None, None, error.0);
        }
        for waker in waiters {
            waker.wake();
        }
        self.event(
            if failed {
                "producer-finished-failed"
            } else {
                "completed"
            },
            Some(node.id),
            None,
            None,
            "all branches finished".into(),
        );
    }

    fn worker(self: &Arc<Self>) {
        loop {
            let (task, queued_ns) = {
                let mut scheduler = lock(&self.scheduler);
                loop {
                    if scheduler.phase != Phase::Running {
                        return;
                    }
                    if let Some(id) = scheduler.queue.pop_front() {
                        let Some(record) = scheduler.tasks.get_mut(&id) else {
                            continue;
                        };
                        if record.state != TaskState::Queued {
                            continue;
                        }
                        record.state = TaskState::Running;
                        record.notified = false;
                        let queued_ns = nanos(record.queued_at);
                        let task = record.task.clone();
                        scheduler.active += 1;
                        self.event(
                            "running",
                            Some(task.node.id),
                            Some(task.id),
                            None,
                            String::new(),
                        );
                        break (task, queued_ns);
                    }
                    if scheduler.active == 0 {
                        // The phase change and active/queue check are atomic.
                        // No producer is polling or between poll and completion.
                        scheduler.phase = Phase::Draining;
                        let pending: Vec<_> = scheduler
                            .tasks
                            .values()
                            .filter(|record| record.state != TaskState::Done)
                            .map(|record| {
                                (
                                    record.task.id,
                                    record.task.node.id,
                                    record.waiting.keys().copied().collect(),
                                )
                            })
                            .collect();
                        drop(scheduler);
                        if !pending.is_empty() {
                            self.terminate_quiescent(Self::quiescence_error(&pending), false);
                        }
                        self.ready.notify_all();
                        return;
                    }
                    scheduler = self
                        .ready
                        .wait(scheduler)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            };
            {
                let mut accounting = lock(&self.accounting);
                accounting.report.queue_ns = accounting.report.queue_ns.saturating_add(queued_ns);
                accounting.report.polls += 1;
            }
            let start = Instant::now();
            let waker = Waker::from(Arc::new(TaskWake {
                shared: Arc::downgrade(self),
                id: task.id,
            }));
            let mut cx = PollContext::from_waker(&waker);
            let previous = CURRENT_TASK.with(|current| current.replace(Some((self.id, task.id))));
            let guard = PollGuard(previous);
            // Ownership is removed from the mutex during arbitrary provider code.
            // Scheduler state, not this mutex, prevents overlapping polls.
            let mut future = lock(&task.future).take();
            let polled = catch_unwind(AssertUnwindSafe(|| {
                future.as_mut().map_or_else(
                    || Poll::Ready(Err(Error::new("task future missing during poll"))),
                    |future| future.as_mut().poll(&mut cx),
                )
            }));
            drop(guard);
            let mut result = match polled {
                Ok(result) => result,
                Err(payload) => Poll::Ready(Err(panic_error("provider future poll", payload))),
            };
            let execution = nanos(start);
            {
                let mut accounting = lock(&self.accounting);
                accounting.report.execution_ns =
                    accounting.report.execution_ns.saturating_add(execution);
            }
            if lock(&self.scheduler).phase != Phase::Running {
                result = Poll::Ready(Err(Error::new("engine execution aborted")));
            }
            if result.is_pending() {
                *lock(&task.future) = future.take();
            } else {
                // Destructors are also provider code and execute without locks.
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(future.take()))) {
                    result = Poll::Ready(Err(panic_error("provider future drop", payload)));
                }
            }
            #[cfg(test)]
            if self
                .worker_fault_on_pending_restore
                .swap(false, Ordering::SeqCst)
            {
                // Deliberately outside the provider catch boundary, after a
                // Pending future has been restored but while its record is
                // still Running. This exercises post-join ownership cleanup.
                if self.worker_abort_instead.load(Ordering::Relaxed) {
                    self.abort(Error::new(
                        "injected engine abort after pending future restoration",
                    ));
                } else {
                    panic!("injected engine worker fault after pending future restoration");
                }
            }
            if let Poll::Ready(outcome) = &result {
                if let Err(error) = outcome {
                    self.fail_node(&task.node, error.clone());
                }
                self.finish_branch(&task.node);
            }
            let mut scheduler = lock(&self.scheduler);
            let record = scheduler
                .tasks
                .get_mut(&task.id)
                .expect("scheduled task retained");
            let mut requeue = false;
            let suspension_reason = if record.waiting.is_empty() {
                "pending without registered engine dependencies; external-waker waits unsupported at quiescence".to_owned()
            } else {
                let mut waits: Vec<_> = record.waiting.keys().copied().collect();
                waits.sort_unstable();
                format!("awaiting producer slots {waits:?}")
            };
            if result.is_ready() {
                record.state = TaskState::Done;
                record.waiting.clear();
                scheduler.live -= 1;
            } else if record.notified {
                record.state = TaskState::Queued;
                record.queued_at = Instant::now();
                requeue = true;
            } else {
                record.state = TaskState::Suspended;
                record.wait_since = Some(Instant::now());
            }
            if requeue {
                scheduler.queue.push_back(task.id);
            }
            // Keep active until publication, finalization, and enqueue are done.
            scheduler.active -= 1;
            self.event(
                if result.is_ready() {
                    "task-done"
                } else if requeue {
                    "queued"
                } else {
                    "suspended"
                },
                Some(task.node.id),
                Some(task.id),
                None,
                if result.is_ready() {
                    String::new()
                } else if requeue {
                    "notification during poll".into()
                } else {
                    suspension_reason
                },
            );
            drop(scheduler);
            self.ready.notify_all();
        }
    }

    fn quiescence_error(pending: &[PendingWaits]) -> Error {
        let mut sorted: Vec<_> = pending.iter().collect();
        sorted.sort_by_key(|(task, _, _)| *task);
        let external: Vec<_> = sorted
            .iter()
            .filter(|(_, _, waits)| waits.is_empty())
            .map(|(task, _, _)| *task)
            .collect();
        let reason = if external.is_empty() {
            "unproductive dependency cycle: all remaining continuations await unresolved engine outputs".to_owned()
        } else {
            format!(
                "unsupported external-waker wait at tasks {external:?}: prototype supports engine-internal waits only"
            )
        };
        let chain: Vec<_> = sorted
            .iter()
            .take(64)
            .map(|(task, node, waits)| {
                let mut waits: Vec<_> = waits.iter().copied().collect();
                waits.sort_unstable();
                format!("task {task} (node {node}) -> producer slots {waits:?}")
            })
            .collect();
        Error::new(format!(
            "quiescence: {reason}; dependency wait chain: {}{}",
            chain.join("; "),
            if pending.len() > 64 {
                " (additional tasks omitted)"
            } else {
                ""
            }
        ))
    }

    fn terminate_quiescent(&self, error: Error, include_running: bool) {
        let tasks = {
            let mut scheduler = lock(&self.scheduler);
            let mut tasks = Vec::new();
            let mut waited = 0_u64;
            for record in scheduler.tasks.values_mut() {
                if record.state == TaskState::Done
                    || (!include_running && record.state == TaskState::Running)
                {
                    continue;
                }
                record.state = TaskState::Done;
                if let Some(start) = record.wait_since.take() {
                    waited = waited.saturating_add(nanos(start));
                }
                tasks.push(record.task.clone());
            }
            scheduler.live -= tasks.len();
            scheduler.queue.clear();
            drop(scheduler);
            let mut accounting = lock(&self.accounting);
            accounting.report.wait_ns = accounting.report.wait_ns.saturating_add(waited);
            tasks
        };
        for task in tasks {
            self.fail_node(&task.node, error.clone());
            let future = lock(&task.future).take();
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(future))) {
                lock(&self.accounting).report.capture_complete = false;
                self.event(
                    "drop-panic",
                    Some(task.node.id),
                    Some(task.id),
                    None,
                    panic_error("provider future drop", payload).0,
                );
            }
            self.finish_branch(&task.node);
            self.event(
                "task-terminal",
                Some(task.node.id),
                Some(task.id),
                None,
                error.0.clone(),
            );
        }
    }

    fn abort(&self, error: Error) {
        lock(&self.scheduler).phase = Phase::Draining;
        lock(&self.accounting).report.capture_complete = false;
        let nodes = lock(&self.nodes).clone();
        for node in nodes {
            let completed = lock(&node.state).completed;
            if !completed {
                self.fail_node(&node, error.clone());
            }
        }
        self.terminate_quiescent(error, false);
        self.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PendingProvider(Arc<AtomicU64>);

    struct Dropped(Arc<AtomicU64>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Provider for PendingProvider {
        fn outputs(&self, _: &Key) -> usize {
            2
        }
        fn start(&self, cx: Context, _: Key) -> TaskFuture {
            let drops = self.0.clone();
            Box::pin(async move {
                let _dropped = Dropped(drops);
                cx.publish(0, 41)?;
                std::future::pending::<()>().await;
                // Retain Context across Pending so a leaked future would keep
                // Shared alive even after all external handles were dropped.
                cx.publish(1, 42)
            })
        }
    }

    fn fault_cleanup(abort_instead: bool) {
        for workers in [1, 2, 8] {
            let drops = Arc::new(AtomicU64::new(0));
            let engine = Engine::new(PendingProvider(drops.clone()));
            let shared = Arc::downgrade(&engine.shared);
            let demand = engine.request(Key::new(1, vec![]));
            engine
                .shared
                .worker_fault_on_pending_restore
                .store(true, Ordering::SeqCst);
            engine
                .shared
                .worker_abort_instead
                .store(abort_instead, Ordering::SeqCst);
            assert!(engine.run(workers).is_err());
            assert_eq!(
                demand.result(0),
                Ok(41),
                "early publication survives worker failure"
            );
            assert!(demand.result(1).is_err());
            assert!(demand.completion().is_err());
            assert_eq!(
                drops.load(Ordering::SeqCst),
                1,
                "pending future dropped exactly once"
            );
            assert_eq!(lock(&demand.outputs.node.state).branches, 0);
            assert!(!lock(&engine.shared.accounting).report.capture_complete);
            {
                let scheduler = lock(&engine.shared.scheduler);
                assert_eq!(scheduler.live, 0);
                assert_eq!(scheduler.active, 0);
                assert!(scheduler.queue.is_empty());
                assert!(scheduler.tasks.values().all(|record| {
                    record.state == TaskState::Done && lock(&record.task.future).is_none()
                }));
            }
            drop(demand);
            drop(engine);
            assert!(
                shared.upgrade().is_none(),
                "unfinished future must not retain its graph"
            );
        }
    }

    #[test]
    fn outer_worker_fault_drains_running_future_and_breaks_context_cycle() {
        fault_cleanup(false);
    }

    #[test]
    fn abort_between_pending_restore_and_suspend_drains_and_breaks_context_cycle() {
        fault_cleanup(true);
    }
}
