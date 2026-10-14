//! Bounded diagnostic facts for one selected frontend allowance prefix.
//! This is not token ownership, helper termination, or execution-time telemetry.
use std::fmt::Write;
use std::io;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use rustc_thread_pool::admission::{AdmissionAttachmentId, AdmissionContext, AdmissionPhase};

use super::ticketed::State;
use parking_lot::Mutex;

static NEXT_CAPTURE: AtomicU64 = AtomicU64::new(1);
static PROCESS_NONCE: OnceLock<Option<String>> = OnceLock::new();
/// Four facts per transaction fit the consumer's 100,000-row prefix budget.
pub const MAX_RESOURCE_TRANSACTIONS: usize = 25_000;
fn next_capture() -> Option<u64> {
    let mut current = NEXT_CAPTURE.load(Ordering::Relaxed);
    loop {
        let next = current.checked_add(1)?;
        match NEXT_CAPTURE.compare_exchange_weak(
            current,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Some(current),
            Err(actual) => current = actual,
        }
    }
}
fn unique(prefix: &str) -> Option<String> {
    // Preparation and binding only, never called by native capture hooks.
    let nonce = PROCESS_NONCE
        .get_or_init(|| {
            let mut bytes = [0u8; 16];
            std::fs::File::open("/dev/urandom").ok()?.read_exact(&mut bytes).ok()?;
            let mut text = String::with_capacity(32);
            for byte in bytes {
                write!(&mut text, "{byte:02x}").ok()?;
            }
            Some(text)
        })
        .as_ref()?;
    let value = next_capture()?;
    Some(format!("{prefix}-{nonce}-{value}"))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerId(String);
impl ControllerId {
    pub(super) fn mint() -> Option<Self> {
        unique("controller").map(Self)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
use rustc_thread_pool::admission_resource::{
    AttachmentEndorsement, EndorsementGate, EndorsementRole, RegistrationSnapshot,
};
pub use rustc_thread_pool::admission_resource::{
    BindFailure, PreparedDescriptor as ResourceAttachmentDescriptor,
};
#[derive(Clone, Debug)]
pub struct ResourceEndorsementToken {
    endorsement: AttachmentEndorsement,
    gate: EndorsementGate,
}
impl ResourceEndorsementToken {
    pub fn capture_id(&self) -> &str {
        self.endorsement.capture_id()
    }
    pub fn write_json(&self, output: &mut impl io::Write) -> io::Result<()> {
        if !self.gate.is_complete() {
            return Err(io::Error::other("resource registration incomplete"));
        }
        self.endorsement.write_json(output)
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ResourceCaptureConfig {
    pub transaction_capacity: usize,
    pub expected_attachments: Option<u8>,
}
impl ResourceCaptureConfig {
    pub fn new(transaction_capacity: usize) -> Self {
        Self { transaction_capacity, expected_attachments: Some(1) }
    }
}
pub(super) struct PreparedCapture {
    pub controller_id: ControllerId,
    pub capture_id: String,
    pub attachment_capture_id: String,
    pub expected_attachments: Option<u8>,
    pub attachment: OnceLock<Option<AdmissionAttachmentId>>,
    pub descriptor: OnceLock<ResourceAttachmentDescriptor>,
    pub capacity: usize,
    pub gate: EndorsementGate,
}
impl PreparedCapture {
    pub(super) fn new(controller_id: ControllerId, config: ResourceCaptureConfig) -> Option<Arc<Self>> {
        if !matches!(config.expected_attachments, None | Some(0 | 1))
            || config.transaction_capacity > MAX_RESOURCE_TRANSACTIONS
        {
            return None;
        }
        Some(Arc::new(Self {
            controller_id,
            capture_id: unique("resource")?,
            attachment_capture_id: unique("attachment")?,
            expected_attachments: config.expected_attachments,
            attachment: OnceLock::new(),
            descriptor: OnceLock::new(),
            capacity: config.transaction_capacity,
            gate: EndorsementGate::default(),
        }))
    }
}
#[derive(Clone)]
pub struct ResourceCaptureControl {
    pub(super) state: Arc<Mutex<State>>,
    pub(super) prepared: Arc<PreparedCapture>,
}
impl ResourceCaptureControl {
    pub fn controller_id(&self) -> &ControllerId {
        &self.prepared.controller_id
    }
    pub fn capture_id(&self) -> &str {
        &self.prepared.capture_id
    }
    pub fn attachment_capture_id(&self) -> &str {
        &self.prepared.attachment_capture_id
    }
    /// Called by the actual frontend wrapper, before workers start.
    pub fn expect_attachment(&self, id: Option<AdmissionAttachmentId>) -> Result<(), BindFailure> {
        self.prepared.attachment.set(id).map_err(|_| {
            self.prepared.gate.failed();
            BindFailure::Conflict
        })
    }
    pub fn transaction_capacity(&self) -> usize {
        self.prepared.capacity
    }
    pub fn registration_snapshot(&self) -> RegistrationSnapshot {
        self.prepared.gate.snapshot()
    }
    pub fn bind(
        &self,
        descriptor: ResourceAttachmentDescriptor,
    ) -> Result<ResourceEndorsementToken, BindFailure> {
        self.prepared.gate.begin_attempt()?;
        let result = self.bind_first(descriptor);
        if result.is_ok() {
            self.prepared.gate.succeeded();
        } else {
            self.prepared.gate.failed();
        }
        result
    }
    fn bind_first(
        &self,
        descriptor: ResourceAttachmentDescriptor,
    ) -> Result<ResourceEndorsementToken, BindFailure> {
        if descriptor.attachment_capture_id().as_ref() != self.prepared.attachment_capture_id
            || descriptor.controller_id().as_ref() != self.prepared.controller_id.as_str()
            || descriptor.resource_capture_id().as_ref() != self.prepared.capture_id
            || self.prepared.attachment.get().copied().flatten() != Some(descriptor.attachment_id())
            || self.prepared.expected_attachments != Some(1)
        {
            return Err(BindFailure::WrongOwner);
        }
        let capture_id = unique("resource-endorsement").ok_or(BindFailure::IdentityExhausted)?;
        let endorsement = AttachmentEndorsement::new(
            EndorsementRole::Resource,
            Arc::from(capture_id),
            descriptor.clone(),
        )?;
        self.prepared.descriptor.set(descriptor).map_err(|_| BindFailure::Conflict)?;
        Ok(ResourceEndorsementToken { endorsement, gate: self.prepared.gate.clone() })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureCutReason {
    FrontendScopeReturned,
    FrontendScopeUnwound,
    ControllerCheckpoint,
    Unknown,
}
impl CaptureCutReason {
    fn wire(self) -> &'static str {
        match self {
            Self::FrontendScopeReturned => "frontend_scope_returned",
            Self::FrontendScopeUnwound => "frontend_scope_unwound",
            Self::ControllerCheckpoint => "controller_checkpoint",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Aggregate {
    pub assigned: Option<u64>,
    pub pending: Option<u64>,
    pub granted_unacked: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocator {
    pub high_water: Option<u64>,
    pub exhausted: bool,
}
impl Default for Allocator {
    fn default() -> Self {
        Self { high_water: Some(0), exhausted: false }
    }
}
impl Allocator {
    fn mint(&mut self) -> Option<u64> {
        let current = self.high_water;
        self.high_water = current.and_then(|v| v.checked_add(1));
        self.exhausted |= self.high_water.is_none();
        current
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Allocators {
    pub request: Allocator,
    pub release: Allocator,
    pub grant: Allocator,
    pub ack: Allocator,
    pub acquire_return: Allocator,
    pub helper_issuance: Allocator,
    pub callback: Allocator,
    pub raw_release: Allocator,
    pub transaction: Allocator,
    pub event: Allocator,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CounterSet {
    pub requests: u64,
    pub pending_registrations: u64,
    pub idle_grants: u64,
    pub transfer_grants: u64,
    pub helper_grants: u64,
    pub acks: u64,
    pub acquire_returns: u64,
    pub release_calls: u64,
    pub release_transfers: u64,
    pub release_retains: u64,
    pub release_raw_obligations: u64,
    pub helper_issuances: u64,
    pub callbacks_registered: u64,
    pub callbacks_accepted: u64,
    pub callbacks_surplus: u64,
    pub callbacks_error: u64,
    pub callbacks_completed: u64,
    pub raw_release_begins: u64,
    pub raw_release_ok: u64,
    pub raw_release_error: u64,
    pub raw_release_unknown: u64,
    pub unqualified_requests: u64,
    pub unqualified_releases: u64,
    pub unidentified_events: u64,
    pub transactions_attempted: u64,
    pub events_attempted: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegisteredInflight {
    pub callbacks: Option<u64>,
    pub raw_release_attempts: Option<u64>,
    pub acquire_cores_not_returned: Option<u64>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixedCheckpoint {
    pub state: Aggregate,
    pub allocators: Allocators,
    pub counters: CounterSet,
    pub registered_inflight: RegisteredInflight,
}
/// Diagnostic tuple, not an allowance, holder, or release capability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AcquireCompletion {
    pub(super) capture: Option<u64>,
    pub(super) request: Option<u64>,
    pub(super) grant: Option<u64>,
    pub(super) ack: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    BrokenPipe,
    Interrupted,
    InvalidInput,
    WouldBlock,
    PermissionDenied,
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeError {
    pub error_kind: ErrorKind,
    pub raw_os_error: Option<i32>,
}
impl NativeError {
    pub fn observe(error: &io::Error) -> Self {
        Self {
            error_kind: match error.kind() {
                io::ErrorKind::BrokenPipe => ErrorKind::BrokenPipe,
                io::ErrorKind::Interrupted => ErrorKind::Interrupted,
                io::ErrorKind::InvalidInput => ErrorKind::InvalidInput,
                io::ErrorKind::WouldBlock => ErrorKind::WouldBlock,
                io::ErrorKind::PermissionDenied => ErrorKind::PermissionDenied,
                _ => ErrorKind::Other,
            },
            raw_os_error: error.raw_os_error(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireGrantSource {
    IdleImplicit,
    Release(u64),
    Helper(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseDisposition {
    Transfer { grant_id: u64, request_id: u64 },
    RetainImplicit,
    RawRelease { attempt_id: u64 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperOutcome {
    Accepted { grant_id: u64, request_id: u64 },
    Surplus { attempt_id: u64 },
    Error(NativeError),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackDisposition {
    Consumed,
    SurplusDropFinished,
    Error,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawParent {
    Release(u64),
    HelperSurplus(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawOutcome {
    Ok,
    Error(NativeError),
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fact {
    CaptureStart,
    CaptureCut,
    Request {
        request_id: u64,
        context: Option<AdmissionContext>,
    },
    Pending {
        request_id: u64,
    },
    Grant {
        grant_id: u64,
        request_id: u64,
        source: WireGrantSource,
    },
    Ack {
        ack_id: u64,
        request_id: u64,
        grant_id: u64,
    },
    AcquireReturned {
        return_id: u64,
        request_id: u64,
        grant_id: u64,
        ack_id: u64,
    },
    ReleaseDecision {
        release_id: u64,
        context: Option<AdmissionContext>,
        disposition: ReleaseDisposition,
    },
    HelperIssued {
        issuance_id: u64,
        trigger_request_id: u64,
    },
    CallbackRegistered {
        callback_id: u64,
    },
    HelperDecision {
        callback_id: u64,
        outcome: HelperOutcome,
    },
    CallbackComplete {
        callback_id: u64,
        disposition: CallbackDisposition,
    },
    RawReleaseBegin {
        attempt_id: u64,
        parent: RawParent,
    },
    RawReleaseEnd {
        attempt_id: u64,
        outcome: RawOutcome,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub fact: Fact,
}
#[derive(Clone, Copy, Debug)]
pub struct FixedCopyTxn {
    pub transaction_id: u64,
    pub before: Aggregate,
    pub after: Aggregate,
    pub first_seq: u64,
    pub next_seq: u64,
    pub events: [Option<Event>; 4],
}
/// Preallocated before workers start. No replacement allocation at take time.
pub struct PreallocatedResourceExportBuffer {
    pub(super) transactions: Vec<FixedCopyTxn>,
}
impl PreallocatedResourceExportBuffer {
    pub fn new(capacity: usize) -> Self {
        let mut transactions = Vec::new();
        if capacity <= MAX_RESOURCE_TRANSACTIONS {
            // An unavailable replacement fails take_sealed_prefix's capacity
            // check, never native admission or a later under-lock allocation.
            let _ = transactions.try_reserve_exact(capacity);
        }
        Self { transactions }
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Frozen {
    pub start: FixedCheckpoint,
    pub end: FixedCheckpoint,
    pub reason: CaptureCutReason,
    pub identity_exhausted: bool,
    pub counter_overflowed: bool,
    pub lost_transactions: u64,
    pub lost_events: u64,
    pub loss_overflowed: bool,
}
pub struct PrefixFacts {
    pub(super) prepared: Arc<PreparedCapture>,
    pub(super) frozen: Frozen,
    pub transactions: Vec<FixedCopyTxn>,
}
/// All mutation is protected by the native controller's existing State guard.
pub(super) struct Ledger {
    pub key: u64,
    pub allocators: Allocators,
    pub counters: CounterSet,
    pub journal: Vec<FixedCopyTxn>,
    pub capacity: usize,
    pub start: FixedCheckpoint,
    pub frozen: Option<Frozen>,
    pub taken: bool,
    pub identity_exhausted: bool,
    pub counter_overflowed: bool,
    pub lost_transactions: u64,
    pub lost_events: u64,
    pub loss_overflowed: bool,
    pub inflight_known: bool,
}
pub(super) struct Txn {
    before: Aggregate,
    facts: [Option<Fact>; 4],
    len: usize,
}
impl Txn {
    pub(super) fn new(before: Aggregate) -> Self {
        Self { before, facts: [None; 4], len: 0 }
    }
    pub(super) fn push(&mut self, fact: Option<Fact>) {
        assert!(self.len < 4);
        self.facts[self.len] = fact;
        self.len += 1;
    }
}
fn bump(value: &mut u64, overflow: &mut bool) {
    if let Some(v) = value.checked_add(1) {
        *value = v;
    } else {
        *overflow = true;
    }
}
impl Ledger {
    pub(super) fn new(config: ResourceCaptureConfig) -> Option<Self> {
        if config.transaction_capacity > MAX_RESOURCE_TRANSACTIONS {
            return None;
        }
        let mut journal = Vec::new();
        journal.try_reserve_exact(config.transaction_capacity).ok()?;
        let key = next_capture()?;
        Some(Self {
            key,
            allocators: Allocators::default(),
            counters: CounterSet::default(),
            journal,
            capacity: config.transaction_capacity,
            start: FixedCheckpoint::default(),
            frozen: None,
            taken: false,
            identity_exhausted: false,
            counter_overflowed: false,
            lost_transactions: 0,
            lost_events: 0,
            loss_overflowed: false,
            inflight_known: true,
        })
    }
    pub(super) fn active(&self) -> bool {
        self.frozen.is_none()
    }
    pub(super) fn checkpoint(&self, state: Aggregate) -> FixedCheckpoint {
        let c = self.counters;
        FixedCheckpoint {
            state,
            allocators: self.allocators,
            counters: c,
            registered_inflight: RegisteredInflight {
                callbacks: if self.counter_overflowed || !self.inflight_known {
                    None
                } else {
                    c.callbacks_registered.checked_sub(c.callbacks_completed)
                },
                raw_release_attempts: if self.counter_overflowed || !self.inflight_known {
                    None
                } else {
                    c.raw_release_begins
                        .checked_sub(c.raw_release_ok)
                        .and_then(|v| v.checked_sub(c.raw_release_error))
                        .and_then(|v| v.checked_sub(c.raw_release_unknown))
                },
                acquire_cores_not_returned: if self.counter_overflowed || !self.inflight_known {
                    None
                } else {
                    c.requests.checked_sub(c.acquire_returns)
                },
            },
        }
    }
    pub(super) fn count(&mut self, field: fn(&mut CounterSet) -> &mut u64) {
        if self.active() {
            bump(field(&mut self.counters), &mut self.counter_overflowed);
        }
    }
    pub(super) fn id(&mut self, field: fn(&mut Allocators) -> &mut Allocator) -> Option<u64> {
        if !self.active() {
            return None;
        }
        let allocator = field(&mut self.allocators);
        let result = allocator.mint();
        self.identity_exhausted |= allocator.exhausted;
        result
    }
    pub(super) fn finish(&mut self, txn: Txn, after: Aggregate) {
        if !self.active() {
            return;
        }
        self.count(|c| &mut c.transactions_attempted);
        let transaction_id = self.id(|a| &mut a.transaction);
        let first_seq = self.allocators.event.high_water;
        let mut events = [None; 4];
        let mut valid = transaction_id.is_some() && first_seq.is_some();
        for (i, fact) in txn.facts[..txn.len].iter().copied().enumerate() {
            self.count(|c| &mut c.events_attempted);
            let seq = self.id(|a| &mut a.event);
            if fact.is_none() || seq.is_none() || transaction_id.is_none() {
                self.count(|c| &mut c.unidentified_events);
                self.identity_exhausted = true;
            }
            events[i] = seq.zip(fact).map(|(seq, fact)| Event { seq, fact });
            valid &= events[i].is_some();
        }
        let next_seq = self.allocators.event.high_water;
        valid &= next_seq.is_some();
        if valid && self.journal.len() < self.capacity {
            self.journal.push(FixedCopyTxn {
                transaction_id: transaction_id.unwrap(),
                before: txn.before,
                after,
                first_seq: first_seq.unwrap(),
                next_seq: next_seq.unwrap(),
                events,
            });
        } else {
            bump(&mut self.lost_transactions, &mut self.loss_overflowed);
            for _ in 0..txn.len {
                bump(&mut self.lost_events, &mut self.loss_overflowed);
            }
        }
    }
}

fn number<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "null".into(), |v| v.to_string())
}
fn aggregate_json(a: Aggregate) -> String {
    format!(
        "{{\"assigned\":{},\"pending\":{},\"granted_unacked\":{}}}",
        number(a.assigned),
        number(a.pending),
        number(a.granted_unacked)
    )
}
macro_rules! scalar_fields {
    ($out:expr, $value:expr, $($field:ident),+ $(,)?) => {{
        let mut separator = "";
        $(write!($out, "{}\"{}\":{}", separator, stringify!($field), $value.$field).unwrap(); separator = ",";)+
        let _ = separator;
    }};
}
fn checkpoint_json(c: FixedCheckpoint, label: &str) -> String {
    let mut out =
        format!("{{\"label\":\"{label}\",\"state\":{},\"allocators\":{{", aggregate_json(c.state));
    let mut separator = "";
    macro_rules! allocators {
        ($($field:ident),+) => {$(
            write!(&mut out, "{}\"{}\":{{\"baseline\":0,\"high_water\":{},\"exhausted\":{}}}", separator, stringify!($field), number(c.allocators.$field.high_water), c.allocators.$field.exhausted).unwrap();
            separator = ",";
        )+};
    }
    allocators!(
        request,
        release,
        grant,
        ack,
        acquire_return,
        helper_issuance,
        callback,
        raw_release,
        transaction,
        event
    );
    let _ = separator;
    out.push_str("},\"counters\":{");
    scalar_fields!(
        &mut out,
        c.counters,
        requests,
        pending_registrations,
        idle_grants,
        transfer_grants,
        helper_grants,
        acks,
        acquire_returns,
        release_calls,
        release_transfers,
        release_retains,
        release_raw_obligations,
        helper_issuances,
        callbacks_registered,
        callbacks_accepted,
        callbacks_surplus,
        callbacks_error,
        callbacks_completed,
        raw_release_begins,
        raw_release_ok,
        raw_release_error,
        raw_release_unknown,
        unqualified_requests,
        unqualified_releases,
        unidentified_events,
        transactions_attempted,
        events_attempted
    );
    write!(&mut out, "}},\"registered_inflight\":{{\"callbacks\":{},\"raw_release_attempts\":{},\"acquire_cores_not_returned\":{}}}}}", number(c.registered_inflight.callbacks), number(c.registered_inflight.raw_release_attempts), number(c.registered_inflight.acquire_cores_not_returned)).unwrap();
    out
}
fn context_json(context: Option<AdmissionContext>) -> String {
    match context {
        None => "\"context\":null,\"unavailable_reason\":\"not_supplied\"".into(),
        Some(c) => format!(
            "\"context\":{{\"attachment_id\":{},\"query_root_namespace\":{},\"wait_id\":{},\"phase\":\"{}\"}},\"unavailable_reason\":null",
            c.attachment_id.wire_value(),
            c.qualified_wait_id.query_root.wire_value(),
            c.qualified_wait_id.wait_id,
            match c.phase {
                AdmissionPhase::QueryWaitRelease => "query_wait_release",
                AdmissionPhase::QueryWaitReacquire => "query_wait_reacquire",
            }
        ),
    }
}
fn error_json(error: NativeError) -> String {
    let kind = match error.error_kind {
        ErrorKind::BrokenPipe => "broken_pipe",
        ErrorKind::Interrupted => "interrupted",
        ErrorKind::InvalidInput => "invalid_input",
        ErrorKind::WouldBlock => "would_block",
        ErrorKind::PermissionDenied => "permission_denied",
        ErrorKind::Other => "other",
    };
    format!("\"error_kind\":\"{kind}\",\"raw_os_error\":{}", number(error.raw_os_error))
}
fn fact_json(event: Event) -> String {
    let fields = match event.fact {
        Fact::CaptureStart => "\"kind\":\"capture_start\"".into(),
        Fact::CaptureCut => "\"kind\":\"capture_cut\"".into(),
        Fact::Request { request_id, context } => {
            format!("\"kind\":\"request\",\"request_id\":{request_id},{}", context_json(context))
        }
        Fact::Pending { request_id } => format!("\"kind\":\"pending\",\"request_id\":{request_id}"),
        Fact::Grant { grant_id, request_id, source } => {
            let source = match source {
                WireGrantSource::IdleImplicit => "\"kind\":\"idle_implicit\"".into(),
                WireGrantSource::Release(id) => format!("\"kind\":\"release\",\"release_id\":{id}"),
                WireGrantSource::Helper(id) => format!("\"kind\":\"helper\",\"callback_id\":{id}"),
            };
            format!(
                "\"kind\":\"grant\",\"grant_id\":{grant_id},\"request_id\":{request_id},\"source\":{{{source}}}"
            )
        }
        Fact::Ack { ack_id, request_id, grant_id } => format!(
            "\"kind\":\"ack\",\"ack_id\":{ack_id},\"request_id\":{request_id},\"grant_id\":{grant_id}"
        ),
        Fact::AcquireReturned { return_id, request_id, grant_id, ack_id } => format!(
            "\"kind\":\"acquire_returned\",\"return_id\":{return_id},\"request_id\":{request_id},\"grant_id\":{grant_id},\"ack_id\":{ack_id}"
        ),
        Fact::ReleaseDecision { release_id, context, disposition } => {
            let disposition = match disposition {
                ReleaseDisposition::Transfer { grant_id, request_id } => format!(
                    "\"kind\":\"transfer\",\"grant_id\":{grant_id},\"request_id\":{request_id}"
                ),
                ReleaseDisposition::RetainImplicit => "\"kind\":\"retain_implicit\"".into(),
                ReleaseDisposition::RawRelease { attempt_id } => {
                    format!("\"kind\":\"raw_release\",\"attempt_id\":{attempt_id}")
                }
            };
            format!(
                "\"kind\":\"release_decision\",\"release_id\":{release_id},{},\"disposition\":{{{disposition}}}",
                context_json(context)
            )
        }
        Fact::HelperIssued { issuance_id, trigger_request_id } => format!(
            "\"kind\":\"helper_issued\",\"issuance_id\":{issuance_id},\"trigger_request_id\":{trigger_request_id}"
        ),
        Fact::CallbackRegistered { callback_id } => {
            format!("\"kind\":\"callback_registered\",\"callback_id\":{callback_id}")
        }
        Fact::HelperDecision { callback_id, outcome } => {
            let outcome = match outcome {
                HelperOutcome::Accepted { grant_id, request_id } => format!(
                    "\"kind\":\"accepted\",\"grant_id\":{grant_id},\"request_id\":{request_id}"
                ),
                HelperOutcome::Surplus { attempt_id } => {
                    format!("\"kind\":\"surplus\",\"attempt_id\":{attempt_id}")
                }
                HelperOutcome::Error(error) => format!("\"kind\":\"error\",{}", error_json(error)),
            };
            format!(
                "\"kind\":\"helper_decision\",\"callback_id\":{callback_id},\"outcome\":{{{outcome}}}"
            )
        }
        Fact::CallbackComplete { callback_id, disposition } => format!(
            "\"kind\":\"callback_complete\",\"callback_id\":{callback_id},\"disposition\":\"{}\"",
            match disposition {
                CallbackDisposition::Consumed => "consumed",
                CallbackDisposition::SurplusDropFinished => "surplus_drop_finished",
                CallbackDisposition::Error => "error",
            }
        ),
        Fact::RawReleaseBegin { attempt_id, parent } => {
            let (parent, authority) = match parent {
                RawParent::Release(id) => {
                    (format!("\"kind\":\"release\",\"release_id\":{id}"), "native_result")
                }
                RawParent::HelperSurplus(id) => {
                    (format!("\"kind\":\"helper_surplus\",\"callback_id\":{id}"), "opaque_drop")
                }
            };
            format!(
                "\"kind\":\"raw_release_begin\",\"attempt_id\":{attempt_id},\"parent\":{{{parent}}},\"result_authority\":\"{authority}\""
            )
        }
        Fact::RawReleaseEnd { attempt_id, outcome } => {
            let outcome = match outcome {
                RawOutcome::Ok => "\"kind\":\"ok\"".into(),
                RawOutcome::Error(error) => format!("\"kind\":\"error\",{}", error_json(error)),
                RawOutcome::Unknown => {
                    "\"kind\":\"unknown\",\"reason\":\"acquired_drop_discards_result\"".into()
                }
            };
            format!(
                "\"kind\":\"raw_release_end\",\"attempt_id\":{attempt_id},\"outcome\":{{{outcome}}}"
            )
        }
    };
    format!("{{\"seq\":{},{fields}}}", event.seq)
}
impl PrefixFacts {
    pub fn start(&self) -> FixedCheckpoint {
        self.frozen.start
    }
    pub fn end(&self) -> FixedCheckpoint {
        self.frozen.end
    }
    pub fn records_complete(&self) -> bool {
        let f = self.frozen;
        !f.identity_exhausted
            && !f.counter_overflowed
            && !f.loss_overflowed
            && f.lost_transactions == 0
            && f.lost_events == 0
            && f.end.state.granted_unacked.is_some()
    }
    /// Pure serialization of the immutable prefix, outside all native locks.
    pub fn to_json(&self) -> String {
        let f = self.frozen;
        let attachment = if self.prepared.expected_attachments == Some(1) {
            format!("\"{}\"", self.prepared.attachment_capture_id)
        } else {
            "null".into()
        };
        let mut out = format!(
            "{{\"schema\":1,\"producer\":\"rustc-frontend-allowance\",\"scope\":\"selected-ticketed-grant-decisions\",\"capture_id\":\"{}\",\"controller_id\":\"{}\",\"capture_state\":\"sealed_prefix\",\"attachment_capture_id\":{},\"expected_attachments\":{},\"full_pipeline_complete\":false,\"resource_full_complete\":false,\"producer_closed\":false,\"raw_io_coverage\":false,\"cpu_work_ns\":null,\"execution_span_ns\":null,\"pre_attach_coverage\":false,\"initial_assignment_origin\":\"opaque\",\"physical_callbacks_inflight\":null,\"helper_issuance_callback_pairing\":\"unknown\",\"cut_reason\":\"{}\",\"start\":{},\"end\":{},\"transactions\":[",
            self.prepared.capture_id,
            self.prepared.controller_id.0,
            attachment,
            number(self.prepared.expected_attachments),
            f.reason.wire(),
            checkpoint_json(f.start, "start"),
            checkpoint_json(f.end, "end")
        );
        for (i, txn) in self.transactions.iter().enumerate() {
            if i != 0 {
                out.push(',');
            }
            write!(&mut out, "{{\"transaction_id\":{},\"before\":{},\"after\":{},\"first_seq\":{},\"next_seq\":{},\"events\":[", txn.transaction_id, aggregate_json(txn.before), aggregate_json(txn.after), txn.first_seq, txn.next_seq).unwrap();
            for (i, event) in txn.events.iter().flatten().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                out.push_str(&fact_json(*event));
            }
            out.push_str("]}");
        }
        write!(&mut out, "],\"records_complete\":{},\"identity_exhausted\":{},\"counter_overflowed\":{},\"lost_transactions\":{},\"lost_events\":{},\"loss_overflowed\":{},\"opaque_boundaries\":[\"pre-attach-history\",\"initial-assignment-owner\",\"unqualified-calls\",\"helper-demand-delivery-pairing\",\"physical-helper-entries\",\"helper-lifecycle-tail\",\"surplus-release-result\",\"external-jobserver-origin\",\"native-token-holders\",\"backend-and-other-pools\",\"post-cutoff-activity\",\"nonquery-scheduling\",\"cpu-and-span\"]}}", self.records_complete(), f.identity_exhausted, f.counter_overflowed, f.lost_transactions, f.lost_events, f.loss_overflowed).unwrap();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excessive_storage_request_is_rejected_before_allocation() {
        assert!(Ledger::new(ResourceCaptureConfig::new(usize::MAX)).is_none());
        assert_eq!(PreallocatedResourceExportBuffer::new(usize::MAX).transactions.capacity(), 0);
    }
    #[test]
    fn allocator_max_is_issued_once_with_sticky_unknown_high_water() {
        let mut allocator = Allocator { high_water: Some(u64::MAX), exhausted: false };
        assert_eq!(allocator.mint(), Some(u64::MAX));
        assert_eq!(allocator.high_water, None);
        assert!(allocator.exhausted);
        assert_eq!(allocator.mint(), None);
        assert_eq!(allocator.mint(), None);
    }
    #[test]
    fn missing_envelope_or_event_identity_counts_unidentified_and_whole_loss() {
        for event_identity in [false, true] {
            let mut ledger = Ledger::new(ResourceCaptureConfig::new(4)).unwrap();
            if event_identity {
                ledger.allocators.event.high_water = None;
            } else {
                ledger.allocators.transaction.high_water = None;
            }
            let mut txn = Txn::new(Aggregate::default());
            txn.push(Some(Fact::CaptureStart));
            txn.push(Some(Fact::CaptureCut));
            ledger.finish(txn, Aggregate::default());
            assert!(ledger.journal.is_empty());
            assert_eq!(ledger.counters.transactions_attempted, 1);
            assert_eq!(ledger.counters.events_attempted, 2);
            assert_eq!(ledger.counters.unidentified_events, 2);
            assert_eq!(ledger.lost_transactions, 1);
            assert_eq!(ledger.lost_events, 2);
            assert!(ledger.identity_exhausted);
        }
    }
    #[test]
    fn four_fixed_facts_are_retained_or_lost_as_one_envelope() {
        for capacity in [0, 1] {
            let mut ledger = Ledger::new(ResourceCaptureConfig::new(capacity)).unwrap();
            let mut txn = Txn::new(Aggregate::default());
            for _ in 0..4 {
                txn.push(Some(Fact::CaptureStart));
            }
            ledger.finish(txn, Aggregate::default());
            assert_eq!(ledger.journal.len(), capacity);
            assert_eq!(ledger.counters.events_attempted, 4);
            assert_eq!(ledger.allocators.event.high_water, Some(4));
            assert_eq!(ledger.lost_events, if capacity == 0 { 4 } else { 0 });
        }
    }
    #[test]
    fn loss_saturation_is_a_sticky_lower_bound() {
        let mut ledger = Ledger::new(ResourceCaptureConfig::new(0)).unwrap();
        ledger.lost_events = u64::MAX;
        ledger.lost_transactions = u64::MAX;
        let mut txn = Txn::new(Aggregate::default());
        txn.push(Some(Fact::CaptureStart));
        ledger.finish(txn, Aggregate::default());
        assert!(ledger.loss_overflowed);
        assert_eq!(ledger.lost_events, u64::MAX);
        assert_eq!(ledger.lost_transactions, u64::MAX);
        assert_eq!(ledger.counters.events_attempted, 1);
    }
    #[test]
    fn error_mapping_is_closed_and_does_not_claim_native_io_injection() {
        for (native, wire) in [
            (io::ErrorKind::BrokenPipe, ErrorKind::BrokenPipe),
            (io::ErrorKind::Interrupted, ErrorKind::Interrupted),
            (io::ErrorKind::InvalidInput, ErrorKind::InvalidInput),
            (io::ErrorKind::WouldBlock, ErrorKind::WouldBlock),
            (io::ErrorKind::PermissionDenied, ErrorKind::PermissionDenied),
            (io::ErrorKind::NotFound, ErrorKind::Other),
        ] {
            let observed = NativeError::observe(&io::Error::from(native));
            assert_eq!(observed.error_kind, wire);
            assert_eq!(observed.raw_os_error, None);
        }
        let error = io::Error::from_raw_os_error(13);
        assert_eq!(NativeError::observe(&error).raw_os_error, Some(13));
    }
}
