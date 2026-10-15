//! Explicit native control flow. IDs and timestamps never imply causal edges.
//! Unknown scheduler/foreign work is represented by cuts, not inferred ancestry.
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

mod readiness;
pub use readiness::{LatchReadinessToken, LatchWaitGuard, ReadinessPreparation};

static TLS_FAILED: AtomicBool = AtomicBool::new(false);
thread_local! {
    static CURRENT: RefCell<Option<Cursor>> = const { RefCell::new(None) };
}
#[derive(Clone)]
pub(crate) struct Cursor {
    recorder: Arc<Recorder>,
    tail: u64,
    scope: Option<u64>,
    next: &'static str,
}
struct Node {
    id: u64,
    kind: &'static str,
    at: u64,
    segment: Option<u64>,
    scope: Option<u64>,
    task: Option<u64>,
    outcome: Option<&'static str>,
    gap: Option<&'static str>,
}
struct Edge {
    id: u64,
    from: u64,
    to: u64,
    kind: &'static str,
}
struct Scope {
    id: u64,
    context: u64,
    job: u64,
    name: &'static str,
    entry: u64,
    exit: Option<u64>,
    outcome: &'static str,
}
struct Buffer {
    readiness: Option<readiness::Inventory>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    scopes: Vec<Scope>,
    next_node: u64,
    next_edge: u64,
    next_scope: u64,
    lost_nodes: u64,
    lost_edges: u64,
    lost_scopes: u64,
    exhausted: bool,
    overflowed: bool,
    active: u64,
    sealed: bool,
}
pub(crate) struct Recorder {
    namespace: Option<u64>,
    epoch: Instant,
    capacity: usize,
    buffer: Mutex<Buffer>,
}
fn id(next: &mut u64, exhausted: &mut bool) -> Option<u64> {
    let value = *next;
    match value.checked_add(1) {
        Some(n) => {
            *next = n;
            Some(value)
        }
        None => {
            *exhausted = true;
            None
        }
    }
}
fn loss(counter: &mut u64, overflowed: &mut bool) {
    if let Some(n) = counter.checked_add(1) { *counter = n } else { *overflowed = true }
}
pub(crate) fn take() -> Option<Cursor> {
    CURRENT
        .try_with(|slot| slot.try_borrow_mut().map(|mut slot| slot.take()))
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| {
            TLS_FAILED.store(true, Ordering::Relaxed);
            None
        })
}
pub(crate) fn restore(cursor: Option<Cursor>) {
    if CURRENT
        .try_with(|slot| slot.try_borrow_mut().map(|mut slot| *slot = cursor))
        .ok()
        .and_then(Result::ok)
        .is_none()
    {
        TLS_FAILED.store(true, Ordering::Relaxed);
    }
}
fn current() -> Option<Cursor> {
    CURRENT
        .try_with(|slot| slot.try_borrow().map(|slot| slot.clone()))
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| {
            TLS_FAILED.store(true, Ordering::Relaxed);
            None
        })
}
impl Recorder {
    pub(crate) fn new(epoch: Instant, capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            namespace: None,
            epoch,
            capacity: capacity.saturating_mul(4).min(4_000_000),
            buffer: Mutex::new(Buffer {
                readiness: None,
                nodes: Vec::new(),
                edges: Vec::new(),
                scopes: Vec::new(),
                next_node: 1,
                next_edge: 1,
                next_scope: 1,
                lost_nodes: 0,
                lost_edges: 0,
                lost_scopes: 0,
                exhausted: false,
                overflowed: false,
                active: 0,
                sealed: false,
            }),
        })
    }
    pub(crate) fn new_with_readiness(epoch: Instant, capacity: usize, namespace: u64) -> Arc<Self> {
        let mut recorder = Self::new(epoch, capacity);
        let inner = Arc::get_mut(&mut recorder).unwrap();
        inner.namespace = Some(namespace);
        inner.buffer.get_mut().unwrap().readiness = Some(readiness::Inventory::new(capacity));
        recorder
    }
    pub(crate) fn readiness_enabled(&self) -> bool {
        self.namespace.is_some()
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Buffer> {
        self.buffer.lock().unwrap_or_else(|e| {
            let mut b = e.into_inner();
            b.exhausted = true;
            b
        })
    }
    fn now(&self) -> Option<u64> {
        u64::try_from(self.epoch.elapsed().as_nanos()).ok()
    }
    fn node(
        self: &Arc<Self>,
        kind: &'static str,
        at: Option<u64>,
        segment: Option<u64>,
        scope: Option<u64>,
        task: Option<u64>,
        outcome: Option<&'static str>,
        gap: Option<&'static str>,
    ) -> Option<Cursor> {
        self.node_attempt(kind, at, segment, scope, task, outcome, gap).0
    }
    fn node_attempt(
        self: &Arc<Self>,
        kind: &'static str,
        at: Option<u64>,
        segment: Option<u64>,
        scope: Option<u64>,
        task: Option<u64>,
        outcome: Option<&'static str>,
        gap: Option<&'static str>,
    ) -> (Option<Cursor>, Option<u64>) {
        let mut b = self.lock();
        let Buffer { next_node, exhausted, .. } = &mut *b;
        let allocated = id(next_node, exhausted);
        let retained = !b.sealed
            && b.nodes.len() < self.capacity
            && at.is_some()
            && allocated.is_some()
            && b.nodes.try_reserve(1).is_ok();
        if !retained {
            let Buffer { lost_nodes, overflowed, .. } = &mut *b;
            loss(lost_nodes, overflowed);
            return (None, allocated);
        }
        let id = allocated.unwrap();
        b.nodes.push(Node { id, kind, at: at.unwrap(), segment, scope, task, outcome, gap });
        (Some(Cursor { recorder: Arc::clone(self), tail: id, scope, next: "strand" }), Some(id))
    }
    pub(crate) fn edge(&self, from: &Cursor, to: &Cursor, kind: &'static str) {
        if !std::ptr::eq(self, from.recorder.as_ref()) || !std::ptr::eq(self, to.recorder.as_ref())
        {
            self.lock().exhausted = true;
            return;
        }
        let mut b = self.lock();
        let Buffer { next_edge, exhausted, .. } = &mut *b;
        let allocated = id(next_edge, exhausted);
        if b.sealed
            || b.edges.len() >= self.capacity
            || allocated.is_none()
            || b.edges.try_reserve(1).is_err()
        {
            let Buffer { lost_edges, overflowed, .. } = &mut *b;
            loss(lost_edges, overflowed);
            return;
        }
        b.edges.push(Edge { id: allocated.unwrap(), from: from.tail, to: to.tail, kind });
    }
    fn append(
        self: &Arc<Self>,
        kind: &'static str,
        at: Option<u64>,
        segment: Option<u64>,
        scope: Option<u64>,
        task: Option<u64>,
        outcome: Option<&'static str>,
        gap: Option<&'static str>,
    ) -> Option<Cursor> {
        let previous = take();
        let next = self.node(kind, at, segment, scope, task, outcome, gap);
        if let (Some(previous), Some(next)) = (&previous, &next) {
            if Arc::ptr_eq(&previous.recorder, self) {
                self.edge(previous, next, previous.next);
            }
        }
        restore(next.clone());
        next
    }
    pub(crate) fn cpu(
        self: &Arc<Self>,
        segment: u64,
        at: Option<u64>,
        task: Option<u64>,
        query_owned: bool,
    ) {
        let scope = current().filter(|c| Arc::ptr_eq(&c.recorder, self)).and_then(|c| c.scope);
        if query_owned && scope.is_none() {
            // Retain the independently qualified timing row, not an invalid
            // query CPU node with an invented/missing invocation identity.
            let mut b = self.lock();
            let Buffer { next_node, exhausted, .. } = &mut *b;
            let _ = id(next_node, exhausted);
            let Buffer { lost_nodes, overflowed, .. } = &mut *b;
            loss(lost_nodes, overflowed);
            return;
        }
        self.append("cpu_segment", at, Some(segment), scope, task, None, None);
    }
    /// A timing attempt without an identity cannot leave a pending call/return
    /// token live. Never reuse its predecessor or manufacture segment zero.
    pub(crate) fn unavailable_cpu(&self) {
        take();
        let mut b = self.lock();
        let Buffer { next_node, exhausted, .. } = &mut *b;
        let _ = id(next_node, exhausted);
        let Buffer { lost_nodes, overflowed, .. } = &mut *b;
        loss(lost_nodes, overflowed);
    }
    pub(crate) fn gate(
        self: &Arc<Self>,
        kind: &'static str,
        task: Option<u64>,
        outcome: Option<&'static str>,
    ) -> Option<Cursor> {
        let scope = current().filter(|c| Arc::ptr_eq(&c.recorder, self)).and_then(|c| c.scope);
        self.append(kind, self.now(), None, scope, task, outcome, None)
    }
    pub(crate) fn enter_scope(
        self: &Arc<Self>,
        context: u64,
        job: u64,
        name: &'static str,
    ) -> ScopeGuard {
        let previous = take();
        let scope = {
            let mut b = self.lock();
            let Buffer { next_scope, exhausted, .. } = &mut *b;
            let allocated = id(next_scope, exhausted);
            if b.sealed
                || b.scopes.len() >= self.capacity
                || allocated.is_none()
                || b.scopes.try_reserve(1).is_err()
            {
                let Buffer { lost_scopes, overflowed, .. } = &mut *b;
                loss(lost_scopes, overflowed);
                None
            } else {
                allocated
            }
        };
        let entry = scope.and_then(|scope| {
            self.node("scope_enter", self.now(), None, Some(scope), None, None, None)
        });
        if let (Some(previous), Some(entry)) = (&previous, &entry) {
            if Arc::ptr_eq(&previous.recorder, self) {
                self.edge(previous, entry, "sync_call");
            }
        }
        let mut row_index = None;
        if let (Some(scope), Some(entry)) = (scope, &entry) {
            let mut b = self.lock();
            if b.scopes.len() < self.capacity && b.scopes.try_reserve(1).is_ok() && !b.sealed {
                row_index = Some(b.scopes.len());
                b.scopes.push(Scope {
                    id: scope,
                    context,
                    job,
                    name,
                    entry: entry.tail,
                    exit: None,
                    outcome: "active",
                });
                if let Some(n) = b.active.checked_add(1) {
                    b.active = n
                } else {
                    b.exhausted = true
                }
            } else {
                let Buffer { lost_scopes, overflowed, .. } = &mut *b;
                loss(lost_scopes, overflowed);
            }
        } else if scope.is_some() {
            let mut b = self.lock();
            let Buffer { lost_scopes, overflowed, .. } = &mut *b;
            loss(lost_scopes, overflowed);
        }
        restore(entry);
        ScopeGuard { recorder: Arc::clone(self), scope, row_index, previous }
    }
    pub(crate) fn seal(&self) {
        self.lock().sealed = true;
    }
    pub(crate) fn write_json(
        &self,
        out: &mut impl std::io::Write,
        timing_failed: bool,
    ) -> std::io::Result<()> {
        let b = self.lock();
        let tls = TLS_FAILED.load(Ordering::Relaxed) || timing_failed;
        let exhausted = b.exhausted || tls;
        let complete = b.readiness.as_ref().is_none_or(|r| r.complete())
            && b.sealed
            && b.active == 0
            && !exhausted
            && !b.overflowed
            && b.lost_nodes == 0
            && b.lost_edges == 0
            && b.lost_scopes == 0
            && b.nodes.len() as u64 == b.next_node - 1
            && b.edges.len() as u64 == b.next_edge - 1
            && b.scopes.len() as u64 == b.next_scope - 1;
        write!(
            out,
            ",\"execution_flow\":{{\"schema\":{},\"scope\":\"{}\",\"full_pipeline_complete\":false,\"flow_complete\":false,\"sealed\":{},\"records_complete\":{complete},\"identity_exhausted\":{exhausted},\"tls_failed\":{tls},\"active_scopes\":{},\"node_allocator_baseline\":1,\"node_allocator_high_water\":{},\"expected_nodes\":{},\"edge_allocator_baseline\":1,\"edge_allocator_high_water\":{},\"expected_edges\":{},\"scope_allocator_baseline\":1,\"scope_allocator_high_water\":{},\"expected_scopes\":{},\"lost_nodes\":{},\"lost_edges\":{},\"lost_scopes\":{},\"loss_overflowed\":{},\"nodes\":[",
            if self.readiness_enabled() { 2 } else { 1 },
            if self.readiness_enabled() {
                "observed-query-join-and-latch-control-flow"
            } else {
                "observed-query-and-join-control-flow"
            },
            b.sealed,
            b.active,
            num((!exhausted).then_some(b.next_node)),
            num((!exhausted).then_some(b.next_node - 1)),
            num((!exhausted).then_some(b.next_edge)),
            num((!exhausted).then_some(b.next_edge - 1)),
            num((!exhausted).then_some(b.next_scope)),
            num((!exhausted).then_some(b.next_scope - 1)),
            b.lost_nodes,
            b.lost_edges,
            b.lost_scopes,
            b.overflowed
        )?;
        for (i, n) in b.nodes.iter().enumerate() {
            if i != 0 {
                write!(out, ",")?;
            }
            write!(
                out,
                "{{\"id\":{},\"kind\":\"{}\",\"at_ns\":{},\"segment_id\":{},\"scope_id\":{},\"task\":{},\"outcome\":{},\"gap_reason\":{}}}",
                n.id,
                n.kind,
                n.at,
                num(n.segment),
                num(n.scope),
                num(n.task),
                string(n.outcome),
                string(n.gap)
            )?;
        }
        write!(out, "],\"edges\":[")?;
        for (i, e) in b.edges.iter().enumerate() {
            if i != 0 {
                write!(out, ",")?;
            }
            write!(
                out,
                "{{\"id\":{},\"from\":{},\"to\":{},\"kind\":\"{}\"}}",
                e.id, e.from, e.to, e.kind
            )?;
        }
        write!(out, "],\"scopes\":[")?;
        for (i, s) in b.scopes.iter().enumerate() {
            if i != 0 {
                write!(out, ",")?;
            }
            write!(
                out,
                "{{\"id\":{},\"query_context\":{},\"query_job\":{},\"query_name\":\"{}\",\"entry\":{},\"exit\":{},\"outcome\":\"{}\"}}",
                s.id,
                s.context,
                s.job,
                s.name,
                s.entry,
                num(s.exit),
                s.outcome
            )?;
        }
        write!(out, "]")?;
        if let Some(readiness) = &b.readiness {
            readiness.write_json(out, exhausted)?;
        }
        write!(out, "}}")
    }
}
pub(crate) struct ScopeGuard {
    recorder: Arc<Recorder>,
    scope: Option<u64>,
    row_index: Option<usize>,
    previous: Option<Cursor>,
}
impl Drop for ScopeGuard {
    fn drop(&mut self) {
        if self.row_index.is_none() {
            take();
            restore(cut(self.previous.take(), "missing_flow"));
            return;
        }
        let outcome = if std::thread::panicking() { "unwound" } else { "returned" };
        // Active invocation identity is explicit, never reconstructed from job nesting.
        if let Some(mut c) = take().filter(|c| Arc::ptr_eq(&c.recorder, &self.recorder)) {
            c.scope = self.scope;
            restore(Some(c));
        }
        let last = take();
        let (exit, exit_id) = self.recorder.node_attempt(
            "scope_exit",
            self.recorder.now(),
            None,
            self.scope,
            None,
            Some(outcome),
            None,
        );
        if let (Some(last), Some(exit)) = (&last, &exit) {
            if Arc::ptr_eq(&last.recorder, &self.recorder) {
                self.recorder.edge(last, exit, last.next);
            }
        }
        restore(exit.clone());
        if let Some(index) = self.row_index {
            let mut b = self.recorder.lock();
            if !b.sealed {
                if let Some(row) = b.scopes.get_mut(index).filter(|s| Some(s.id) == self.scope) {
                    row.exit = exit_id;
                    row.outcome = outcome;
                    b.active = b.active.saturating_sub(1);
                }
            }
        }
        let mut previous = self.previous.take();
        if exit.is_none() {
            restore(cut(previous, "missing_flow"));
            return;
        }
        if let (Some(parent), Some(exit)) = (&mut previous, exit) {
            if Arc::ptr_eq(&parent.recorder, &exit.recorder) {
                parent.tail = exit.tail;
                parent.next = "sync_return";
            } else {
                previous = cut(previous, "foreign_recorder");
            }
        }
        restore(previous);
    }
}
/// Save and restore exact caller lineage around an unobserved execution region.
/// The discarded region remains an independent root; never bridge its terminal.
pub(crate) fn cut(previous: Option<Cursor>, reason: &'static str) -> Option<Cursor> {
    let Some(previous) = previous else { return None };
    let recorder = Arc::clone(&previous.recorder);
    let scope = previous.scope;
    restore(Some(previous));
    let mut node = recorder.append("cut", recorder.now(), None, scope, None, None, Some(reason));
    if let Some(c) = &mut node {
        c.next = "continuation";
    }
    restore(None);
    node
}
pub(crate) fn branch(parent: &Option<Cursor>) -> Option<Cursor> {
    parent.clone().map(|mut c| {
        c.scope = None;
        c.next = "fork_branch";
        c
    })
}
pub(crate) fn join(
    recorder: &Arc<Recorder>,
    children: &[Option<Cursor>],
    caller: Option<Cursor>,
    task: u64,
) -> Option<Cursor> {
    restore(None);
    let scope =
        caller.as_ref().filter(|c| Arc::ptr_eq(&c.recorder, recorder)).and_then(|c| c.scope);
    let mut joined = recorder.node("join", recorder.now(), None, scope, Some(task), None, None);
    if let Some(joined) = &joined {
        for child in children.iter().flatten() {
            recorder.edge(child, joined, "join_required");
        }
    }
    if let Some(c) = &mut joined {
        c.next = "continuation";
    }
    if let Some(caller) = caller {
        if !Arc::ptr_eq(&caller.recorder, recorder) {
            return cut(Some(caller), "foreign_recorder");
        }
    }
    joined
}
fn num(n: Option<u64>) -> String {
    n.map_or_else(|| "null".into(), |n| n.to_string())
}
fn string(s: Option<&str>) -> String {
    s.map_or_else(|| "null".into(), |s| format!("\"{s}\""))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::execution_observer::{self as observed, ExecutionObserver, InterfaceOutcome};

    fn new_observer(capacity: usize) -> Arc<ExecutionObserver> {
        ExecutionObserver::new_for_query_with_execution_flow(capacity, "flow-test", Instant::now())
            .unwrap()
    }
    fn current_recorder() -> Arc<Recorder> {
        Arc::clone(&current().unwrap().recorder)
    }
    fn finish(observer: &ExecutionObserver) -> String {
        observer.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        observer.write_json(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }
    pub(crate) fn check_edges(recorder: &Recorder) {
        let b = recorder.lock();
        for edge in &b.edges {
            let source = b.nodes.iter().find(|n| n.id == edge.from).unwrap();
            let target = b.nodes.iter().find(|n| n.id == edge.to).unwrap();
            assert!(source.at <= target.at, "{} {} -> {}", edge.kind, source.kind, target.kind);
            if edge.kind == "sync_call" {
                assert_eq!(target.kind, "scope_enter");
                assert_ne!(source.kind, "scope_exit");
            }
            if edge.kind == "sync_return" {
                assert_eq!(source.kind, "scope_exit");
                assert_eq!(target.kind, "cpu_segment");
            }
            if edge.kind == "fork_branch" {
                assert_eq!(source.kind, "fork");
                assert_eq!(target.kind, "task_enter");
            }
            if edge.kind == "join_required" {
                assert_eq!(source.kind, "task_exit");
                assert_eq!(target.kind, "join");
            }
        }
        let mut segments: Vec<_> = b.nodes.iter().filter_map(|n| n.segment).collect();
        let count = segments.len();
        segments.sort_unstable();
        segments.dedup();
        assert_eq!(count, segments.len());
    }
    #[test]
    fn nested_repeated_job_invocations_have_fresh_scopes_and_exact_call_return() {
        let observer = new_observer(100);
        let root = observer.root().unwrap();
        let task = root.enter().unwrap();
        let recorder = current_recorder();
        {
            let _outer = observer.enter_query_execution(1, 1, "outer").unwrap();
            for _ in 0..2 {
                let _inner = observer.enter_query_execution(1, 2, "repeated").unwrap();
            }
            let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _inner = observer.enter_query_execution(1, 3, "unwound").unwrap();
                panic!("scope unwind");
            }));
            assert!(unwind.is_err());
        }
        drop(task);
        let json = finish(&observer);
        assert!(json.starts_with("{\"schema\":5,"));
        assert!(json.contains("\"timing\":{\"schema\":2,"));
        assert!(json.contains("\"execution_flow\":{\"schema\":1,"));
        assert_eq!(json.matches("\"records_complete\":true").count(), 2);
        check_edges(&recorder);
        let b = recorder.lock();
        assert_eq!(b.scopes.len(), 4);
        assert_eq!(b.active, 0);
        assert_ne!(b.scopes[1].id, b.scopes[2].id);
        assert_eq!(b.scopes[1].job, b.scopes[2].job);
        assert_eq!(b.scopes[3].outcome, "unwound");
        assert_eq!(b.edges.iter().filter(|e| e.kind == "sync_call").count(), 4);
        assert_eq!(b.edges.iter().filter(|e| e.kind == "sync_return").count(), 4);
    }
    #[test]
    fn masks_keep_helper_work_independent_and_restore_only_exact_caller() {
        let observer = new_observer(200);
        let outer = observer.enter_query_execution(1, 1, "outer").unwrap();
        let recorder = current_recorder();
        {
            let _dispatch = observed::suspend_for_dispatch();
            let _helper = observer.enter_query_execution(1, 2, "helper").unwrap();
        }
        {
            let _unknown = observed::suspend_query_execution().unwrap();
        }
        {
            let legacy =
                ExecutionObserver::new_for_query_with_cpu_ownership(20, "legacy", Instant::now())
                    .unwrap();
            let _legacy = legacy.enter_query_execution(2, 1, "legacy").unwrap();
        }
        {
            let foreign = new_observer(20);
            let query = foreign.enter_query_execution(2, 2, "foreign").unwrap();
            let foreign_recorder = current_recorder();
            drop(query);
            finish(&foreign);
            check_edges(&foreign_recorder);
            assert!(foreign_recorder.lock().edges.iter().all(|e| e.kind != "sync_call"));
        }
        drop(outer);
        finish(&observer);
        check_edges(&recorder);
        let b = recorder.lock();
        let outer_scope = b.scopes.iter().find(|s| s.name == "outer").unwrap().id;
        let helper = b.scopes.iter().find(|s| s.name == "helper").unwrap();
        assert!(
            b.edges.iter().filter(|e| e.to == helper.entry).all(|edge| {
                b.nodes.iter().find(|n| n.id == edge.from).unwrap().scope.is_none()
            })
        );
        assert!(b.nodes.iter().filter(|n| n.kind == "cut").count() >= 4);
        assert!(b.nodes.iter().filter(|n| n.kind == "cut").all(|n| n.scope == Some(outer_scope)));
    }
    #[test]
    fn nested_forks_join_actual_terminal_continuations_not_original_branch() {
        let observer = new_observer(200);
        let root = observer.root().unwrap();
        let task = root.enter().unwrap();
        let query = observer.enter_query_execution(1, 1, "outer").unwrap();
        let recorder = current_recorder();
        let fork = root.fork(2).unwrap();
        let nested_terminal;
        {
            let branch = &fork.children()[0];
            let _branch = branch.enter().unwrap();
            let nested = branch.fork(2).unwrap();
            for child in nested.children() {
                let _child = child.enter().unwrap();
            }
            nested_terminal = nested.resume().ok().unwrap().task;
        }
        {
            let _branch = fork.children()[1].enter().unwrap();
        }
        fork.resume().ok().unwrap();
        drop(query);
        drop(task);
        finish(&observer);
        check_edges(&recorder);
        let b = recorder.lock();
        assert_eq!(b.nodes.iter().filter(|n| n.kind == "fork").count(), 2);
        assert_eq!(b.nodes.iter().filter(|n| n.kind == "join").count(), 2);
        assert_eq!(b.edges.iter().filter(|e| e.kind == "fork_branch").count(), 4);
        assert_eq!(b.edges.iter().filter(|e| e.kind == "join_required").count(), 4);
        assert!(b.edges.iter().filter(|e| e.kind == "join_required").any(|edge| {
            b.nodes.iter().find(|n| n.id == edge.from).unwrap().task == Some(nested_terminal)
        }));
    }
    #[test]
    fn explicit_small_budget_bounds_native_flow_without_affecting_query_work() {
        let observer = new_observer(7);
        let root = observer.root().unwrap();
        let task = root.enter().unwrap();
        let recorder = current_recorder();
        assert_eq!(recorder.capacity, 28);
        let mut user_work = 0;
        for job in 1..=20 {
            let _query = observer.enter_query_execution(1, job, "bounded").unwrap();
            user_work += 1;
        }
        drop(task);
        let json = finish(&observer);
        assert_eq!(user_work, 20);
        let b = recorder.lock();
        assert_eq!(b.nodes.len(), 28);
        assert!(b.next_node > 29);
        assert!(b.lost_nodes > 0);
        assert_eq!(b.active, 0);
        assert!(b.nodes.iter().all(|n| n.at > 0));
        assert!(json.contains("\"records_complete\":false"));
    }
    #[test]
    fn independent_capacity_and_open_scope_inventory_never_claim_complete() {
        let recorder = Recorder::new(Instant::now(), 0);
        let scope = recorder.enter_scope(1, 1, "query");
        drop(scope);
        recorder.seal();
        let mut out = Vec::new();
        recorder.write_json(&mut out, false).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("\"lost_scopes\":1"));
        assert!(out.contains("\"records_complete\":false"));
        let recorder = Recorder::new(Instant::now(), 10);
        let scope = recorder.enter_scope(1, 1, "query");
        recorder.seal();
        drop(scope);
        let b = recorder.lock();
        assert_eq!(b.active, 1);
        assert_eq!(b.scopes[0].outcome, "active");
        assert!(b.lost_nodes > 0);
    }
    #[test]
    fn checked_allocators_refuse_wrap_and_loss_overflow_is_sticky() {
        let recorder = Recorder::new(Instant::now(), 0);
        {
            let mut b = recorder.lock();
            b.next_node = u64::MAX;
            b.next_scope = u64::MAX;
            b.lost_nodes = u64::MAX;
        }
        let scope = recorder.enter_scope(1, 1, "query");
        drop(scope);
        recorder.cpu(1, Some(1), None, true);
        let b = recorder.lock();
        assert!(b.exhausted);
        assert!(b.overflowed);
        assert_eq!(b.next_node, u64::MAX);
        assert_eq!(b.next_scope, u64::MAX);
    }
    #[test]
    fn terminal_node_loss_preserves_attempted_id_and_cuts_caller() {
        let recorder = Recorder::new(Instant::now(), 1);
        recorder.gate("task_enter", Some(1), None);
        let scope = recorder.enter_scope(1, 1, "query");
        recorder.cpu(1, recorder.now(), Some(1), true);
        recorder.cpu(2, recorder.now(), Some(1), true);
        drop(scope);
        assert!(current().is_none(), "full buffer cannot silently restore pre-call lineage");
        recorder.seal();
        let b = recorder.lock();
        assert_eq!(b.scopes[0].exit, Some(5));
        assert_eq!(b.scopes[0].outcome, "returned");
        assert_eq!(b.active, 0);
        assert_eq!(b.lost_nodes, 2);
        assert!(!b.nodes.iter().any(|n| n.kind == "scope_exit"));
        drop(b);
        let mut out = Vec::new();
        recorder.write_json(&mut out, false).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("\"exit\":5,\"outcome\":\"returned\""));
        assert!(out.contains("\"records_complete\":false"));
    }
    #[test]
    fn foreign_receiving_query_survives_pool_fork_as_independent_cut_continuation() {
        let pool = new_observer(200);
        let receiver = new_observer(200);
        let root = pool.root().unwrap();
        let task = root.enter().unwrap();
        let pool_recorder = current_recorder();
        let query = receiver.enter_query_execution(2, 1, "foreign_query").unwrap();
        let receiver_recorder = current_recorder();
        let fork = root.fork(2).unwrap();
        for child in fork.children() {
            let _child = child.enter().unwrap();
        }
        fork.resume().ok().unwrap();
        assert!(Arc::ptr_eq(&current_recorder(), &receiver_recorder));
        drop(query);
        drop(task);
        finish(&pool);
        finish(&receiver);
        check_edges(&pool_recorder);
        check_edges(&receiver_recorder);
        let b = receiver_recorder.lock();
        assert!(b.nodes.iter().any(|n| n.kind == "cut" && n.gap == Some("foreign_recorder")));
        assert!(
            !b.nodes.iter().any(|n| matches!(n.kind, "fork" | "join" | "task_enter" | "task_exit"))
        );
        assert!(
            b.nodes
                .iter()
                .filter(|n| n.kind == "cpu_segment")
                .all(|n| n.scope == Some(b.scopes[0].id))
        );
    }
    #[test]
    fn real_parallel_join_and_panic_preserve_explicit_branch_tokens() {
        for threads in [1, 2] {
            let observer = new_observer(500);
            let pool = crate::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            let saved = Arc::new(Mutex::new(None));
            let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pool.install(|| {
                    assert!(
                        crate::Registry::current().attach_execution_observer(Arc::clone(&observer))
                    );
                    let _query = observer.enter_query_execution(1, 1, "parallel_query").unwrap();
                    *saved.lock().unwrap() = Some(current_recorder());
                    crate::join(
                        || crate::join(|| std::hint::black_box(1), || std::hint::black_box(2)),
                        || panic!("branch panic"),
                    );
                })
            }));
            assert!(unwind.is_err());
            observer.seal(InterfaceOutcome::Unwound);
            let recorder = saved.lock().unwrap().take().unwrap();
            check_edges(&recorder);
            let b = recorder.lock();
            assert!(b.nodes.iter().any(|n| n.kind == "task_exit" && n.outcome == Some("unwound")));
            assert!(b.nodes.iter().any(|n| n.kind == "join"));
            assert_eq!(b.active, 0);
        }
    }
    #[test]
    fn failed_tls_borrow_is_isolated_sticky_and_does_not_change_user_work() {
        const CHILD: &str = "JR_EXECUTION_FLOW_TLS_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "execution_flow::tests::failed_tls_borrow_is_isolated_sticky_and_does_not_change_user_work", "--nocapture"])
                .env(CHILD, "1").output().unwrap();
            assert!(
                output.status.success(),
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("flow TLS child verified"));
            return;
        }
        let mut user_work = 0;
        CURRENT.with(|slot| {
            let _held = slot.borrow_mut();
            assert!(take().is_none());
            restore(None);
            assert!(current().is_none());
            user_work += 1;
        });
        assert_eq!(user_work, 1);
        assert!(TLS_FAILED.load(Ordering::Relaxed));
        let recorder = Recorder::new(Instant::now(), 1);
        recorder.seal();
        let mut out = Vec::new();
        recorder.write_json(&mut out, false).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("\"tls_failed\":true"));
        assert!(out.contains("\"records_complete\":false"));
        println!("flow TLS child verified");
    }
}
