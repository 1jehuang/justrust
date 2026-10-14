//! Experimental targeted admission, not connected to the compiler pipeline.
//! Sources name release operations, not holders or physical tokens.
use super::{Client, HelperThread, client};
use parking_lot::{Condvar, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantSource {
    IdleImplicit,
    Release(Option<u64>),
    Helper(Option<u64>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Requested { ticket: Option<u64> },
    Pending { ticket: Option<u64> },
    Granted { ticket: Option<u64>, source: GrantSource },
    Ack { ticket: Option<u64>, source: GrantSource },
    Released { operation: Option<u64>, transferred: bool },
    HelperSupply { callback: Option<u64>, granted: bool },
    HelperError,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub requests: u64,
    pub grants: u64,
    pub acks: u64,
    pub releases: u64,
    pub supplies: u64,
    pub callbacks_completed: u64,
    pub helper_errors: u64,
    pub lost_events: u64,
    pub overflow: bool,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub used: usize,
    pub pending: usize,
    pub inventory: Inventory,
    pub events: Vec<Transition>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Pending,
    Granted(GrantSource),
    Ack,
}
struct Slot {
    ticket: Option<u64>,
    next: Mutex<Option<Arc<Slot>>>,
    // Storage synchronization only. Every phase access additionally holds State,
    // and the private CV waits on State, never on this subordinate mutex.
    phase: Mutex<Phase>,
    wake: Condvar,
}
struct State {
    // Includes reserved grants before acknowledgment, with initial implicit use.
    used: usize,
    pending: usize,
    head: Option<Arc<Slot>>,
    tail: Option<Arc<Slot>>,
    next_ticket: Option<u64>,
    next_release: Option<u64>,
    next_callback: Option<u64>,
    inventory: Inventory,
    events: Vec<Transition>,
    capture_events: bool,
    event_cap: usize,
}
fn increment(value: &mut u64, overflow: &mut bool) {
    if let Some(next) = value.checked_add(1) {
        *value = next;
    } else {
        *overflow = true;
    }
}
impl State {
    fn identity(&mut self, kind: usize) -> Option<u64> {
        let next = match kind {
            0 => &mut self.next_ticket,
            1 => &mut self.next_release,
            _ => &mut self.next_callback,
        };
        let current = *next;
        *next = current.and_then(|n| n.checked_add(1));
        if next.is_none() {
            self.inventory.overflow = true;
        }
        current
    }
    fn push(&mut self, slot: Arc<Slot>) {
        self.pending = self.pending.checked_add(1).expect("pending count overflow");
        if let Some(tail) = self.tail.replace(Arc::clone(&slot)) {
            *tail.next.lock() = Some(slot);
        } else {
            self.head = Some(slot);
        }
    }
    fn pop(&mut self) -> Option<Arc<Slot>> {
        self.head.as_ref()?;
        let pending = self.pending.checked_sub(1).expect("queue count underflow");
        let slot = self.head.take().unwrap();
        self.head = slot.next.lock().take();
        self.pending = pending;
        if self.head.is_none() {
            self.tail = None;
        }
        Some(slot)
    }
    fn record(&mut self, event: Transition) {
        if !self.capture_events {
            return;
        }
        // Fixed preallocated capacity. Loss never controls admission.
        if self.events.len() < self.event_cap {
            self.events.push(event);
        } else {
            increment(&mut self.inventory.lost_events, &mut self.inventory.overflow);
        }
    }
    fn grant(&mut self, slot: &Slot, source: GrantSource) {
        let mut phase = slot.phase.lock();
        assert_eq!(*phase, Phase::Pending);
        *phase = Phase::Granted(source);
        increment(&mut self.inventory.grants, &mut self.inventory.overflow);
        self.record(Transition::Granted { ticket: slot.ticket, source });
        slot.wake.notify_one();
    }
    fn error(&mut self) {
        increment(&mut self.inventory.helper_errors, &mut self.inventory.overflow);
        self.record(Transition::HelperError);
    }
}
#[cfg(test)]
type TestHook = Mutex<Option<Arc<dyn Fn(Option<u64>) + Send + Sync>>>;

pub struct TicketedProxy {
    client: Client,
    helper: OnceLock<HelperThread>,
    state: Arc<Mutex<State>>,
    // Selects whether to try the slot-free path, never authorizes a grant.
    // Relaxed is sufficient: State protects every actual admission predicate.
    // A stale false misses an optimization, a stale true merely takes a lock.
    idle_hint: AtomicBool,
    #[cfg(test)]
    pending_rechecked: TestHook,
    #[cfg(test)]
    before_ack: TestHook,
    #[cfg(test)]
    before_slot_allocation: TestHook,
}
impl TicketedProxy {
    pub fn new() -> Arc<Self> {
        Self::new_with_event_capacity(4096)
    }
    /// Zero disables event capture only. Inventories remain active in both modes,
    /// and neither recording capacity nor identity overflow changes admission.
    pub fn new_with_event_capacity(capacity: usize) -> Arc<Self> {
        Self::with_client_capture(client(), capacity, capacity != 0)
    }
    #[cfg(test)]
    fn with_client(client: Client, event_cap: usize) -> Arc<Self> {
        Self::with_client_capture(client, event_cap, true)
    }
    fn with_client_capture(client: Client, event_cap: usize, capture_events: bool) -> Arc<Self> {
        let proxy = Arc::new(Self {
            client,
            helper: OnceLock::new(),
            idle_hint: AtomicBool::new(false),
            state: Arc::new(Mutex::new(State {
                used: 1,
                pending: 0,
                head: None,
                tail: None,
                next_ticket: Some(0),
                next_release: Some(0),
                next_callback: Some(0),
                inventory: Inventory::default(),
                events: Vec::with_capacity(event_cap),
                capture_events,
                event_cap,
            })),
            #[cfg(test)]
            pending_rechecked: Mutex::new(None),
            #[cfg(test)]
            before_ack: Mutex::new(None),
            #[cfg(test)]
            before_slot_allocation: Mutex::new(None),
        });
        // Only state is captured. No proxy/helper cycle and no last proxy drop
        // on the helper thread (HelperThread::drop joins its own worker).
        let callback_state = Arc::clone(&proxy.state);
        let helper = proxy
            .client
            .clone()
            .into_helper_thread(move |token| {
                match token {
                    Ok(token) => {
                        let granted = Self::helper_supply(&callback_state);
                        // Both surplus release and token consumption happen outside locks.
                        if granted {
                            token.drop_without_releasing();
                        } else {
                            drop(token);
                        }
                        let mut guard = callback_state.lock();
                        let state = &mut *guard;
                        increment(
                            &mut state.inventory.callbacks_completed,
                            &mut state.inventory.overflow,
                        );
                    }
                    Err(_) => {
                        let mut guard = callback_state.lock();
                        let state = &mut *guard;
                        state.error();
                        increment(
                            &mut state.inventory.callbacks_completed,
                            &mut state.inventory.overflow,
                        );
                    }
                }
            })
            .expect("failed to spawn helper thread");
        proxy.helper.set(helper).unwrap();
        proxy
    }
    fn helper_supply(state: &Mutex<State>) -> bool {
        let mut guard = state.lock();
        let state = &mut *guard;
        let callback = state.identity(2);
        increment(&mut state.inventory.supplies, &mut state.inventory.overflow);
        // Check conservation before mutating Pending or consuming an acquired token.
        let used = if state.pending > 0 {
            state.used.checked_add(1).expect("used count overflow")
        } else {
            state.used
        };
        let granted = if let Some(slot) = state.pop() {
            assert!(state.used > 0);
            state.used = used;
            state.grant(&slot, GrantSource::Helper(callback));
            true
        } else {
            false
        };
        state.record(Transition::HelperSupply { callback, granted });
        granted
    }
    pub fn acquire_thread(&self) {
        if self.idle_hint.load(Ordering::Relaxed) {
            let mut guard = self.state.lock();
            if guard.used == 0 {
                let state = &mut *guard;
                assert_eq!(state.pending, 0);
                let ticket = state.identity(0);
                increment(&mut state.inventory.requests, &mut state.inventory.overflow);
                state.record(Transition::Requested { ticket });
                // No request can already be pending when the retained implicit
                // allowance is idle. Reserve and acknowledge it under State,
                // without allocating a slot or locking a subordinate predicate.
                state.used = 1;
                self.idle_hint.store(false, Ordering::Relaxed);
                let source = GrantSource::IdleImplicit;
                increment(&mut state.inventory.grants, &mut state.inventory.overflow);
                state.record(Transition::Granted { ticket, source });
                #[cfg(test)]
                {
                    // Preserve the actual grant-before-ack pause seam without
                    // letting a controller wait while State is protected.
                    drop(guard);
                    let hook = self.before_ack.lock().clone();
                    if let Some(hook) = hook {
                        hook(ticket);
                    }
                    guard = self.state.lock();
                }
                let state = &mut *guard;
                increment(&mut state.inventory.acks, &mut state.inventory.overflow);
                state.record(Transition::Ack { ticket, source });
                return;
            }
        }
        #[cfg(test)]
        {
            let hook = self.before_slot_allocation.lock().clone();
            if let Some(hook) = hook {
                hook(None);
            }
        }
        // Allocate before taking the protected proxy lock. Queue linking does not allocate.
        // Registration checks used regardless of the advisory hint, because a
        // release can make the implicit allowance idle during this allocation.
        // Register exactly one request here, as in the original slow path.
        let mut slot = Arc::new(Slot {
            ticket: None,
            next: Mutex::new(None),
            phase: Mutex::new(Phase::Pending),
            wake: Condvar::new(),
        });
        {
            let mut guard = self.state.lock();
            let state = &mut *guard;
            let ticket = state.identity(0);
            Arc::get_mut(&mut slot).unwrap().ticket = ticket;
            increment(&mut state.inventory.requests, &mut state.inventory.overflow);
            state.record(Transition::Requested { ticket });
            if state.used == 0 {
                assert_eq!(state.pending, 0);
                state.used = 1;
                self.idle_hint.store(false, Ordering::Relaxed);
                state.grant(&slot, GrantSource::IdleImplicit);
            } else {
                state.push(Arc::clone(&slot));
                state.record(Transition::Pending { ticket });
                self.helper.get().unwrap().request_token();
            }
        }
        {
            let mut state = self.state.lock();
            while *slot.phase.lock() == Phase::Pending {
                slot.wake.wait(&mut state);
                #[cfg(test)]
                if *slot.phase.lock() == Phase::Pending {
                    drop(state);
                    let hook = self.pending_rechecked.lock().clone();
                    if let Some(hook) = hook {
                        hook(slot.ticket);
                    }
                    state = self.state.lock();
                }
            }
        }
        #[cfg(test)]
        {
            let hook = self.before_ack.lock().clone();
            if let Some(hook) = hook {
                hook(slot.ticket);
            }
        }
        let mut guard = self.state.lock();
        let state = &mut *guard;
        let mut phase = slot.phase.lock();
        let Phase::Granted(source) = *phase else { panic!("request must have targeted grant") };
        *phase = Phase::Ack;
        increment(&mut state.inventory.acks, &mut state.inventory.overflow);
        state.record(Transition::Ack { ticket: slot.ticket, source });
    }
    pub fn release_thread(&self) {
        let release_raw = {
            let mut guard = self.state.lock();
            let state = &mut *guard;
            assert!(state.used > 0, "release without aggregate allowance");
            let operation = state.identity(1);
            increment(&mut state.inventory.releases, &mut state.inventory.overflow);
            let transferred = if let Some(slot) = state.pop() {
                state.grant(&slot, GrantSource::Release(operation));
                true
            } else {
                state.used = state.used.checked_sub(1).expect("used count underflow");
                if state.used == 0 {
                    // Publish the optimization hint under the same State guard
                    // as the idle transition. Acquirers still recheck State.
                    self.idle_hint.store(true, Ordering::Relaxed);
                }
                false
            };
            state.record(Transition::Released { operation, transferred });
            !transferred && state.used > 0
        };
        if release_raw {
            self.client.release_raw().ok();
        }
    }
    /// Diagnostic only. Allocates a snapshot and must not be called in callbacks.
    pub fn debug_snapshot(&self) -> Snapshot {
        let state = self.state.lock();
        Snapshot {
            used: state.used,
            pending: state.pending,
            inventory: state.inventory.clone(),
            events: state.events.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};
    fn wait_until(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(Instant::now() < deadline, "protocol setup timed out");
            thread::yield_now();
        }
    }
    fn fixture(cap: usize) -> (Arc<TicketedProxy>, Client) {
        let client = Client::new(0).unwrap();
        (TicketedProxy::with_client(client.clone(), cap), client)
    }
    fn supply_fixture() -> (Arc<TicketedProxy>, Client, super::super::Acquired) {
        let client = Client::new(1).unwrap();
        let held = client.acquire().unwrap();
        (TicketedProxy::with_client(client.clone(), 64), client, held)
    }
    fn worker(proxy: &Arc<TicketedProxy>) -> (thread::JoinHandle<()>, mpsc::Receiver<()>) {
        let proxy = proxy.clone();
        let (tx, rx) = mpsc::channel();
        (
            thread::spawn(move || {
                proxy.acquire_thread();
                tx.send(()).unwrap();
            }),
            rx,
        )
    }
    fn notify_and_confirm_pending(proxy: &TicketedProxy, slot: &Slot) {
        let (tx, rx) = mpsc::channel();
        *proxy.pending_rechecked.lock() = Some(Arc::new(move |ticket| {
            let _ = tx.send(ticket);
        }));
        wait_until(|| {
            // Take State so the notification is serialized with predicate checks.
            {
                let _state = proxy.state.lock();
                slot.wake.notify_one();
            }
            rx.try_recv().is_ok()
        });
        *proxy.pending_rechecked.lock() = None;
        let _state = proxy.state.lock();
        assert_eq!(*slot.phase.lock(), Phase::Pending);
    }
    #[test]
    fn ungranted_notification_does_not_return() {
        let (proxy, _) = fixture(64);
        let (w, r) = worker(&proxy);
        wait_until(|| proxy.debug_snapshot().pending == 1);
        let slot = proxy.state.lock().head.clone().unwrap();
        notify_and_confirm_pending(&proxy, &slot);
        assert!(r.try_recv().is_err());
        {
            let _state = proxy.state.lock();
            assert_eq!(*slot.phase.lock(), Phase::Pending);
        }
        proxy.release_thread();
        r.recv_timeout(Duration::from_secs(5)).unwrap();
        w.join().unwrap();
        proxy.release_thread();
        assert_eq!(proxy.debug_snapshot().used, 0);
    }
    #[test]
    fn actual_target_grants_survive_reverse_ack_order() {
        let (proxy, client, held) = supply_fixture();
        let pause = Arc::new(Barrier::new(2));
        let (tx, rx) = mpsc::channel();
        let pause_worker = pause.clone();
        *proxy.before_ack.lock() = Some(Arc::new(move |ticket| {
            if ticket == Some(0) {
                tx.send(()).unwrap();
                pause_worker.wait();
            }
        }));
        let (w1, r1) = worker(&proxy);
        wait_until(|| proxy.debug_snapshot().pending == 1);
        let (w2, r2) = worker(&proxy);
        wait_until(|| proxy.debug_snapshot().pending == 2);
        let slot2 = proxy.state.lock().tail.clone().unwrap();
        proxy.release_thread();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        notify_and_confirm_pending(&proxy, &slot2);
        assert!(r2.try_recv().is_err());
        drop(held);
        r2.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(r1.try_recv().is_err());
        let s = proxy.debug_snapshot();
        assert_eq!((s.used, s.inventory.grants, s.inventory.acks), (2, 2, 1));
        assert!(s.events.contains(&Transition::Granted {
            ticket: Some(0),
            source: GrantSource::Release(Some(0))
        }));
        assert!(s.events.contains(&Transition::Granted {
            ticket: Some(1),
            source: GrantSource::Helper(Some(0))
        }));
        pause.wait();
        r1.recv_timeout(Duration::from_secs(5)).unwrap();
        w1.join().unwrap();
        w2.join().unwrap();
        let acks: Vec<_> =
            proxy
                .debug_snapshot()
                .events
                .into_iter()
                .filter_map(|e| {
                    if let Transition::Ack { ticket, .. } = e { Some(ticket) } else { None }
                })
                .collect();
        assert_eq!(acks, vec![Some(1), Some(0)]);
        proxy.release_thread();
        proxy.release_thread();
        wait_until(|| proxy.debug_snapshot().inventory.callbacks_completed == 2);
        assert_eq!(client.available().unwrap(), 1);
    }
    #[test]
    fn local_transfer_surplus_and_retained_implicit() {
        let (proxy, client, held) = supply_fixture();
        let (w, r) = worker(&proxy);
        wait_until(|| proxy.debug_snapshot().pending == 1);
        proxy.release_thread();
        r.recv_timeout(Duration::from_secs(5)).unwrap();
        w.join().unwrap();
        drop(held);
        wait_until(|| proxy.debug_snapshot().inventory.callbacks_completed == 1);
        assert_eq!(proxy.debug_snapshot().used, 1);
        assert_eq!(client.available().unwrap(), 1);
        let surplus = client.acquire().unwrap();
        assert_eq!(client.available().unwrap(), 0);
        drop(surplus);
        proxy.release_thread();
        proxy.acquire_thread();
        assert_eq!(proxy.debug_snapshot().used, 1);
        assert!(
            proxy.debug_snapshot().events.iter().any(|e| matches!(
                e,
                Transition::Granted { source: GrantSource::IdleImplicit, .. }
            ))
        );
        proxy.release_thread();
    }
    #[test]
    fn local_transfer_races_actual_helper_supply() {
        for _ in 0..20 {
            let (proxy, client, held) = supply_fixture();
            let (w, r) = worker(&proxy);
            wait_until(|| proxy.debug_snapshot().pending == 1);
            let gate = Arc::new(Barrier::new(3));
            let p = proxy.clone();
            let g = gate.clone();
            let release = thread::spawn(move || {
                g.wait();
                p.release_thread();
            });
            let g = gate.clone();
            let supply = thread::spawn(move || {
                g.wait();
                drop(held);
            });
            gate.wait();
            release.join().unwrap();
            supply.join().unwrap();
            r.recv_timeout(Duration::from_secs(5)).unwrap();
            w.join().unwrap();
            wait_until(|| proxy.debug_snapshot().inventory.callbacks_completed == 1);
            let s = proxy.debug_snapshot();
            assert_eq!((s.used, s.pending, s.inventory.grants, s.inventory.acks), (1, 0, 1, 1));
            assert_eq!(client.available().unwrap(), 1);
            proxy.release_thread();
        }
    }
    #[test]
    fn error_is_not_a_grant() {
        let (proxy, _) = fixture(64);
        let (w, r) = worker(&proxy);
        wait_until(|| proxy.debug_snapshot().pending == 1);
        // Exact callback error transition injection, not a real OS failure.
        proxy.state.lock().error();
        let s = proxy.debug_snapshot();
        assert_eq!((s.pending, s.inventory.grants, s.inventory.helper_errors), (1, 0, 1));
        let slot = proxy.state.lock().head.clone().unwrap();
        notify_and_confirm_pending(&proxy, &slot);
        assert!(r.try_recv().is_err());
        proxy.release_thread();
        r.recv_timeout(Duration::from_secs(5)).unwrap();
        w.join().unwrap();
        proxy.release_thread();
    }
    #[test]
    fn helper_first_release_preserves_reserved_grant() {
        let (proxy, client, held) = supply_fixture();
        let gate = Arc::new(Barrier::new(2));
        let g = gate.clone();
        let (tx, rx) = mpsc::channel();
        *proxy.before_ack.lock() = Some(Arc::new(move |_| {
            tx.send(()).unwrap();
            g.wait();
        }));
        let (w, r) = worker(&proxy);
        wait_until(|| proxy.debug_snapshot().pending == 1);
        drop(held);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        wait_until(|| proxy.debug_snapshot().inventory.callbacks_completed == 1);
        assert_eq!(proxy.debug_snapshot().used, 2);
        proxy.release_thread();
        assert_eq!(proxy.debug_snapshot().used, 1);
        assert_eq!(client.available().unwrap(), 1);
        gate.wait();
        r.recv_timeout(Duration::from_secs(5)).unwrap();
        w.join().unwrap();
        proxy.release_thread();
    }
    #[test]
    fn dropping_proxy_interrupts_outstanding_real_helper_request() {
        let (proxy, _) = fixture(64);
        let (w, r) = worker(&proxy);
        wait_until(|| proxy.debug_snapshot().pending == 1);
        proxy.release_thread();
        r.recv_timeout(Duration::from_secs(5)).unwrap();
        w.join().unwrap();
        proxy.release_thread();
        let (tx, rx) = mpsc::channel();
        let dropper = thread::spawn(move || {
            drop(proxy);
            tx.send(()).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        dropper.join().unwrap();
    }
    #[test]
    fn checked_native_ceiling_does_not_mutate_pending_and_surplus_is_safe() {
        let (proxy, _) = fixture(64);
        proxy.state.lock().used = usize::MAX;
        assert!(!TicketedProxy::helper_supply(&proxy.state));
        let slot = Arc::new(Slot {
            ticket: None,
            next: Mutex::new(None),
            phase: Mutex::new(Phase::Pending),
            wake: Condvar::new(),
        });
        proxy.state.lock().push(slot.clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            TicketedProxy::helper_supply(&proxy.state);
        }));
        assert!(result.is_err());
        let mut state = proxy.state.lock();
        assert_eq!(state.pending, 1);
        assert_eq!(*slot.phase.lock(), Phase::Pending);
        state.pop();
        state.used = 0;
    }
    #[test]
    fn recorder_cap_and_identity_overflow_do_not_control_admission() {
        let (proxy, _) = fixture(0);
        {
            let mut s = proxy.state.lock();
            s.next_ticket = Some(u64::MAX);
            s.next_release = None;
            s.inventory.requests = u64::MAX;
        }
        proxy.release_thread();
        for _ in 0..3 {
            proxy.acquire_thread();
            proxy.release_thread();
        }
        let s = proxy.debug_snapshot();
        assert_eq!((s.used, s.pending, s.inventory.grants, s.inventory.acks), (0, 0, 3, 3));
        assert!(s.events.is_empty());
        assert!(s.inventory.lost_events > 0);
        assert!(s.inventory.overflow);
    }

    #[test]
    fn event_disabled_and_enabled_have_identical_admission_inventories() {
        let mut inventories = Vec::new();
        for capture in [false, true] {
            let client = Client::new(1).unwrap();
            let held = client.acquire().unwrap();
            let proxy = TicketedProxy::with_client_capture(
                client.clone(),
                if capture { 64 } else { 0 },
                capture,
            );
            let (w, r) = worker(&proxy);
            wait_until(|| proxy.debug_snapshot().pending == 1);
            drop(held);
            r.recv_timeout(Duration::from_secs(5)).unwrap();
            w.join().unwrap();
            wait_until(|| proxy.debug_snapshot().inventory.callbacks_completed == 1);
            proxy.release_thread();
            proxy.release_thread();
            proxy.acquire_thread();
            proxy.release_thread();
            let s = proxy.debug_snapshot();
            assert_eq!((s.used, s.pending), (0, 0));
            assert_eq!(s.inventory.lost_events, 0);
            assert_eq!(s.events.is_empty(), !capture);
            assert_eq!(client.available().unwrap(), 1);
            inventories.push(s.inventory);
        }
        assert_eq!(inventories[0], inventories[1]);
    }
    #[test]
    fn queue_count_boundary_rejects_before_mutating_links() {
        let (proxy, _) = fixture(64);
        let slot = Arc::new(Slot {
            ticket: None,
            next: Mutex::new(None),
            phase: Mutex::new(Phase::Pending),
            wake: Condvar::new(),
        });
        {
            let mut s = proxy.state.lock();
            s.push(slot.clone());
            s.pending = 0;
        }
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                proxy.state.lock().pop();
            }))
            .is_err()
        );
        {
            let mut s = proxy.state.lock();
            assert!(Arc::ptr_eq(s.head.as_ref().unwrap(), &slot));
            assert!(Arc::ptr_eq(s.tail.as_ref().unwrap(), &slot));
            s.pending = 1;
            s.pop();
            s.pending = usize::MAX;
        }
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                proxy.state.lock().push(slot.clone());
            }))
            .is_err()
        );
        let mut s = proxy.state.lock();
        assert!(s.head.is_none());
        assert!(s.tail.is_none());
        s.pending = 0;
    }

    #[test]
    fn idle_grant_is_reserved_before_outside_lock_ack_hook() {
        let (proxy, client, held) = supply_fixture();
        proxy.release_thread();
        let gate = Arc::new(Barrier::new(2));
        let g = gate.clone();
        let (tx, rx) = mpsc::channel();
        *proxy.before_ack.lock() = Some(Arc::new(move |ticket| {
            if ticket == Some(0) {
                tx.send(()).unwrap();
                g.wait();
            }
        }));
        let (w1, r1) = worker(&proxy);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let s = proxy.debug_snapshot();
        assert_eq!(
            (s.used, s.pending, s.inventory.requests, s.inventory.grants, s.inventory.acks),
            (1, 0, 1, 1, 0)
        );
        assert!(
            s.events.contains(&Transition::Granted {
                ticket: Some(0),
                source: GrantSource::IdleImplicit
            })
        );
        let (w2, r2) = worker(&proxy);
        wait_until(|| proxy.debug_snapshot().pending == 1);
        drop(held);
        r2.recv_timeout(Duration::from_secs(5)).unwrap();
        w2.join().unwrap();
        assert!(r1.try_recv().is_err());
        let s = proxy.debug_snapshot();
        assert_eq!((s.used, s.inventory.grants, s.inventory.acks), (2, 2, 1));
        proxy.release_thread();
        gate.wait();
        r1.recv_timeout(Duration::from_secs(5)).unwrap();
        w1.join().unwrap();
        proxy.release_thread();
        wait_until(|| proxy.debug_snapshot().inventory.callbacks_completed == 1);
        assert_eq!(client.available().unwrap(), 1);
    }

    #[test]
    fn slow_registration_rechecks_idle_after_allocation_gap() {
        let (proxy, _) = fixture(64);
        let gate = Arc::new(Barrier::new(2));
        let g = gate.clone();
        let (tx, rx) = mpsc::channel();
        *proxy.before_slot_allocation.lock() = Some(Arc::new(move |_| {
            tx.send(()).unwrap();
            g.wait();
        }));
        let (w, r) = worker(&proxy);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        *proxy.before_slot_allocation.lock() = None;
        assert_eq!(proxy.debug_snapshot().inventory.requests, 0);
        proxy.release_thread();
        assert_eq!(proxy.debug_snapshot().used, 0);
        gate.wait();
        r.recv_timeout(Duration::from_secs(5)).unwrap();
        w.join().unwrap();
        let s = proxy.debug_snapshot();
        assert_eq!(
            (s.used, s.pending, s.inventory.requests, s.inventory.grants, s.inventory.acks),
            (1, 0, 1, 1, 1)
        );
        assert!(!s.events.iter().any(|e| matches!(e, Transition::Pending { .. })));
        assert!(
            s.events.contains(&Transition::Granted {
                ticket: Some(0),
                source: GrantSource::IdleImplicit
            })
        );
        proxy.release_thread();
    }

    #[test]
    fn allocation_gap_cannot_leapfrog_a_registered_pending_request() {
        let (proxy, _) = fixture(64);
        let gate = Arc::new(Barrier::new(2));
        let g = gate.clone();
        let (tx, rx) = mpsc::channel();
        *proxy.before_slot_allocation.lock() = Some(Arc::new(move |_| {
            tx.send(()).unwrap();
            g.wait();
        }));
        let (w1, r1) = worker(&proxy);
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        *proxy.before_slot_allocation.lock() = None;
        let (w2, r2) = worker(&proxy);
        wait_until(|| proxy.debug_snapshot().pending == 1);
        proxy.release_thread();
        r2.recv_timeout(Duration::from_secs(5)).unwrap();
        w2.join().unwrap();
        gate.wait();
        wait_until(|| proxy.debug_snapshot().pending == 1);
        let slot = proxy.state.lock().head.clone().unwrap();
        notify_and_confirm_pending(&proxy, &slot);
        assert!(r1.try_recv().is_err());
        let s = proxy.debug_snapshot();
        assert_eq!(
            (s.used, s.inventory.requests, s.inventory.grants, s.inventory.acks),
            (1, 2, 1, 1)
        );
        proxy.release_thread();
        r1.recv_timeout(Duration::from_secs(5)).unwrap();
        w1.join().unwrap();
        let s = proxy.debug_snapshot();
        assert!(s.events.contains(&Transition::Granted {
            ticket: Some(0),
            source: GrantSource::Release(Some(0))
        }));
        assert!(s.events.contains(&Transition::Granted {
            ticket: Some(1),
            source: GrantSource::Release(Some(1))
        }));
        proxy.release_thread();
    }

    #[test]
    fn slot_free_idle_path_preserves_event_modes_and_overflow() {
        for capture in [false, true] {
            let proxy = TicketedProxy::with_client_capture(Client::new(0).unwrap(), 2, capture);
            proxy.release_thread();
            *proxy.before_slot_allocation.lock() =
                Some(Arc::new(|_| panic!("idle acquired a slot")));
            {
                let mut state = proxy.state.lock();
                state.next_ticket = None;
                state.inventory.requests = u64::MAX;
                state.inventory.grants = u64::MAX;
                state.inventory.acks = u64::MAX;
            }
            proxy.acquire_thread();
            let s = proxy.debug_snapshot();
            assert_eq!((s.used, s.pending), (1, 0));
            assert_eq!(
                (s.inventory.requests, s.inventory.grants, s.inventory.acks),
                (u64::MAX, u64::MAX, u64::MAX)
            );
            assert!(s.inventory.overflow);
            assert_eq!(s.events.is_empty(), !capture);
            assert_eq!(s.inventory.lost_events > 0, capture);
            proxy.release_thread();
            assert_eq!(proxy.debug_snapshot().used, 0);
        }
    }

    #[test]
    fn stale_true_hint_does_not_authorize_a_grant() {
        let mut inventories = Vec::new();
        for capture in [false, true] {
            let proxy = TicketedProxy::with_client_capture(Client::new(0).unwrap(), 64, capture);
            assert_eq!(proxy.debug_snapshot().used, 1);
            proxy.idle_hint.store(true, Ordering::Relaxed);
            let (w, r) = worker(&proxy);
            wait_until(|| proxy.debug_snapshot().pending == 1);
            let slot = proxy.state.lock().head.clone().unwrap();
            notify_and_confirm_pending(&proxy, &slot);
            let s = proxy.debug_snapshot();
            assert_eq!(
                (s.used, s.inventory.requests, s.inventory.grants, s.inventory.acks),
                (1, 1, 0, 0)
            );
            assert!(r.try_recv().is_err());
            proxy.release_thread();
            r.recv_timeout(Duration::from_secs(5)).unwrap();
            w.join().unwrap();
            let s = proxy.debug_snapshot();
            assert_eq!((s.used, s.pending, s.inventory.grants, s.inventory.acks), (1, 0, 1, 1));
            if capture {
                assert!(s.events.contains(&Transition::Granted {
                    ticket: Some(0),
                    source: GrantSource::Release(Some(0))
                }));
                assert!(!s.events.iter().any(|e| matches!(
                    e,
                    Transition::Granted { source: GrantSource::IdleImplicit, .. }
                )));
            }
            proxy.release_thread();
            assert!(proxy.idle_hint.load(Ordering::Relaxed));
            inventories.push(proxy.debug_snapshot().inventory);
        }
        assert_eq!(inventories[0], inventories[1]);
    }

    #[test]
    fn stale_false_hint_uses_original_registration_and_rechecks_idle() {
        use std::sync::atomic::AtomicUsize;
        let mut inventories = Vec::new();
        for capture in [false, true] {
            let proxy = TicketedProxy::with_client_capture(Client::new(0).unwrap(), 64, capture);
            proxy.release_thread();
            assert!(proxy.idle_hint.load(Ordering::Relaxed));
            proxy.idle_hint.store(false, Ordering::Relaxed);
            let allocations = Arc::new(AtomicUsize::new(0));
            let count = allocations.clone();
            *proxy.before_slot_allocation.lock() = Some(Arc::new(move |_| {
                count.fetch_add(1, Ordering::Relaxed);
            }));
            let acks = Arc::new(AtomicUsize::new(0));
            let count = acks.clone();
            *proxy.before_ack.lock() = Some(Arc::new(move |_| {
                count.fetch_add(1, Ordering::Relaxed);
            }));
            proxy.acquire_thread();
            assert_eq!(allocations.load(Ordering::Relaxed), 1);
            assert_eq!(acks.load(Ordering::Relaxed), 1);
            assert!(!proxy.idle_hint.load(Ordering::Relaxed));
            let s = proxy.debug_snapshot();
            assert_eq!(
                (s.used, s.pending, s.inventory.requests, s.inventory.grants, s.inventory.acks),
                (1, 0, 1, 1, 1)
            );
            if capture {
                assert!(s.events.contains(&Transition::Granted {
                    ticket: Some(0),
                    source: GrantSource::IdleImplicit
                }));
                assert!(!s.events.iter().any(|e| matches!(e, Transition::Pending { .. })));
            }
            proxy.release_thread();
            assert!(proxy.idle_hint.load(Ordering::Relaxed));
            inventories.push(proxy.debug_snapshot().inventory);
        }
        assert_eq!(inventories[0], inventories[1]);
    }

    #[test]
    fn advisory_hint_release_acquire_races_conserve_allowance_in_both_event_modes() {
        let mut inventories = Vec::new();
        for capture in [false, true] {
            for _ in 0..40 {
                let proxy =
                    TicketedProxy::with_client_capture(Client::new(0).unwrap(), 64, capture);
                let gate = Arc::new(Barrier::new(3));
                let g = gate.clone();
                let p = proxy.clone();
                let release = thread::spawn(move || {
                    g.wait();
                    p.release_thread();
                });
                let g = gate.clone();
                let p = proxy.clone();
                let acquire = thread::spawn(move || {
                    g.wait();
                    p.acquire_thread();
                });
                gate.wait();
                release.join().unwrap();
                acquire.join().unwrap();
                let s = proxy.debug_snapshot();
                assert_eq!(
                    (
                        s.used,
                        s.pending,
                        s.inventory.requests,
                        s.inventory.grants,
                        s.inventory.acks,
                        s.inventory.releases
                    ),
                    (1, 0, 1, 1, 1, 1)
                );
                assert!(!proxy.idle_hint.load(Ordering::Relaxed));
                proxy.release_thread();
                assert!(proxy.idle_hint.load(Ordering::Relaxed));
                let s = proxy.debug_snapshot();
                assert_eq!((s.used, s.pending, s.inventory.lost_events), (0, 0, 0));
                inventories.push(s.inventory);
            }
        }
        assert!(inventories.windows(2).all(|pair| pair[0] == pair[1]));
    }
}
