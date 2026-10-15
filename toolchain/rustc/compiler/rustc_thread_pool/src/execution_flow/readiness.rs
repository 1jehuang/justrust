//! Latch provenance is a qualified transition witness, never a publication clock.
//! All recorder/TLS/clock operations run outside the native latch lock.
use super::*;
use std::marker::PhantomData;
use std::rc::Rc;

#[derive(Clone, Copy, Debug)]
pub struct LatchReadinessToken {
    namespace: u64,
    id: u64,
    row: Option<usize>,
    context: u64,
    job: u64,
    prerequisite: Option<u64>,
    visibility: Option<u64>,
}
struct Ready {
    id: u64,
    context: u64,
    job: u64,
    prefix: Option<u64>,
    prerequisite: Option<u64>,
    visibility: Option<u64>,
    outcome: &'static str,
}
struct Wait {
    id: u64,
    context: u64,
    waitee: u64,
    consumer: Option<u64>,
    scope: Option<u64>,
    wait: Option<u64>,
    mode: &'static str,
    outcome: &'static str,
    begin: Option<u64>,
    gap: Option<u64>,
    ready: Option<u64>,
    wake: Option<u64>,
    consumed: Option<u64>,
}
pub(super) struct Inventory {
    capacity: usize,
    ready: Vec<Ready>,
    waits: Vec<Wait>,
    next_ready: u64,
    next_wait: u64,
    lost_ready: u64,
    lost_wait: u64,
    active_ready: u64,
    active_wait: u64,
}
impl Inventory {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            ready: Vec::new(),
            waits: Vec::new(),
            next_ready: 1,
            next_wait: 1,
            lost_ready: 0,
            lost_wait: 0,
            active_ready: 0,
            active_wait: 0,
        }
    }
    pub(super) fn complete(&self) -> bool {
        self.active_ready == 0
            && self.active_wait == 0
            && self.lost_ready == 0
            && self.lost_wait == 0
            && self.ready.len() as u64 == self.next_ready - 1
            && self.waits.len() as u64 == self.next_wait - 1
    }
    pub(super) fn write_json(
        &self,
        out: &mut impl std::io::Write,
        exhausted: bool,
    ) -> std::io::Result<()> {
        write!(
            out,
            ",\"readiness_allocator_baseline\":1,\"readiness_allocator_high_water\":{},\"expected_readiness\":{},\"lost_readiness\":{},\"active_readiness\":{},\"wait_attempt_allocator_baseline\":1,\"wait_attempt_allocator_high_water\":{},\"expected_wait_attempts\":{},\"lost_wait_attempts\":{},\"active_wait_attempts\":{},\"readiness\":[",
            num((!exhausted).then_some(self.next_ready)),
            num((!exhausted).then_some(self.next_ready - 1)),
            self.lost_ready,
            self.active_ready,
            num((!exhausted).then_some(self.next_wait)),
            num((!exhausted).then_some(self.next_wait - 1)),
            self.lost_wait,
            self.active_wait
        )?;
        for (i, r) in self.ready.iter().enumerate() {
            if i != 0 {
                write!(out, ",")?;
            }
            write!(
                out,
                "{{\"id\":{},\"query_context\":{},\"query_job\":{},\"prefix\":{},\"prerequisite\":{},\"visibility_gap\":{},\"outcome\":\"{}\"}}",
                r.id,
                r.context,
                r.job,
                num(r.prefix),
                num(r.prerequisite),
                num(r.visibility),
                r.outcome
            )?;
        }
        write!(out, "],\"wait_attempts\":[")?;
        for (i, w) in self.waits.iter().enumerate() {
            if i != 0 {
                write!(out, ",")?;
            }
            write!(
                out,
                "{{\"id\":{},\"query_context\":{},\"waitee_job\":{},\"consumer_job\":{},\"caller_scope\":{},\"wait_id\":{},\"mode\":\"{}\",\"outcome\":\"{}\",\"begin\":{},\"wait_gap\":{},\"readiness\":{},\"wake_gap\":{},\"consumed\":{}}}",
                w.id,
                w.context,
                w.waitee,
                num(w.consumer),
                num(w.scope),
                num(w.wait),
                w.mode,
                w.outcome,
                num(w.begin),
                num(w.gap),
                num(w.ready),
                num(w.wake),
                num(w.consumed)
            )?;
        }
        write!(out, "]")
    }
}
impl Recorder {
    fn parked(previous: &Option<Cursor>, reason: &'static str) -> Option<Cursor> {
        let previous = previous.as_ref()?;
        let recorder = &previous.recorder;
        let mut root =
            recorder.node("cut", recorder.now(), None, previous.scope, None, None, Some(reason));
        if let Some(root) = &mut root {
            root.next = "continuation";
        }
        root
    }
    fn missing_readiness_node(&self) -> Option<u64> {
        let mut b = self.lock();
        let Buffer { next_node, exhausted, lost_nodes, overflowed, .. } = &mut *b;
        let allocated = id(next_node, exhausted);
        loss(lost_nodes, overflowed);
        allocated
    }
    fn ready_row(
        self: &Arc<Self>,
        context: u64,
        job: u64,
        prefix: Option<u64>,
        prerequisite: Option<u64>,
        visibility: Option<u64>,
    ) -> (Option<usize>, Option<u64>) {
        let mut b = self.lock();
        let Buffer { readiness, exhausted, overflowed, sealed, .. } = &mut *b;
        let Some(r) = readiness else { return (None, None) };
        let allocated = id(&mut r.next_ready, exhausted);
        if *sealed
            || allocated.is_none()
            || r.ready.len() >= r.capacity
            || r.ready.try_reserve(1).is_err()
        {
            loss(&mut r.lost_ready, overflowed);
            return (None, allocated);
        }
        let index = r.ready.len();
        r.ready.push(Ready {
            id: allocated.unwrap(),
            context,
            job,
            prefix,
            prerequisite,
            visibility,
            outcome: "active",
        });
        r.active_ready = r.active_ready.checked_add(1).unwrap_or_else(|| {
            *exhausted = true;
            u64::MAX
        });
        (Some(index), allocated)
    }
    pub(crate) fn prepare_readiness(
        self: &Arc<Self>,
        context: u64,
        job: u64,
    ) -> ReadinessPreparation {
        // Producer scope may have exited already. Producer authority is explicit admission,
        // never the restored caller scope currently occupying TLS.
        let previous = take();
        let saved = previous.as_ref().filter(|c| Arc::ptr_eq(&c.recorder, self)).map(|c| c.tail);
        let (index, allocated) = self.ready_row(context, job, saved, None, None);
        let (prefix, prefix_id) = if index.is_some() {
            self.node_attempt("readiness_prerequisite", self.now(), None, None, None, None, None)
        } else {
            (None, self.missing_readiness_node())
        };
        if let (Some(a), Some(z)) = (&previous, &prefix) {
            if Arc::ptr_eq(&a.recorder, self) {
                self.edge(a, z, "readiness_prepare");
            }
        }
        let (visibility, visibility_id) = if index.is_some() {
            self.node_attempt("cut", self.now(), None, None, None, None, Some("latch_visibility"))
        } else {
            (None, self.missing_readiness_node())
        };
        if let (Some(a), Some(z)) = (&prefix, &visibility) {
            self.edge(a, z, "strand");
        }
        if let Some(index) = index {
            let mut b = self.lock();
            let row = &mut b.readiness.as_mut().unwrap().ready[index];
            row.prerequisite = prefix_id;
            row.visibility = visibility_id;
        }
        let token = allocated.zip(self.namespace).map(|(id, namespace)| LatchReadinessToken {
            namespace,
            id,
            row: index,
            context,
            job,
            prerequisite: prefix_id,
            visibility: visibility_id,
        });
        // A diagnostic side branch does not consume a pending lexical return.
        restore(previous);
        ReadinessPreparation {
            recorder: Arc::clone(self),
            index,
            token,
            terminal: false,
            _not_send: PhantomData,
        }
    }
    pub(crate) fn begin_wait(
        self: &Arc<Self>,
        context: u64,
        waitee: u64,
        consumer: Option<u64>,
        task: Option<u64>,
    ) -> LatchWaitGuard {
        let previous = take();
        let scope =
            previous.as_ref().filter(|c| Arc::ptr_eq(&c.recorder, self)).and_then(|c| c.scope);
        let index = {
            let mut b = self.lock();
            let Buffer { readiness, exhausted, overflowed, sealed, .. } = &mut *b;
            let r = readiness.as_mut().unwrap();
            let allocated = id(&mut r.next_wait, exhausted);
            if *sealed
                || allocated.is_none()
                || r.waits.len() >= r.capacity
                || r.waits.try_reserve(1).is_err()
            {
                loss(&mut r.lost_wait, overflowed);
                None
            } else {
                let index = r.waits.len();
                r.waits.push(Wait {
                    id: allocated.unwrap(),
                    context,
                    waitee,
                    consumer,
                    scope,
                    wait: None,
                    mode: "active",
                    outcome: "active",
                    begin: None,
                    gap: None,
                    ready: None,
                    wake: None,
                    consumed: None,
                });
                r.active_wait = r.active_wait.checked_add(1).unwrap_or_else(|| {
                    *exhausted = true;
                    u64::MAX
                });
                Some(index)
            }
        };
        let (begin, begin_id) = if index.is_some() {
            self.node_attempt("wait_begin", self.now(), None, scope, task, None, None)
        } else {
            (None, self.missing_readiness_node())
        };
        if let (Some(a), Some(z)) = (&previous, &begin) {
            if Arc::ptr_eq(&a.recorder, self) {
                self.edge(a, z, a.next);
            }
        }
        let (gap, gap_id) = if index.is_some() {
            self.node_attempt("cut", self.now(), None, scope, None, None, Some("query_wait"))
        } else {
            (None, self.missing_readiness_node())
        };
        if let Some(index) = index {
            let mut b = self.lock();
            let row = &mut b.readiness.as_mut().unwrap().waits[index];
            row.begin = begin_id;
            row.gap = gap_id;
        }
        if previous.as_ref().is_some_and(|p| !Arc::ptr_eq(&p.recorder, self)) {
            restore(Self::parked(&previous, "foreign_recorder"));
        } else if index.is_none() {
            restore(Self::parked(&previous, "missing_flow"));
        } else {
            restore(gap);
        }
        LatchWaitGuard {
            recorder: Arc::clone(self),
            index,
            begin,
            previous,
            scope,
            context,
            waitee,
            consumer,
            mode: "active",
            wait: None,
            token: None,
            terminal: false,
            _not_send: PhantomData,
        }
    }
}
/// Prepared outside latch lock. Only its Copy token may be moved under that lock.
pub struct ReadinessPreparation {
    recorder: Arc<Recorder>,
    index: Option<usize>,
    token: Option<LatchReadinessToken>,
    terminal: bool,
    _not_send: PhantomData<Rc<()>>,
}
impl ReadinessPreparation {
    pub fn token(&self) -> Option<LatchReadinessToken> {
        self.token
    }
    pub fn finish(mut self, published: bool) {
        self.terminal = true;
        self.finish_row(if published { "published" } else { "poisoned" });
    }
    fn finish_row(&self, outcome: &'static str) {
        let mut b = self.recorder.lock();
        if b.sealed {
            return;
        }
        if let (Some(index), Some(r)) = (self.index, b.readiness.as_mut()) {
            if let Some(row) = r.ready.get_mut(index) {
                row.outcome = outcome;
                r.active_ready = r.active_ready.saturating_sub(1);
            }
        }
    }
}
impl Drop for ReadinessPreparation {
    fn drop(&mut self) {
        if !self.terminal {
            self.finish_row("aborted");
        }
    }
}
/// Saved lexical caller, deliberately independent of helping worker/task tails.
pub struct LatchWaitGuard {
    recorder: Arc<Recorder>,
    index: Option<usize>,
    begin: Option<Cursor>,
    previous: Option<Cursor>,
    scope: Option<u64>,
    context: u64,
    waitee: u64,
    consumer: Option<u64>,
    mode: &'static str,
    wait: Option<u64>,
    token: Option<LatchReadinessToken>,
    terminal: bool,
    _not_send: PhantomData<Rc<()>>,
}
impl LatchWaitGuard {
    /// Called after releasing latch lock and reacquiring jobserver allowance.
    pub fn observed(
        &mut self,
        registered: bool,
        wait: Option<u64>,
        token: Option<LatchReadinessToken>,
    ) {
        self.mode = if registered { "registered" } else { "already_ready" };
        self.wait = wait;
        self.token = token;
        let mut b = self.recorder.lock();
        if b.sealed {
            return;
        }
        if let (Some(index), Some(r)) = (self.index, b.readiness.as_mut()) {
            let row = &mut r.waits[index];
            row.mode = self.mode;
            row.wait = wait;
        }
    }
    pub fn consumed(mut self) {
        self.finish("consumed");
    }
    pub fn failed(mut self, reason: &'static str) {
        self.finish(if reason == "cycle" { "cycle" } else { "poisoned" });
    }
    fn finish(&mut self, requested: &'static str) {
        let timing = crate::execution_timing::pause();
        let task =
            crate::execution_observer::ExecutionObserver::readiness_task(self.recorder.namespace);
        take(); // discard the independent wait/help stream
        let matching = self.token.filter(|t| {
            Some(t.namespace) == self.recorder.namespace
                && t.context == self.context
                && t.job == self.waitee
        });
        let published = matching.is_some_and(|token| {
            self.recorder.lock().readiness.as_ref().is_some_and(|r| {
                token.row.and_then(|index| r.ready.get(index)).is_some_and(|row| {
                    row.id == token.id
                        && row.context == token.context
                        && row.job == token.job
                        && row.prerequisite == token.prerequisite
                        && row.visibility == token.visibility
                        && row.outcome == "published"
                })
            })
        });
        let authorized = matching.filter(|_| {
            published
                && self.consumer.is_some()
                && self.scope.is_some()
                && self.index.is_some()
                && (self.mode == "already_ready" || self.wait.is_some())
        });
        let success = requested == "consumed" && authorized.is_some();
        let outcome =
            if requested == "consumed" && !success { "consumed_unbound" } else { requested };
        let reason = match outcome {
            "consumed" => "wake_to_consume",
            "cycle" => "wait_cycle",
            "poisoned" => "wait_poisoned",
            "unwound" => "wait_unwound",
            _ => "readiness_unavailable",
        };
        let (wake, wake_id) = if self.index.is_some() {
            self.recorder.node_attempt(
                "cut",
                self.recorder.now(),
                None,
                self.scope,
                task,
                None,
                Some(reason),
            )
        } else {
            (None, self.recorder.missing_readiness_node())
        };
        if let (Some(begin), Some(wake)) = (&self.begin, &wake) {
            self.recorder.edge(begin, wake, "wait_boundary");
        }
        let mut consumed_id = None;
        let mut continuation = wake.clone();
        if success {
            let (consumed, allocated) = self.recorder.node_attempt(
                "wait_consumed",
                self.recorder.now(),
                None,
                self.scope,
                task,
                None,
                None,
            );
            consumed_id = allocated;
            if let (Some(wake), Some(consumed)) = (&wake, &consumed) {
                self.recorder.edge(wake, consumed, "wait_required");
            }
            if let (Some(token), Some(consumed)) = (authorized, &consumed) {
                if let Some(tail) = token.visibility {
                    let source = Cursor {
                        recorder: Arc::clone(&self.recorder),
                        tail,
                        scope: None,
                        next: "strand",
                    };
                    self.recorder.edge(&source, consumed, "readiness_required");
                }
            }
            continuation = consumed;
        }
        {
            let mut b = self.recorder.lock();
            if !b.sealed {
                if let (Some(index), Some(r)) = (self.index, b.readiness.as_mut()) {
                    let row = &mut r.waits[index];
                    row.mode = self.mode;
                    row.wait = self.wait;
                    row.outcome = outcome;
                    row.ready = matching.map(|t| t.id);
                    row.wake = wake_id;
                    row.consumed = consumed_id;
                    r.active_wait = r.active_wait.saturating_sub(1);
                }
            }
        }
        if let Some(c) = &mut continuation {
            c.next = "continuation";
        }
        if self.previous.as_ref().is_some_and(|p| !Arc::ptr_eq(&p.recorder, &self.recorder)) {
            continuation = cut(self.previous.take(), "foreign_recorder");
        } else if continuation.is_none() {
            continuation = cut(self.previous.take(), "missing_flow");
        }
        restore(continuation);
        self.terminal = true;
        crate::execution_observer::resume_current_timing(timing.as_ref());
    }
}
impl Drop for LatchWaitGuard {
    fn drop(&mut self) {
        if !self.terminal {
            self.finish("unwound");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_observer::{ExecutionObserver, InterfaceOutcome};

    fn new_observer() -> Arc<ExecutionObserver> {
        ExecutionObserver::new_for_query_with_latch_readiness(256, "ready-test", Instant::now())
            .unwrap()
    }
    fn recorder(observer: &ExecutionObserver) -> Arc<Recorder> {
        Arc::clone(observer.flow().unwrap())
    }
    fn finish(observer: &ExecutionObserver) -> String {
        observer.seal(InterfaceOutcome::Returned);
        let mut out = Vec::new();
        observer.write_json(&mut out).unwrap();
        let packet = String::from_utf8(out).unwrap();
        // Explicit test-only export. No compiler controls, production environment,
        // or accepted runtime/private metadata are changed by this harness hook.
        if let Some(directory) = std::env::var_os("JR_READY_NATIVE_PACKETS") {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("native-readiness").replace("::", "_");
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            std::fs::write(
                std::path::Path::new(&directory).join(format!("{name}-{sequence}.pool.json")),
                &packet,
            )
            .unwrap();
        }
        packet
    }
    fn assert_ordinary_degrees(r: &Recorder) {
        let b = r.lock();
        for n in &b.nodes {
            let ordinary = b
                .edges
                .iter()
                .filter(|e| {
                    e.from == n.id
                        && e.kind != "readiness_prepare"
                        && e.kind != "readiness_required"
                })
                .count();
            if n.kind != "fork" {
                assert!(
                    ordinary <= 1,
                    "node {} {} has {} ordinary successors",
                    n.id,
                    n.kind,
                    ordinary
                );
            }
        }
    }
    #[test]
    fn detached_producer_preserves_exact_pending_return_and_namespaces() {
        let observer = new_observer();
        let r = recorder(&observer);
        let caller = observer.enter_query_execution(1, 1, "caller").unwrap();
        let child = r.enter_scope(1, 2, "producer");
        drop(child); // deliberately preserve a pending sync_return, without CPU resumption
        let before = current().unwrap();
        assert_eq!(before.next, "sync_return");
        let preparation = r.prepare_readiness(1, 2);
        let after = current().unwrap();
        assert_eq!((before.tail, before.scope, before.next), (after.tail, after.scope, after.next));
        preparation.finish(true);
        crate::execution_observer::resume_current_timing(None);
        drop(caller);
        let b = r.lock();
        let ready = &b.readiness.as_ref().unwrap().ready[0];
        assert_eq!(ready.prefix, Some(before.tail));
        assert!(b.edges.iter().any(|e| e.kind == "readiness_prepare"
            && e.from == before.tail
            && Some(e.to) == ready.prerequisite));
        assert!(
            !b.edges.iter().any(|e| Some(e.from) == ready.visibility && e.kind == "continuation")
        );
        drop(b);
        assert!(finish(&observer).starts_with("{\"schema\":6,"));
    }
    #[test]
    fn registered_and_already_ready_have_fresh_encounters_and_two_exact_obligations() {
        let observer = new_observer();
        let r = recorder(&observer);
        let caller = observer.enter_query_execution(1, 1, "caller").unwrap();
        let prep = observer.prepare_latch_readiness(1, 2).unwrap();
        let token = prep.token();
        prep.finish(true);
        for (registered, wait) in [(true, Some(41)), (false, None), (true, Some(42))] {
            let mut guard = observer.begin_latch_wait(1, 2, Some(1)).unwrap();
            guard.observed(registered, wait, token);
            guard.consumed();
        }
        drop(caller);
        let b = r.lock();
        let inv = b.readiness.as_ref().unwrap();
        assert_eq!(inv.waits.len(), 3);
        for w in &inv.waits {
            assert_eq!(w.outcome, "consumed");
            assert_eq!(w.scope, Some(1));
            let incoming: Vec<_> = b.edges.iter().filter(|e| Some(e.to) == w.consumed).collect();
            assert_eq!(incoming.len(), 2);
            assert!(incoming.iter().any(|e| e.kind == "wait_required" && Some(e.from) == w.wake));
            assert!(
                incoming
                    .iter()
                    .any(|e| e.kind == "readiness_required"
                        && Some(e.from) == inv.ready[0].visibility)
            );
            assert!(!b.edges.iter().any(|e| Some(e.to) == w.gap));
        }
        assert_eq!(inv.active_wait, 0);
        drop(b);
        let json = finish(&observer);
        assert!(json.contains("\"execution_flow\":{\"schema\":2,"));
        assert!(json.contains("\"wait_id\":41,\"mode\":\"registered\""));
    }
    #[test]
    fn active_aborted_poisoned_foreign_and_original_wait_exhaustion_never_transfer() {
        let observer = new_observer();
        let foreign = new_observer();
        let r = recorder(&observer);
        let caller = observer.enter_query_execution(1, 1, "caller").unwrap();
        let active = observer.prepare_latch_readiness(1, 2).unwrap();
        let active_token = active.token();
        let mut w = observer.begin_latch_wait(1, 2, Some(1)).unwrap();
        w.observed(false, None, active_token);
        w.consumed();
        drop(active);
        let poison = observer.prepare_latch_readiness(1, 2).unwrap();
        let poison_token = poison.token();
        poison.finish(false);
        let foreign_prep = foreign.prepare_latch_readiness(1, 2).unwrap();
        let foreign_token = foreign_prep.token();
        foreign_prep.finish(true);
        let good = observer.prepare_latch_readiness(1, 2).unwrap();
        let good_token = good.token();
        good.finish(true);
        for (token, registered) in [
            (active_token, false),
            (poison_token, false),
            (foreign_token, false),
            (good_token, true),
        ] {
            let mut w = observer.begin_latch_wait(1, 2, Some(1)).unwrap();
            w.observed(registered, None, token);
            w.consumed();
        }
        drop(caller);
        let b = r.lock();
        let inv = b.readiness.as_ref().unwrap();
        assert!(inv.waits.iter().all(|w| w.outcome == "consumed_unbound" && w.consumed.is_none()));
        assert!(
            !b.edges.iter().any(|e| e.kind == "readiness_required" || e.kind == "wait_required")
        );
        assert_eq!(inv.waits.last().unwrap().mode, "registered");
        drop(b);
        finish(&observer);
    }
    #[test]
    fn cycle_poison_and_unwind_restore_caller_with_explicit_cuts() {
        let observer = new_observer();
        let r = recorder(&observer);
        let caller = observer.enter_query_execution(1, 1, "caller").unwrap();
        for (index, outcome) in ["cycle", "poisoned", "unwound"].into_iter().enumerate() {
            let mut w = observer.begin_latch_wait(1, 2, Some(1)).unwrap();
            w.observed(true, Some(7 + index as u64), None);
            if outcome == "unwound" {
                drop(w);
            } else {
                w.failed(outcome);
            }
            assert_eq!(current().unwrap().scope, Some(1));
        }
        // Unwind may occur before actual latch registration/observation returns.
        drop(observer.begin_latch_wait(1, 2, Some(1)).unwrap());
        drop(caller);
        let b = r.lock();
        let inv = b.readiness.as_ref().unwrap();
        assert_eq!(
            inv.waits.iter().map(|w| w.outcome).collect::<Vec<_>>(),
            vec!["cycle", "poisoned", "unwound", "unwound"]
        );
        assert_eq!(inv.waits[3].mode, "active");
        assert!(inv.waits[3].wait.is_none());
        assert!(inv.waits.iter().all(|w| w.consumed.is_none()));
        assert_eq!(inv.active_wait, 0);
        drop(b);
        finish(&observer);
    }
    #[test]
    fn bounded_inventory_and_allocator_failures_are_honest_and_nonwrapping() {
        let r = Recorder::new_with_readiness(Instant::now(), 0, 777);
        let prep = r.prepare_readiness(1, 2);
        prep.finish(true);
        let mut wait = r.begin_wait(1, 2, Some(1), None);
        wait.observed(true, None, None);
        wait.consumed();
        let b = r.lock();
        let inv = b.readiness.as_ref().unwrap();
        assert_eq!((inv.lost_ready, inv.lost_wait), (1, 1));
        assert!(!inv.complete());
        drop(b);
        {
            let mut b = r.lock();
            let inv = b.readiness.as_mut().unwrap();
            inv.next_ready = u64::MAX;
            inv.next_wait = u64::MAX;
        }
        drop(r.prepare_readiness(1, 2));
        drop(r.begin_wait(1, 2, Some(1), None));
        let b = r.lock();
        assert!(b.exhausted);
        let inv = b.readiness.as_ref().unwrap();
        assert_eq!(inv.next_ready, u64::MAX);
        assert_eq!(inv.next_wait, u64::MAX);
        drop(b);
        r.seal();
        let mut out = Vec::new();
        r.write_json(&mut out, false).unwrap();
        let json = String::from_utf8(out).unwrap();
        assert!(json.contains("\"readiness_allocator_high_water\":null"));
    }
    #[test]
    fn legacy_constructors_do_not_enable_readiness() {
        let observer =
            ExecutionObserver::new_for_query_with_execution_flow(32, "legacy", Instant::now())
                .unwrap();
        assert!(observer.prepare_latch_readiness(1, 2).is_none());
        assert!(observer.begin_latch_wait(1, 2, Some(1)).is_none());
        let json = finish(&observer);
        assert!(json.starts_with("{\"schema\":5,"));
        assert!(!json.contains("readiness_allocator"));
    }

    #[test]
    fn waiter_beating_post_unlock_publication_finalization_is_explicitly_unbound() {
        let observer = new_observer();
        let r = recorder(&observer);
        let caller = observer.enter_query_execution(1, 1, "caller").unwrap();
        let preparation = observer.prepare_latch_readiness(1, 2).unwrap();
        let token = preparation.token();
        for registered in [true, false] {
            let mut wait = observer.begin_latch_wait(1, 2, Some(1)).unwrap();
            wait.observed(registered, registered.then_some(13), token);
            wait.consumed();
        }
        // The successful latch transition was real, but its outside-lock recorder
        // finalization lost the race. Never wait for it or trust an active row.
        preparation.finish(true);
        drop(caller);
        let b = r.lock();
        let inv = b.readiness.as_ref().unwrap();
        assert_eq!(inv.ready[0].outcome, "published");
        assert_eq!(inv.waits.len(), 2);
        assert!(
            inv.waits
                .iter()
                .all(|wait| wait.outcome == "consumed_unbound" && wait.consumed.is_none())
        );
        assert!(!b.edges.iter().any(|e| e.kind == "readiness_required"));
        drop(b);
        finish(&observer);
    }

    #[test]
    fn foreign_receiver_restores_actual_query_flow_with_and_without_genuine_task() {
        for task_owned in [false, true] {
            let a = new_observer();
            let b = new_observer();
            let r = recorder(&a);
            let task = task_owned.then(|| a.root().unwrap().enter().unwrap());
            let query = a.enter_query_execution(1, 1, "actual_caller").unwrap();
            let prep = b.prepare_latch_readiness(1, 2).unwrap();
            let token = prep.token();
            prep.finish(true);
            let mut wait = b.begin_latch_wait(1, 2, None).unwrap();
            wait.observed(false, None, token);
            assert!(Arc::ptr_eq(&current().unwrap().recorder, &r));
            assert_eq!(current().unwrap().scope, Some(1));
            wait.consumed();
            let resumed = current().unwrap();
            assert!(Arc::ptr_eq(&resumed.recorder, &r));
            assert_eq!(resumed.scope, Some(1));
            drop(query);
            drop(task);
            let ab = r.lock();
            assert!(
                ab.nodes.iter().filter(|n| n.kind == "cpu_segment" && n.scope == Some(1)).count()
                    >= 3
            );
            drop(ab);
            let br = recorder(&b);
            let bb = br.lock();
            assert!(!bb.edges.iter().any(|e| e.kind == "readiness_required"));
            drop(bb);
            assert_ordinary_degrees(&r);
            let packet = finish(&a);
            if let Some(directory) = std::env::var_os("JR_READY_NATIVE_PACKETS") {
                std::fs::write(
                    std::path::Path::new(&directory)
                        .join(format!("foreign-{task_owned}.pool.json")),
                    packet,
                )
                .unwrap();
            }
            finish(&b);
        }
    }

    #[test]
    fn independent_row_capacity_loss_does_not_emit_orphan_typed_gates_or_fork_caller() {
        let observer = new_observer();
        let r = recorder(&observer);
        let caller = observer.enter_query_execution(1, 1, "caller").unwrap();
        r.lock().readiness.as_mut().unwrap().capacity = 0;
        let prep = observer.prepare_latch_readiness(1, 2).unwrap();
        prep.finish(true);
        let mut wait = observer.begin_latch_wait(1, 2, Some(1)).unwrap();
        wait.observed(true, Some(4), None);
        wait.consumed();
        drop(caller);
        assert_ordinary_degrees(&r);
        {
            let b = r.lock();
            assert!(!b.nodes.iter().any(|n| matches!(
                n.kind,
                "readiness_prerequisite" | "wait_begin" | "wait_consumed"
            )));
            let inv = b.readiness.as_ref().unwrap();
            assert_eq!((inv.lost_ready, inv.lost_wait), (1, 1));
            assert!(b.lost_nodes >= 5);
        }
        let packet = finish(&observer);
        if let Some(directory) = std::env::var_os("JR_READY_NATIVE_PACKETS") {
            std::fs::write(
                std::path::Path::new(&directory).join("row-capacity-loss.pool.json"),
                packet,
            )
            .unwrap();
        }
    }
}
