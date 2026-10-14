//! Targeted admission selected only by the explicit frontend factory.
//! The legacy default controller is unchanged.
//! Sources name release operations, not holders or physical tokens.
use super::resource::*;
use super::resource::{Frozen, Ledger, PreparedCapture, Txn};
use super::{Client, HelperThread, client};
use parking_lot::{Condvar, Mutex};
use rustc_thread_pool::admission::AdmissionContext;
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
#[derive(Clone, Copy, Debug)]
struct SlotState {
    phase: Phase,
    wire: AcquireCompletion,
}
struct Slot {
    ticket: Option<u64>,
    next: Mutex<Option<Arc<Slot>>>,
    // Storage synchronization only. Every phase access additionally holds State,
    // and the private CV waits on State, never on this subordinate mutex.
    phase: Mutex<SlotState>,
    wake: Condvar,
}
pub(super) struct State {
    resource: Option<Ledger>,
    granted_unacked: Option<u64>,
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
    fn aggregate(&self) -> Aggregate {
        Aggregate {
            assigned: u64::try_from(self.used).ok(),
            pending: u64::try_from(self.pending).ok(),
            granted_unacked: self.granted_unacked,
        }
    }
    fn count(&mut self, field: fn(&mut CounterSet) -> &mut u64) {
        if let Some(r) = &mut self.resource {
            r.count(field);
        }
    }
    fn wire_id(&mut self, field: fn(&mut Allocators) -> &mut Allocator) -> Option<u64> {
        self.resource.as_mut().and_then(|r| r.id(field))
    }
    fn finish(&mut self, txn: Txn) {
        let after = self.aggregate();
        if let Some(r) = &mut self.resource {
            if r.active() && after.granted_unacked.is_none() {
                r.counter_overflowed = true;
            }
            r.finish(txn, after);
        }
    }
    fn request(&mut self, context: Option<AdmissionContext>, txn: &mut Txn) -> AcquireCompletion {
        self.count(|c| &mut c.requests);
        if context.is_none() {
            self.count(|c| &mut c.unqualified_requests);
        }
        let request = self.wire_id(|a| &mut a.request);
        txn.push(request.map(|request_id| Fact::Request { request_id, context }));
        AcquireCompletion {
            capture: self.resource.as_ref().filter(|r| r.active()).map(|r| r.key),
            request,
            grant: None,
            ack: None,
        }
    }
    fn wire_grant(
        &mut self,
        completion: &mut AcquireCompletion,
        source: Option<WireGrantSource>,
        txn: &mut Txn,
    ) {
        self.granted_unacked = self.granted_unacked.and_then(|v| v.checked_add(1));
        completion.grant = self.wire_id(|a| &mut a.grant);
        txn.push(
            completion.grant.zip(completion.request).zip(source).map(
                |((grant_id, request_id), source)| Fact::Grant { grant_id, request_id, source },
            ),
        );
    }
    fn ack(&mut self, completion: &mut AcquireCompletion, txn: &mut Txn) {
        self.granted_unacked = self.granted_unacked.and_then(|v| v.checked_sub(1));
        self.count(|c| &mut c.acks);
        completion.ack = self.wire_id(|a| &mut a.ack);
        // A late capture may observe an old request's Ack, but cannot invent
        // its missing request/grant identities. Carry association for loss only.
        completion.capture = self.resource.as_ref().filter(|r| r.active()).map(|r| r.key);
        txn.push(
            completion
                .ack
                .zip(completion.request)
                .zip(completion.grant)
                .map(|((ack_id, request_id), grant_id)| Fact::Ack { ack_id, request_id, grant_id }),
        );
    }
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
    fn grant(
        &mut self,
        slot: &Slot,
        source: GrantSource,
        wire_source: Option<WireGrantSource>,
        txn: &mut Txn,
    ) {
        let mut phase = slot.phase.lock();
        assert_eq!(phase.phase, Phase::Pending);
        phase.phase = Phase::Granted(source);
        increment(&mut self.inventory.grants, &mut self.inventory.overflow);
        self.record(Transition::Granted { ticket: slot.ticket, source });
        self.wire_grant(&mut phase.wire, wire_source, txn);
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
    controller_id: OnceLock<ControllerId>,
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
    /// Prepare all identities and storage before entering native synchronization.
    pub fn prepare_resource_capture(
        &self,
        config: ResourceCaptureConfig,
    ) -> Option<ResourceCaptureControl> {
        if self.controller_id.get().is_none() {
            let _ = self.controller_id.set(ControllerId::mint()?);
        }
        let prepared = PreparedCapture::new(self.controller_id.get()?.clone(), config)?;
        let mut ledger = Ledger::new(config)?;
        let mut state = self.state.lock();
        if state.resource.is_some() {
            return None;
        }
        ledger.inflight_known = state.inventory == Inventory::default()
            && state.used == 1
            && state.pending == 0
            && state.granted_unacked == Some(0);
        ledger.start = ledger.checkpoint(state.aggregate());
        state.resource = Some(ledger);
        let mut txn = Txn::new(state.aggregate());
        txn.push(Some(Fact::CaptureStart));
        state.finish(txn);
        drop(state);
        Some(ResourceCaptureControl { state: Arc::clone(&self.state), prepared })
    }
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
            controller_id: OnceLock::new(),
            idle_hint: AtomicBool::new(false),
            state: Arc::new(Mutex::new(State {
                resource: None,
                granted_unacked: Some(0),
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
        // on the helper thread. HelperThread Drop may conditionally detach its
        // worker, so Drop return does not establish helper quiescence.
        let callback_state = Arc::clone(&proxy.state);
        let helper = proxy
            .client
            .clone()
            .into_helper_thread(move |token| {
                let callback = {
                    let mut state = callback_state.lock();
                    let mut txn = Txn::new(state.aggregate());
                    state.count(|c| &mut c.callbacks_registered);
                    let id = state.wire_id(|a| &mut a.callback);
                    txn.push(id.map(|callback_id| Fact::CallbackRegistered { callback_id }));
                    state.finish(txn);
                    id
                };
                match token {
                    Ok(token) => {
                        let (granted, attempt) = Self::helper_decision(&callback_state, callback);
                        // Preserve payload-consuming and payload-releasing semantics.
                        if granted {
                            token.drop_without_releasing();
                        } else {
                            Self::raw_begin(
                                &callback_state,
                                attempt,
                                callback.map(RawParent::HelperSurplus),
                            );
                            drop(token);
                            Self::raw_end(&callback_state, attempt, RawOutcome::Unknown);
                        }
                        Self::callback_complete(
                            &callback_state,
                            callback,
                            if granted {
                                CallbackDisposition::Consumed
                            } else {
                                CallbackDisposition::SurplusDropFinished
                            },
                        );
                    }
                    Err(error) => {
                        let mut guard = callback_state.lock();
                        let state = &mut *guard;
                        let mut txn = Txn::new(state.aggregate());
                        state.error();
                        state.count(|c| &mut c.callbacks_error);
                        txn.push(callback.map(|callback_id| Fact::HelperDecision {
                            callback_id,
                            outcome: HelperOutcome::Error(NativeError::observe(&error)),
                        }));
                        state.finish(txn);
                        drop(guard);
                        Self::callback_complete(
                            &callback_state,
                            callback,
                            CallbackDisposition::Error,
                        );
                    }
                }
            })
            .expect("failed to spawn helper thread");
        proxy.helper.set(helper).unwrap();
        proxy
    }
    #[cfg(test)]
    fn helper_supply(state: &Mutex<State>) -> bool {
        Self::helper_decision(state, None).0
    }
    fn helper_decision(state: &Mutex<State>, wire_callback: Option<u64>) -> (bool, Option<u64>) {
        let mut guard = state.lock();
        let state = &mut *guard;
        let mut txn = Txn::new(state.aggregate());
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
            state.count(|c| &mut c.helper_grants);
            state.count(|c| &mut c.callbacks_accepted);
            state.grant(
                &slot,
                GrantSource::Helper(callback),
                wire_callback.map(WireGrantSource::Helper),
                &mut txn,
            );
            let wire = slot.phase.lock().wire;
            txn.push(wire_callback.zip(wire.grant).zip(wire.request).map(
                |((callback_id, grant_id), request_id)| Fact::HelperDecision {
                    callback_id,
                    outcome: HelperOutcome::Accepted { grant_id, request_id },
                },
            ));
            true
        } else {
            false
        };
        let attempt = if granted {
            None
        } else {
            state.count(|c| &mut c.callbacks_surplus);
            let attempt = state.wire_id(|a| &mut a.raw_release);
            txn.push(wire_callback.zip(attempt).map(|(callback_id, attempt_id)| {
                Fact::HelperDecision { callback_id, outcome: HelperOutcome::Surplus { attempt_id } }
            }));
            attempt
        };
        state.record(Transition::HelperSupply { callback, granted });
        state.finish(txn);
        (granted, attempt)
    }
    fn callback_complete(
        state: &Mutex<State>,
        callback: Option<u64>,
        disposition: CallbackDisposition,
    ) {
        let mut state = state.lock();
        let mut txn = Txn::new(state.aggregate());
        let s = &mut *state;
        increment(&mut s.inventory.callbacks_completed, &mut s.inventory.overflow);
        state.count(|c| &mut c.callbacks_completed);
        txn.push(callback.map(|callback_id| Fact::CallbackComplete { callback_id, disposition }));
        state.finish(txn);
    }
    fn raw_begin(state: &Mutex<State>, attempt: Option<u64>, parent: Option<RawParent>) {
        let mut state = state.lock();
        let mut txn = Txn::new(state.aggregate());
        state.count(|c| &mut c.raw_release_begins);
        txn.push(
            attempt
                .zip(parent)
                .map(|(attempt_id, parent)| Fact::RawReleaseBegin { attempt_id, parent }),
        );
        state.finish(txn);
    }
    fn raw_end(state: &Mutex<State>, attempt: Option<u64>, outcome: RawOutcome) {
        let mut state = state.lock();
        let mut txn = Txn::new(state.aggregate());
        state.count(match outcome {
            RawOutcome::Ok => |c| &mut c.raw_release_ok,
            RawOutcome::Error(_) => |c| &mut c.raw_release_error,
            RawOutcome::Unknown => |c| &mut c.raw_release_unknown,
        });
        txn.push(attempt.map(|attempt_id| Fact::RawReleaseEnd { attempt_id, outcome }));
        state.finish(txn);
    }
    pub fn acquire_thread(&self) {
        let completion = self.acquire_frontend(None);
        self.record_acquire_returned(completion);
    }
    pub fn acquire_frontend(&self, context: Option<AdmissionContext>) -> AcquireCompletion {
        if self.idle_hint.load(Ordering::Relaxed) {
            let mut guard = self.state.lock();
            if guard.used == 0 {
                let state = &mut *guard;
                let mut txn = Txn::new(state.aggregate());
                let mut completion = state.request(context, &mut txn);
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
                state.count(|c| &mut c.idle_grants);
                state.wire_grant(&mut completion, Some(WireGrantSource::IdleImplicit), &mut txn);
                #[cfg(test)]
                {
                    state.finish(txn);
                    // Preserve the actual grant-before-ack pause seam without
                    // letting a controller wait while State is protected.
                    drop(guard);
                    let hook = self.before_ack.lock().clone();
                    if let Some(hook) = hook {
                        hook(ticket);
                    }
                    guard = self.state.lock();
                    txn = Txn::new(guard.aggregate());
                }
                let state = &mut *guard;
                increment(&mut state.inventory.acks, &mut state.inventory.overflow);
                state.record(Transition::Ack { ticket, source });
                state.ack(&mut completion, &mut txn);
                state.finish(txn);
                return completion;
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
            phase: Mutex::new(SlotState {
                phase: Phase::Pending,
                wire: AcquireCompletion::default(),
            }),
            wake: Condvar::new(),
        });
        {
            let mut guard = self.state.lock();
            let state = &mut *guard;
            let mut txn = Txn::new(state.aggregate());
            slot.phase.lock().wire = state.request(context, &mut txn);
            let ticket = state.identity(0);
            Arc::get_mut(&mut slot).unwrap().ticket = ticket;
            increment(&mut state.inventory.requests, &mut state.inventory.overflow);
            state.record(Transition::Requested { ticket });
            if state.used == 0 {
                assert_eq!(state.pending, 0);
                state.used = 1;
                self.idle_hint.store(false, Ordering::Relaxed);
                state.count(|c| &mut c.idle_grants);
                state.grant(
                    &slot,
                    GrantSource::IdleImplicit,
                    Some(WireGrantSource::IdleImplicit),
                    &mut txn,
                );
            } else {
                state.push(Arc::clone(&slot));
                state.record(Transition::Pending { ticket });
                state.count(|c| &mut c.pending_registrations);
                let request = slot.phase.lock().wire.request;
                txn.push(request.map(|request_id| Fact::Pending { request_id }));
                state.count(|c| &mut c.helper_issuances);
                let issuance = state.wire_id(|a| &mut a.helper_issuance);
                txn.push(issuance.zip(request).map(|(issuance_id, trigger_request_id)| {
                    Fact::HelperIssued { issuance_id, trigger_request_id }
                }));
                self.helper.get().unwrap().request_token();
            }
            state.finish(txn);
        }
        {
            let mut state = self.state.lock();
            while slot.phase.lock().phase == Phase::Pending {
                slot.wake.wait(&mut state);
                #[cfg(test)]
                if slot.phase.lock().phase == Phase::Pending {
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
        let mut txn = Txn::new(state.aggregate());
        let mut phase = slot.phase.lock();
        let Phase::Granted(source) = phase.phase else {
            panic!("request must have targeted grant")
        };
        phase.phase = Phase::Ack;
        increment(&mut state.inventory.acks, &mut state.inventory.overflow);
        state.record(Transition::Ack { ticket: slot.ticket, source });
        let mut completion = phase.wire;
        state.ack(&mut completion, &mut txn);
        state.finish(txn);
        completion
    }
    /// Called by the adapter only after the actual core has returned.
    pub fn record_acquire_returned(&self, completion: AcquireCompletion) {
        if completion.capture.is_none() {
            return;
        }
        let mut state = self.state.lock();
        let mut txn = Txn::new(state.aggregate());
        state.count(|c| &mut c.acquire_returns);
        let returned = state.wire_id(|a| &mut a.acquire_return);
        let matching = state.resource.as_ref().is_some_and(|r| Some(r.key) == completion.capture);
        txn.push(if matching {
            returned.zip(completion.request).zip(completion.grant).zip(completion.ack).map(
                |(((return_id, request_id), grant_id), ack_id)| Fact::AcquireReturned {
                    return_id,
                    request_id,
                    grant_id,
                    ack_id,
                },
            )
        } else {
            None
        });
        state.finish(txn);
    }
    pub fn release_thread(&self) {
        self.release_frontend(None);
    }
    pub fn release_frontend(&self, context: Option<AdmissionContext>) {
        let (release_raw, attempt, wire_release) = {
            let mut guard = self.state.lock();
            let state = &mut *guard;
            let mut txn = Txn::new(state.aggregate());
            assert!(state.used > 0, "release without aggregate allowance");
            let operation = state.identity(1);
            let wire_release = state.wire_id(|a| &mut a.release);
            state.count(|c| &mut c.release_calls);
            if context.is_none() {
                state.count(|c| &mut c.unqualified_releases);
            }
            let mut disposition = None;
            increment(&mut state.inventory.releases, &mut state.inventory.overflow);
            let transferred = if let Some(slot) = state.pop() {
                state.count(|c| &mut c.transfer_grants);
                state.count(|c| &mut c.release_transfers);
                state.grant(
                    &slot,
                    GrantSource::Release(operation),
                    wire_release.map(WireGrantSource::Release),
                    &mut txn,
                );
                let wire = slot.phase.lock().wire;
                disposition = wire.grant.zip(wire.request).map(|(grant_id, request_id)| {
                    ReleaseDisposition::Transfer { grant_id, request_id }
                });
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
            let release_raw = !transferred && state.used > 0;
            let attempt = if release_raw {
                state.count(|c| &mut c.release_raw_obligations);
                let attempt = state.wire_id(|a| &mut a.raw_release);
                disposition =
                    attempt.map(|attempt_id| ReleaseDisposition::RawRelease { attempt_id });
                attempt
            } else {
                if !transferred {
                    state.count(|c| &mut c.release_retains);
                    disposition = Some(ReleaseDisposition::RetainImplicit);
                }
                None
            };
            txn.push(wire_release.zip(disposition).map(|(release_id, disposition)| {
                Fact::ReleaseDecision { release_id, context, disposition }
            }));
            state.finish(txn);
            (release_raw, attempt, wire_release)
        };
        if release_raw {
            Self::raw_begin(&self.state, attempt, wire_release.map(RawParent::Release));
            let result = self.client.release_raw();
            let outcome = match result {
                Ok(()) => RawOutcome::Ok,
                Err(ref error) => RawOutcome::Error(NativeError::observe(error)),
            };
            Self::raw_end(&self.state, attempt, outcome);
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

impl ResourceCaptureControl {
    pub fn belongs_to(&self, proxy: &TicketedProxy) -> bool {
        Arc::ptr_eq(&self.state, &proxy.state)
    }
    pub fn checkpoint(&self) -> FixedCheckpoint {
        let state = self.state.lock();
        let r = state.resource.as_ref().expect("prepared resource");
        r.frozen.map_or_else(|| r.checkpoint(state.aggregate()), |f| f.end)
    }
    /// Record only. Native admission and helper work continue after this cut.
    pub fn seal_prefix(&self, reason: CaptureCutReason) -> bool {
        let mut state = self.state.lock();
        if !state.resource.as_ref().is_some_and(|r| r.active()) {
            return false;
        }
        let mut txn = Txn::new(state.aggregate());
        txn.push(Some(Fact::CaptureCut));
        state.finish(txn);
        let aggregate = state.aggregate();
        let r = state.resource.as_mut().unwrap();
        let frozen = Frozen {
            start: r.start,
            end: r.checkpoint(aggregate),
            reason,
            identity_exhausted: r.identity_exhausted,
            counter_overflowed: r.counter_overflowed,
            lost_transactions: r.lost_transactions,
            lost_events: r.lost_events,
            loss_overflowed: r.loss_overflowed,
        };
        r.frozen = Some(frozen);
        true
    }
    /// Only call after frontend scoped workers have safely returned or unwound,
    /// outside every query latch/Sleep guard. Does not recut or sample live state.
    pub fn take_sealed_prefix(
        &self,
        buffer: &mut PreallocatedResourceExportBuffer,
    ) -> Option<PrefixFacts> {
        let mut state = self.state.lock();
        let r = state.resource.as_mut()?;
        let frozen = r.frozen?;
        if r.taken || !buffer.transactions.is_empty() || buffer.transactions.capacity() < r.capacity
        {
            return None;
        }
        std::mem::swap(&mut buffer.transactions, &mut r.journal);
        r.taken = true;
        drop(state);
        // Ownership moves and serialization occur only after State is released.
        Some(PrefixFacts {
            prepared: Arc::clone(&self.prepared),
            frozen,
            transactions: std::mem::take(&mut buffer.transactions),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Barrier, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};
    fn capture(proxy: &TicketedProxy, capacity: usize) -> ResourceCaptureControl {
        proxy
            .prepare_resource_capture(ResourceCaptureConfig {
                transaction_capacity: capacity,
                expected_attachments: Some(0),
            })
            .unwrap()
    }
    fn prefix(control: &ResourceCaptureControl) -> PrefixFacts {
        control.seal_prefix(CaptureCutReason::ControllerCheckpoint);
        control
            .take_sealed_prefix(&mut PreallocatedResourceExportBuffer::new(
                control.transaction_capacity(),
            ))
            .unwrap()
    }
    fn facts(prefix: &PrefixFacts) -> Vec<Fact> {
        prefix.transactions.iter().flat_map(|t| t.events.iter().flatten().map(|e| e.fact)).collect()
    }
    #[test]
    fn resource_idle_core_return_is_a_separate_actual_endpoint() {
        let (proxy, _) = fixture(0);
        let control = capture(&proxy, 64);
        proxy.release_thread();
        let completion = proxy.acquire_frontend(None);
        assert_eq!(control.checkpoint().counters.acquire_returns, 0);
        assert_eq!(control.checkpoint().registered_inflight.acquire_cores_not_returned, Some(1));
        proxy.record_acquire_returned(completion);
        let p = prefix(&control);
        assert!(p.records_complete());
        assert_eq!(
            p.start().state,
            Aggregate { assigned: Some(1), pending: Some(0), granted_unacked: Some(0) }
        );
        assert_eq!(p.end().counters.requests, 1);
        assert_eq!(p.end().counters.idle_grants, 1);
        assert_eq!(p.end().counters.acquire_returns, 1);
        let rows = facts(&p);
        assert!(rows.contains(&Fact::Grant {
            grant_id: 0,
            request_id: 0,
            source: WireGrantSource::IdleImplicit
        }));
        assert!(rows.contains(&Fact::AcquireReturned {
            return_id: 0,
            request_id: 0,
            grant_id: 0,
            ack_id: 0
        }));
        assert!(p.to_json().contains("\"physical_callbacks_inflight\":null"));
        proxy.release_thread();
    }
    #[test]
    fn resource_transfer_and_opaque_surplus_are_actual_client_boundaries() {
        let (proxy, client, held) = supply_fixture();
        let control = capture(&proxy, 64);
        let (w, r) = worker(&proxy);
        wait_until(|| control.checkpoint().state.pending == Some(1));
        proxy.release_thread();
        r.recv_timeout(Duration::from_secs(5)).unwrap();
        w.join().unwrap();
        drop(held);
        wait_until(|| control.checkpoint().counters.callbacks_completed == 1);
        let p = prefix(&control);
        assert!(p.records_complete());
        let transfer = p
            .transactions
            .iter()
            .find(|t| {
                t.events.iter().flatten().any(|e| {
                    matches!(
                        e.fact,
                        Fact::ReleaseDecision {
                            disposition: ReleaseDisposition::Transfer { .. },
                            ..
                        }
                    )
                })
            })
            .unwrap();
        assert_eq!(transfer.before.assigned, transfer.after.assigned);
        assert_eq!((transfer.before.pending, transfer.after.pending), (Some(1), Some(0)));
        assert!(transfer.events.iter().flatten().any(|e| e.fact
            == Fact::Grant { grant_id: 0, request_id: 0, source: WireGrantSource::Release(0) }));
        let rows = facts(&p);
        assert!(rows.contains(&Fact::RawReleaseBegin {
            attempt_id: 0,
            parent: RawParent::HelperSurplus(0)
        }));
        assert!(
            rows.contains(&Fact::RawReleaseEnd { attempt_id: 0, outcome: RawOutcome::Unknown })
        );
        assert!(rows.contains(&Fact::CallbackComplete {
            callback_id: 0,
            disposition: CallbackDisposition::SurplusDropFinished
        }));
        assert_eq!(p.end().counters.raw_release_unknown, 1);
        assert_eq!(client.available().unwrap(), 1);
        proxy.release_thread();
    }
    #[test]
    fn resource_helper_grant_and_direct_raw_success_are_observed() {
        let (proxy, client, held) = supply_fixture();
        let control = capture(&proxy, 64);
        let (w, r) = worker(&proxy);
        wait_until(|| control.checkpoint().state.pending == Some(1));
        drop(held);
        r.recv_timeout(Duration::from_secs(5)).unwrap();
        w.join().unwrap();
        wait_until(|| control.checkpoint().counters.callbacks_completed == 1);
        proxy.release_thread();
        let p = prefix(&control);
        assert!(p.records_complete());
        let rows = facts(&p);
        assert!(rows.contains(&Fact::Grant {
            grant_id: 0,
            request_id: 0,
            source: WireGrantSource::Helper(0)
        }));
        assert!(rows.contains(&Fact::HelperDecision {
            callback_id: 0,
            outcome: HelperOutcome::Accepted { grant_id: 0, request_id: 0 }
        }));
        assert!(
            rows.contains(&Fact::RawReleaseBegin { attempt_id: 0, parent: RawParent::Release(0) })
        );
        assert!(rows.contains(&Fact::RawReleaseEnd { attempt_id: 0, outcome: RawOutcome::Ok }));
        assert_eq!(p.end().registered_inflight.callbacks, Some(0));
        assert_eq!(client.available().unwrap(), 1);
        proxy.release_thread();
    }
    #[test]
    fn resource_pregrant_cut_freezes_without_cancelling_native_work() {
        let (proxy, _) = fixture(0);
        let control = capture(&proxy, 64);
        let (w, r) = worker(&proxy);
        wait_until(|| control.checkpoint().state.pending == Some(1));
        assert!(control.seal_prefix(CaptureCutReason::ControllerCheckpoint));
        let frozen = control.checkpoint();
        assert_eq!(frozen.state.pending, Some(1));
        assert_eq!(frozen.registered_inflight.acquire_cores_not_returned, Some(1));
        proxy.release_thread();
        r.recv_timeout(Duration::from_secs(5)).unwrap();
        w.join().unwrap();
        proxy.release_thread();
        assert!(!control.seal_prefix(CaptureCutReason::FrontendScopeReturned));
        assert_eq!(control.checkpoint(), frozen);
        let mut undersized = PreallocatedResourceExportBuffer::new(0);
        assert!(control.take_sealed_prefix(&mut undersized).is_none());
        let p = prefix(&control);
        assert_eq!(p.end(), frozen);
        assert!(p.records_complete());
        assert!(!facts(&p).iter().any(|f| matches!(
            f,
            Fact::Grant { .. } | Fact::Ack { .. } | Fact::AcquireReturned { .. }
        )));
        assert!(
            control.take_sealed_prefix(&mut PreallocatedResourceExportBuffer::new(64)).is_none()
        );
        assert_eq!(proxy.debug_snapshot().used, 0);
    }
    #[test]
    fn resource_acknowledged_core_tail_is_not_a_returned_endpoint() {
        let (proxy, _) = fixture(0);
        let control = capture(&proxy, 64);
        proxy.release_thread();
        let completion = proxy.acquire_frontend(None);
        control.seal_prefix(CaptureCutReason::ControllerCheckpoint);
        proxy.record_acquire_returned(completion);
        proxy.release_thread();
        let p = prefix(&control);
        assert_eq!(p.end().registered_inflight.acquire_cores_not_returned, Some(1));
        assert_eq!(p.end().counters.acquire_returns, 0);
        assert!(!facts(&p).iter().any(|f| matches!(f, Fact::AcquireReturned { .. })));
    }
    #[test]
    fn resource_capacity_loss_has_independent_attempted_ranges_and_counts() {
        let mut endpoints = Vec::new();
        for capacity in [0, 1, 64] {
            let (proxy, _) = fixture(0);
            let control = capture(&proxy, capacity);
            proxy.release_thread();
            proxy.acquire_thread();
            proxy.release_thread();
            let p = prefix(&control);
            assert_eq!(p.records_complete(), capacity == 64);
            assert_eq!(
                p.frozen.lost_transactions + p.transactions.len() as u64,
                p.end().counters.transactions_attempted
            );
            assert_eq!(
                p.frozen.lost_events + facts(&p).len() as u64,
                p.end().counters.events_attempted
            );
            assert_eq!(
                p.end().allocators.transaction.high_water,
                Some(p.end().counters.transactions_attempted)
            );
            assert_eq!(
                p.end().allocators.event.high_water,
                Some(p.end().counters.events_attempted)
            );
            endpoints.push(p.end());
        }
        assert_eq!(endpoints[0], endpoints[1]);
        assert_eq!(endpoints[1], endpoints[2]);
    }
    #[test]
    fn resource_identity_failure_suppresses_whole_transaction_not_native_admission() {
        let (proxy, _) = fixture(0);
        let control = capture(&proxy, 64);
        proxy.release_thread();
        proxy.state.lock().resource.as_mut().unwrap().allocators.request =
            Allocator { high_water: None, exhausted: true };
        proxy.acquire_thread();
        proxy.release_thread();
        let p = prefix(&control);
        assert!(!p.records_complete());
        assert!(p.frozen.identity_exhausted);
        assert!(p.end().counters.unidentified_events >= 3);
        assert_eq!(p.end().counters.requests, 1);
        assert_eq!(p.end().counters.acquire_returns, 1);
        assert!(p.frozen.lost_transactions >= 3);
        assert!(!facts(&p).iter().any(|f| matches!(
            f,
            Fact::Request { .. }
                | Fact::Grant { .. }
                | Fact::Ack { .. }
                | Fact::AcquireReturned { .. }
        )));
        assert_eq!(proxy.debug_snapshot().used, 0);
    }
    #[test]
    fn resource_counter_exhaustion_is_sticky_and_diagnostics_only() {
        let (proxy, _) = fixture(0);
        let control = capture(&proxy, 64);
        proxy.state.lock().resource.as_mut().unwrap().counters.requests = u64::MAX;
        proxy.release_thread();
        proxy.acquire_thread();
        proxy.release_thread();
        let p = prefix(&control);
        assert!(p.frozen.counter_overflowed);
        assert!(!p.records_complete());
        assert_eq!(p.end().counters.requests, u64::MAX);
        assert_eq!(p.end().counters.acks, 1);
        assert_eq!(proxy.debug_snapshot().used, 0);
    }
    #[test]
    fn resource_late_attachment_does_not_fabricate_inflight_zero() {
        let (proxy, _) = fixture(0);
        proxy.release_thread();
        let control = capture(&proxy, 64);
        assert_eq!(control.checkpoint().registered_inflight, RegisteredInflight::default());
        proxy.acquire_thread();
        proxy.release_thread();
        let p = prefix(&control);
        assert_eq!(p.start().registered_inflight, RegisteredInflight::default());
        assert_eq!(p.end().registered_inflight, RegisteredInflight::default());
    }
    #[test]
    fn absent_resource_acquire_return_does_not_take_state() {
        let (proxy, _) = fixture(0);
        proxy.release_thread();
        let completion = proxy.acquire_frontend(None);
        let _state = proxy.state.lock();
        // Would deadlock if the absent-resource adapter added a State lock.
        proxy.record_acquire_returned(completion);
    }
    #[test]
    fn resource_owner_registration_validates_actual_controller_and_factory() {
        use rustc_thread_pool::admission::{AdmissionAttachmentId, QueryRootNamespace};
        use std::num::NonZeroU64;
        let (proxy, _) = fixture(0);
        let control = proxy.prepare_resource_capture(ResourceCaptureConfig::new(16)).unwrap();
        let other_wrapper = control.clone();
        assert!(other_wrapper.belongs_to(&proxy));
        assert_eq!(control.controller_id(), other_wrapper.controller_id());
        assert!(proxy.prepare_resource_capture(ResourceCaptureConfig::new(16)).is_none());
        let attachment = AdmissionAttachmentId::mint().unwrap();
        control.expect_attachment(Some(attachment)).unwrap();
        let descriptor = ResourceAttachmentDescriptor::new(
            Arc::from(control.attachment_capture_id()),
            attachment,
            Arc::from(control.controller_id().as_str()),
            Arc::from(control.capture_id()),
            Arc::from("query-exact-capture"),
            QueryRootNamespace::mint().unwrap(),
            Arc::from("pool-exact-capture"),
            NonZeroU64::new(7).unwrap(),
        )
        .unwrap();
        let token = control.bind(descriptor.clone()).unwrap();
        let mut output = Vec::new();
        token.write_json(&mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("\"role\":\"resource\""));
        assert!(text.contains(control.capture_id()));
        assert_ne!(token.capture_id(), control.capture_id());
        assert_ne!(token.capture_id(), control.attachment_capture_id());
        assert!(matches!(control.bind(descriptor), Err(BindFailure::Conflict)));
        let mut rejected = Vec::new();
        assert!(token.write_json(&mut rejected).is_err());
        assert!(rejected.is_empty());
        assert_eq!(control.registration_snapshot().attempts, 2);
        assert_eq!(control.registration_snapshot().successes, 1);
        assert_eq!(control.registration_snapshot().conflicts, 1);
        proxy.release_thread();
    }
    #[test]
    fn resource_reverse_ack_preserves_the_actual_selected_completion_ids() {
        let (proxy, _client, held) = supply_fixture();
        let control = capture(&proxy, 64);
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
        wait_until(|| control.checkpoint().state.pending == Some(1));
        let (w2, r2) = worker(&proxy);
        wait_until(|| control.checkpoint().state.pending == Some(2));
        proxy.release_thread();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(held);
        r2.recv_timeout(Duration::from_secs(5)).unwrap();
        w2.join().unwrap();
        assert_eq!(control.checkpoint().state.granted_unacked, Some(1));
        gate.wait();
        r1.recv_timeout(Duration::from_secs(5)).unwrap();
        w1.join().unwrap();
        let p = prefix(&control);
        assert!(p.records_complete());
        let rows = facts(&p);
        assert!(rows.contains(&Fact::AcquireReturned {
            return_id: 0,
            request_id: 1,
            grant_id: 1,
            ack_id: 0
        }));
        assert!(rows.contains(&Fact::AcquireReturned {
            return_id: 1,
            request_id: 0,
            grant_id: 0,
            ack_id: 1
        }));
        assert_eq!(p.end().state.granted_unacked, Some(0));
        proxy.release_thread();
        proxy.release_thread();
    }
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
        assert_eq!(slot.phase.lock().phase, Phase::Pending);
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
            assert_eq!(slot.phase.lock().phase, Phase::Pending);
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
            phase: Mutex::new(SlotState {
                phase: Phase::Pending,
                wire: AcquireCompletion::default(),
            }),
            wake: Condvar::new(),
        });
        proxy.state.lock().push(slot.clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            TicketedProxy::helper_supply(&proxy.state);
        }));
        assert!(result.is_err());
        let mut state = proxy.state.lock();
        assert_eq!(state.pending, 1);
        assert_eq!(slot.phase.lock().phase, Phase::Pending);
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
            phase: Mutex::new(SlotState {
                phase: Phase::Pending,
                wire: AcquireCompletion::default(),
            }),
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
