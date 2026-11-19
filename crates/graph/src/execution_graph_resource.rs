//! Strict, bounded-prefix frontend allowance facts. Never a token-holder or span graph.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_FACTS: usize = 100_000;
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
pub(crate) fn safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct WireContext {
    pub attachment_id: u64,
    pub query_root_namespace: u64,
    pub wait_id: u64,
    pub phase: Phase,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    QueryWaitRelease,
    QueryWaitReacquire,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    #[serde(deserialize_with = "nullable")]
    assigned: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    pending: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    granted_unacked: Option<u64>,
}
impl State {
    fn values(&self) -> Option<[u64; 3]> {
        Some([self.assigned?, self.pending?, self.granted_unacked?])
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Allocator {
    baseline: u64,
    #[serde(deserialize_with = "nullable")]
    high_water: Option<u64>,
    exhausted: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Allocators {
    request: Allocator,
    release: Allocator,
    grant: Allocator,
    ack: Allocator,
    acquire_return: Allocator,
    helper_issuance: Allocator,
    callback: Allocator,
    raw_release: Allocator,
    transaction: Allocator,
    event: Allocator,
}
impl Allocators {
    fn all(&self) -> [&Allocator; 10] {
        [
            &self.request,
            &self.release,
            &self.grant,
            &self.ack,
            &self.acquire_return,
            &self.helper_issuance,
            &self.callback,
            &self.raw_release,
            &self.transaction,
            &self.event,
        ]
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterSet {
    requests: u64,
    pending_registrations: u64,
    idle_grants: u64,
    transfer_grants: u64,
    helper_grants: u64,
    acks: u64,
    acquire_returns: u64,
    release_calls: u64,
    release_transfers: u64,
    release_retains: u64,
    release_raw_obligations: u64,
    helper_issuances: u64,
    callbacks_registered: u64,
    callbacks_accepted: u64,
    callbacks_surplus: u64,
    callbacks_error: u64,
    callbacks_completed: u64,
    raw_release_begins: u64,
    raw_release_ok: u64,
    raw_release_error: u64,
    raw_release_unknown: u64,
    unqualified_requests: u64,
    unqualified_releases: u64,
    unidentified_events: u64,
    transactions_attempted: u64,
    events_attempted: u64,
}
impl CounterSet {
    fn values(&self) -> [u64; 26] {
        [
            self.requests,
            self.pending_registrations,
            self.idle_grants,
            self.transfer_grants,
            self.helper_grants,
            self.acks,
            self.acquire_returns,
            self.release_calls,
            self.release_transfers,
            self.release_retains,
            self.release_raw_obligations,
            self.helper_issuances,
            self.callbacks_registered,
            self.callbacks_accepted,
            self.callbacks_surplus,
            self.callbacks_error,
            self.callbacks_completed,
            self.raw_release_begins,
            self.raw_release_ok,
            self.raw_release_error,
            self.raw_release_unknown,
            self.unqualified_requests,
            self.unqualified_releases,
            self.unidentified_events,
            self.transactions_attempted,
            self.events_attempted,
        ]
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inflight {
    #[serde(deserialize_with = "nullable")]
    callbacks: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    raw_release_attempts: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    acquire_cores_not_returned: Option<u64>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    label: String,
    state: State,
    allocators: Allocators,
    counters: CounterSet,
    registered_inflight: Inflight,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transaction {
    transaction_id: u64,
    before: State,
    after: State,
    first_seq: u64,
    next_seq: u64,
    events: Vec<Event>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    schema: u32,
    producer: String,
    scope: String,
    pub capture_id: String,
    pub controller_id: String,
    capture_state: String,
    #[serde(deserialize_with = "nullable")]
    pub attachment_capture_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub expected_attachments: Option<u64>,
    full_pipeline_complete: bool,
    resource_full_complete: bool,
    producer_closed: bool,
    raw_io_coverage: bool,
    #[serde(deserialize_with = "nullable")]
    cpu_work_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    execution_span_ns: Option<u64>,
    pre_attach_coverage: bool,
    initial_assignment_origin: String,
    #[serde(deserialize_with = "nullable")]
    physical_callbacks_inflight: Option<u64>,
    helper_issuance_callback_pairing: String,
    cut_reason: CutReason,
    start: Checkpoint,
    end: Checkpoint,
    transactions: Vec<Transaction>,
    records_complete: bool,
    identity_exhausted: bool,
    counter_overflowed: bool,
    lost_transactions: u64,
    lost_events: u64,
    loss_overflowed: bool,
    opaque_boundaries: Vec<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CutReason {
    FrontendScopeReturned,
    FrontendScopeUnwound,
    ControllerCheckpoint,
    Unknown,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    CaptureStart {
        seq: u64,
    },
    CaptureCut {
        seq: u64,
    },
    Request {
        seq: u64,
        request_id: u64,
        #[serde(deserialize_with = "nullable")]
        context: Option<WireContext>,
        #[serde(deserialize_with = "nullable")]
        unavailable_reason: Option<Unavailable>,
    },
    Pending {
        seq: u64,
        request_id: u64,
    },
    Grant {
        seq: u64,
        grant_id: u64,
        request_id: u64,
        source: GrantSource,
    },
    Ack {
        seq: u64,
        ack_id: u64,
        request_id: u64,
        grant_id: u64,
    },
    AcquireReturned {
        seq: u64,
        return_id: u64,
        request_id: u64,
        grant_id: u64,
        ack_id: u64,
    },
    ReleaseDecision {
        seq: u64,
        release_id: u64,
        #[serde(deserialize_with = "nullable")]
        context: Option<WireContext>,
        #[serde(deserialize_with = "nullable")]
        unavailable_reason: Option<Unavailable>,
        disposition: ReleaseDisposition,
    },
    HelperIssued {
        seq: u64,
        issuance_id: u64,
        trigger_request_id: u64,
    },
    CallbackRegistered {
        seq: u64,
        callback_id: u64,
    },
    HelperDecision {
        seq: u64,
        callback_id: u64,
        outcome: HelperOutcome,
    },
    CallbackComplete {
        seq: u64,
        callback_id: u64,
        disposition: CallbackDisposition,
    },
    RawReleaseBegin {
        seq: u64,
        attempt_id: u64,
        parent: RawParent,
        result_authority: ResultAuthority,
    },
    RawReleaseEnd {
        seq: u64,
        attempt_id: u64,
        outcome: RawOutcome,
    },
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Unavailable {
    NotSupplied,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum GrantSource {
    IdleImplicit,
    Release { release_id: u64 },
    Helper { callback_id: u64 },
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReleaseDisposition {
    Transfer { grant_id: u64, request_id: u64 },
    RetainImplicit,
    RawRelease { attempt_id: u64 },
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum HelperOutcome {
    Accepted {
        grant_id: u64,
        request_id: u64,
    },
    Surplus {
        attempt_id: u64,
    },
    Error {
        error_kind: ErrorKind,
        #[serde(deserialize_with = "nullable")]
        raw_os_error: Option<i32>,
    },
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ErrorKind {
    BrokenPipe,
    Interrupted,
    InvalidInput,
    WouldBlock,
    PermissionDenied,
    Other,
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CallbackDisposition {
    Consumed,
    SurplusDropFinished,
    Error,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawParent {
    Release { release_id: u64 },
    HelperSurplus { callback_id: u64 },
}
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ResultAuthority {
    NativeResult,
    OpaqueDrop,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawOutcome {
    Ok,
    Error {
        error_kind: ErrorKind,
        #[serde(deserialize_with = "nullable")]
        raw_os_error: Option<i32>,
    },
    Unknown {
        reason: UnknownReason,
    },
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UnknownReason {
    AcquiredDropDiscardsResult,
}
impl Event {
    fn seq(&self) -> u64 {
        match self {
            Self::CaptureStart { seq }
            | Self::CaptureCut { seq }
            | Self::Request { seq, .. }
            | Self::Pending { seq, .. }
            | Self::Grant { seq, .. }
            | Self::Ack { seq, .. }
            | Self::AcquireReturned { seq, .. }
            | Self::ReleaseDecision { seq, .. }
            | Self::HelperIssued { seq, .. }
            | Self::CallbackRegistered { seq, .. }
            | Self::HelperDecision { seq, .. }
            | Self::CallbackComplete { seq, .. }
            | Self::RawReleaseBegin { seq, .. }
            | Self::RawReleaseEnd { seq, .. } => *seq,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceEdge {
    pub source_kind: String,
    pub source_id: u64,
    pub target_kind: String,
    pub target_id: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct ResourceAssessment {
    pub schema: u32,
    pub producer: String,
    pub capture_id: String,
    pub controller_id: String,
    pub prefix_protocol_valid: bool,
    pub prefix_records_complete: bool,
    pub closed_lifecycle: bool,
    pub producer_closed: bool,
    pub resource_full_complete: bool,
    pub full_pipeline_complete: bool,
    pub raw_io_coverage: bool,
    pub cpu_work_ns: Option<u64>,
    pub execution_span_ns: Option<u64>,
    pub local_resource_edges: Vec<ResourceEdge>,
    pub issues: Vec<String>,
    pub opaque_boundaries: Vec<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct ReturnedContext {
    pub return_id: u64,
    pub context: WireContext,
}
pub(crate) struct ValidatedPrefix {
    pub receipt: Receipt,
    pub assessment: ResourceAssessment,
    pub returns: Vec<ReturnedContext>,
    pub contexts: Vec<WireContext>,
    pub context_coverage_complete: bool,
}
pub fn assess_bytes(bytes: &[u8]) -> Result<ResourceAssessment> {
    Ok(collect_bytes(bytes)?.assessment)
}
pub(crate) fn collect_bytes(bytes: &[u8]) -> Result<ValidatedPrefix> {
    if bytes.len() > MAX_BYTES {
        bail!("resource document exceeds 32 MiB");
    }
    let r: Receipt = serde_json::from_slice(bytes).context("invalid resource receipt")?;
    if r.schema != 1
        || r.producer != "rustc-frontend-allowance"
        || r.scope != "selected-ticketed-grant-decisions"
        || !safe_id(&r.capture_id)
        || !safe_id(&r.controller_id)
    {
        bail!("invalid resource schema or capture authority");
    }
    if r.capture_state != "sealed_prefix"
        || r.full_pipeline_complete
        || r.resource_full_complete
        || r.producer_closed
        || r.raw_io_coverage
        || r.pre_attach_coverage
        || r.cpu_work_ns.is_some()
        || r.execution_span_ns.is_some()
        || r.physical_callbacks_inflight.is_some()
        || r.initial_assignment_origin != "opaque"
        || r.helper_issuance_callback_pairing != "unknown"
    {
        bail!("unsupported resource completeness or lifecycle claim");
    }
    if !matches!(r.expected_attachments, None | Some(0 | 1))
        || r.attachment_capture_id
            .as_ref()
            .is_some_and(|s| !safe_id(s))
        || (r.expected_attachments == Some(0) && r.attachment_capture_id.is_some())
    {
        bail!("invalid resource attachment anchor");
    }
    let facts = r
        .transactions
        .iter()
        .try_fold(0usize, |n, t| n.checked_add(t.events.len()))
        .context("resource fact count overflow")?;
    if facts > MAX_FACTS || r.transactions.len() > MAX_FACTS {
        bail!("resource fact budget exceeded");
    }
    validate(r)
}

const BOUNDARIES: &[&str] = &[
    "pre-attach-history",
    "initial-assignment-owner",
    "unqualified-calls",
    "helper-demand-delivery-pairing",
    "physical-helper-entries",
    "helper-lifecycle-tail",
    "surplus-release-result",
    "external-jobserver-origin",
    "native-token-holders",
    "backend-and-other-pools",
    "post-cutoff-activity",
    "nonquery-scheduling",
    "cpu-and-span",
];
fn context_valid(
    context: &Option<WireContext>,
    reason: &Option<Unavailable>,
    phase: Phase,
) -> bool {
    match (context, reason) {
        (Some(c), None) => {
            c.attachment_id > 0 && c.query_root_namespace > 0 && c.wait_id > 0 && c.phase == phase
        }
        (None, Some(Unavailable::NotSupplied)) => true,
        _ => false,
    }
}
fn validate(r: Receipt) -> Result<ValidatedPrefix> {
    let mut issues = BTreeSet::new();
    let mut partial = false;
    macro_rules! check {
        ($condition:expr,$message:expr) => {
            if !$condition {
                issues.insert($message.to_string());
            }
        };
    }
    check!(
        r.start.label == "start" && r.end.label == "end",
        "invalid checkpoint labels"
    );
    let boundaries: BTreeSet<_> = r.opaque_boundaries.iter().map(String::as_str).collect();
    check!(
        boundaries.len() == r.opaque_boundaries.len()
            && boundaries == BOUNDARIES.iter().copied().collect(),
        "invalid opaque boundaries"
    );
    let _cut_reason = &r.cut_reason;
    let start_alloc = r.start.allocators.all();
    let end_alloc = r.end.allocators.all();
    for (start, end) in start_alloc.iter().zip(&end_alloc) {
        check!(
            start.baseline == 0 && end.baseline == 0,
            "allocator baseline must be zero"
        );
        check!(
            !start.exhausted || start.high_water.is_none(),
            "exhausted start allocator has high water"
        );
        check!(
            !end.exhausted || end.high_water.is_none(),
            "exhausted end allocator has high water"
        );
        match (start.high_water, end.high_water) {
            (Some(a), Some(b)) => check!(a <= b, "allocator regressed"),
            _ => partial = true,
        }
        if start.exhausted || end.exhausted {
            partial = true;
            check!(r.identity_exhausted, "missing identity exhaustion flag");
        }
    }
    let start_counts = r.start.counters.values();
    let end_counts = r.end.counters.values();
    let mut expected = [0u64; 26];
    for i in 0..26 {
        if let Some(delta) = end_counts[i].checked_sub(start_counts[i]) {
            expected[i] = delta;
        } else {
            issues.insert("counter regressed".into());
        }
    }
    if r.identity_exhausted
        || r.counter_overflowed
        || r.loss_overflowed
        || r.lost_events > 0
        || r.lost_transactions > 0
        || expected[23] > 0
    {
        partial = true;
    }
    let mut observed = [0u64; 26];
    observed[24] = r.transactions.len() as u64;
    let mut allocated: [BTreeSet<u64>; 10] = std::array::from_fn(|_| BTreeSet::new());
    let mut requests = BTreeMap::new();
    let mut pending = BTreeMap::new();
    let mut grants = BTreeMap::new();
    let mut acks = BTreeMap::new();
    let mut returns = BTreeMap::new();
    let mut releases = BTreeMap::new();
    let mut callback_reg = BTreeMap::new();
    let mut helper_dec = BTreeMap::new();
    let mut callback_end = BTreeMap::new();
    let mut raw_begin = BTreeMap::new();
    let mut raw_end = BTreeMap::new();
    let mut previous: Option<&State> = None;
    let mut previous_txn = None;
    let mut previous_seq = None;
    let mut captures = [0u64; 2];
    for t in &r.transactions {
        check!(
            !t.events.is_empty() && t.events.len() <= 4,
            "invalid transaction fact count"
        );
        check!(
            allocated[8].insert(t.transaction_id),
            "duplicate transaction ID"
        );
        if let Some(id) = previous_txn {
            check!(
                t.transaction_id > id,
                "transaction inventory order regressed"
            );
        }
        previous_txn = Some(t.transaction_id);
        if let Some(old) = previous {
            if !partial {
                check!(
                    old.values() == t.before.values(),
                    "transaction boundary discontinuity"
                );
            }
        } else if !partial {
            check!(
                r.start.state.values() == t.before.values(),
                "start state mismatch"
            );
        }
        previous = Some(&t.after);
        let mut delta = [0i128; 3];
        for (index, e) in t.events.iter().enumerate() {
            let seq = e.seq();
            observed[25] += 1;
            check!(allocated[9].insert(seq), "duplicate event sequence");
            check!(
                t.first_seq.checked_add(index as u64) == Some(seq),
                "noncontiguous transaction sequence"
            );
            if let Some(old) = previous_seq {
                check!(seq > old, "event inventory order regressed");
            }
            previous_seq = Some(seq);
            macro_rules! mint {
                ($which:expr,$id:expr) => {
                    check!(
                        allocated[$which].insert(*$id),
                        "duplicate operation identity"
                    );
                };
            }
            match e {
                Event::CaptureStart { .. } => captures[0] += 1,
                Event::CaptureCut { .. } => captures[1] += 1,
                Event::Request {
                    request_id,
                    context,
                    unavailable_reason,
                    ..
                } => {
                    observed[0] += 1;
                    mint!(0, request_id);
                    check!(
                        context_valid(context, unavailable_reason, Phase::QueryWaitReacquire),
                        "invalid request context"
                    );
                    if context.is_none() {
                        observed[21] += 1;
                    }
                    requests.insert(*request_id, (t.transaction_id, seq, context));
                }
                Event::Pending { request_id, .. } => {
                    observed[1] += 1;
                    delta[1] += 1;
                    check!(
                        pending
                            .insert(*request_id, (t.transaction_id, seq))
                            .is_none(),
                        "duplicate pending request"
                    );
                }
                Event::Grant {
                    grant_id,
                    request_id,
                    source,
                    ..
                } => {
                    mint!(2, grant_id);
                    delta[2] += 1;
                    match source {
                        GrantSource::IdleImplicit => {
                            observed[2] += 1;
                            delta[0] += 1;
                            if let (Some(before), Some(after)) =
                                (t.before.values(), t.after.values())
                            {
                                check!(
                                    before[0] == 0
                                        && before[1] == 0
                                        && after[0] == 1
                                        && after[1] == 0,
                                    "idle implicit grant violated native idle predicate"
                                );
                            } else {
                                partial = true;
                            }
                            check!(requests.get(request_id).is_some_and(|(rt,rs,_)|*rt==t.transaction_id && *rs<seq),"idle grant lacks same-transaction request");
                            check!(
                                t.events
                                    .iter()
                                    .filter(|e| matches!(
                                        e,
                                        Event::Grant {
                                            source: GrantSource::IdleImplicit,
                                            ..
                                        }
                                    ))
                                    .count()
                                    == 1,
                                "multiple idle grants in atomic transaction"
                            );
                        }
                        GrantSource::Release { .. } => {
                            observed[3] += 1;
                            delta[1] -= 1;
                        }
                        GrantSource::Helper { .. } => {
                            observed[4] += 1;
                            delta[0] += 1;
                            delta[1] -= 1;
                        }
                    }
                    grants.insert(*grant_id, (t.transaction_id, seq, *request_id, source));
                }
                Event::Ack {
                    ack_id,
                    request_id,
                    grant_id,
                    ..
                } => {
                    observed[5] += 1;
                    mint!(3, ack_id);
                    delta[2] -= 1;
                    acks.insert(*ack_id, (t.transaction_id, seq, *request_id, *grant_id));
                }
                Event::AcquireReturned {
                    return_id,
                    request_id,
                    grant_id,
                    ack_id,
                    ..
                } => {
                    observed[6] += 1;
                    mint!(4, return_id);
                    returns.insert(
                        *return_id,
                        (t.transaction_id, seq, *request_id, *grant_id, *ack_id),
                    );
                }
                Event::ReleaseDecision {
                    release_id,
                    context,
                    unavailable_reason,
                    disposition,
                    ..
                } => {
                    observed[7] += 1;
                    mint!(1, release_id);
                    check!(
                        context_valid(context, unavailable_reason, Phase::QueryWaitRelease),
                        "invalid release context"
                    );
                    if context.is_none() {
                        observed[22] += 1;
                    }
                    match disposition {
                        ReleaseDisposition::Transfer { .. } => observed[8] += 1,
                        ReleaseDisposition::RetainImplicit => {
                            observed[9] += 1;
                            delta[0] -= 1;
                        }
                        ReleaseDisposition::RawRelease { attempt_id } => {
                            mint!(7, attempt_id);
                            observed[10] += 1;
                            delta[0] -= 1;
                        }
                    }
                    releases.insert(*release_id, (t.transaction_id, seq, disposition));
                }
                Event::HelperIssued {
                    issuance_id,
                    trigger_request_id,
                    ..
                } => {
                    observed[11] += 1;
                    mint!(5, issuance_id);
                    check!(
                        requests
                            .get(trigger_request_id)
                            .is_some_and(|(rt, rs, _)| *rt == t.transaction_id && *rs < seq)
                            && pending
                                .get(trigger_request_id)
                                .is_some_and(|(pt, ps)| *pt == t.transaction_id && *ps < seq),
                        "helper issuance has missing request"
                    );
                }
                Event::CallbackRegistered { callback_id, .. } => {
                    observed[12] += 1;
                    mint!(6, callback_id);
                    callback_reg.insert(*callback_id, (t.transaction_id, seq));
                }
                Event::HelperDecision {
                    callback_id,
                    outcome,
                    ..
                } => {
                    match outcome {
                        HelperOutcome::Accepted { .. } => observed[13] += 1,
                        HelperOutcome::Surplus { attempt_id } => {
                            mint!(7, attempt_id);
                            observed[14] += 1;
                        }
                        HelperOutcome::Error {
                            error_kind,
                            raw_os_error,
                        } => {
                            let _ = (error_kind, raw_os_error);
                            observed[15] += 1;
                        }
                    }
                    check!(
                        helper_dec
                            .insert(*callback_id, (t.transaction_id, seq, outcome))
                            .is_none(),
                        "duplicate helper decision"
                    );
                }
                Event::CallbackComplete {
                    callback_id,
                    disposition,
                    ..
                } => {
                    observed[16] += 1;
                    check!(
                        callback_end
                            .insert(*callback_id, (t.transaction_id, seq, disposition))
                            .is_none(),
                        "duplicate callback completion"
                    );
                }
                Event::RawReleaseBegin {
                    attempt_id,
                    parent,
                    result_authority,
                    ..
                } => {
                    observed[17] += 1;
                    check!(
                        raw_begin
                            .insert(
                                *attempt_id,
                                (t.transaction_id, seq, parent, result_authority)
                            )
                            .is_none(),
                        "duplicate raw begin"
                    );
                }
                Event::RawReleaseEnd {
                    attempt_id,
                    outcome,
                    ..
                } => {
                    match outcome {
                        RawOutcome::Ok => observed[18] += 1,
                        RawOutcome::Error {
                            error_kind,
                            raw_os_error,
                        } => {
                            let _ = (error_kind, raw_os_error);
                            observed[19] += 1;
                        }
                        RawOutcome::Unknown { reason } => {
                            let _ = reason;
                            observed[20] += 1;
                        }
                    }
                    check!(
                        raw_end
                            .insert(*attempt_id, (t.transaction_id, seq, outcome))
                            .is_none(),
                        "duplicate raw result"
                    );
                }
            }
        }
        check!(
            t.first_seq.checked_add(t.events.len() as u64) == Some(t.next_seq),
            "invalid transaction sequence bound"
        );
        match (t.before.values(), t.after.values()) {
            (Some(before), Some(after)) => {
                for i in 0..3 {
                    check!(
                        i128::from(before[i]) + delta[i] == i128::from(after[i]),
                        "transaction conservation mismatch"
                    );
                }
                check!(
                    before[2] <= before[0] && after[2] <= after[0],
                    "unacked grants exceed assigned aggregate"
                );
            }
            _ => partial = true,
        }
    }
    if !partial {
        check!(captures == [1, 1], "missing capture start/cut");
        check!(
            r.transactions
                .first()
                .and_then(|t| t.events.first())
                .is_some_and(|e| matches!(e, Event::CaptureStart { .. }))
                && r.transactions
                    .last()
                    .and_then(|t| t.events.last())
                    .is_some_and(|e| matches!(e, Event::CaptureCut { .. })),
            "capture delimiters misplaced"
        );
        check!(
            previous.map(State::values) == Some(r.end.state.values()),
            "end state mismatch"
        );
    }
    for i in 0..26 {
        if i == 23 {
            continue;
        }
        if partial {
            check!(
                observed[i] <= expected[i],
                "retained inventory exceeds independent counters"
            );
        } else {
            check!(
                observed[i] == expected[i],
                "independent operation inventory mismatch"
            );
        }
    }
    let allocator_counts = [
        expected[0],
        expected[7],
        expected[2]
            .checked_add(expected[3])
            .and_then(|n| n.checked_add(expected[4]))
            .unwrap_or(u64::MAX),
        expected[5],
        expected[6],
        expected[11],
        expected[12],
        expected[10].saturating_add(expected[14]),
        expected[24],
        expected[25],
    ];
    for i in 0..10 {
        if let (Some(low), Some(high)) = (start_alloc[i].high_water, end_alloc[i].high_water) {
            check!(
                allocated[i].iter().all(|id| *id >= low && *id < high),
                "operation ID outside checkpoint range"
            );
            if !r.counter_overflowed {
                check!(
                    high.checked_sub(low) == Some(allocator_counts[i]),
                    "allocator disagrees with independent operation count"
                );
            }
            if !partial {
                check!(
                    allocated[i].len() as u64 == high.saturating_sub(low),
                    "missing independent allocated identities"
                );
            }
        }
    }
    // Source rows may follow Grant within one atomic transaction, never another transaction.
    let mut edge_list = Vec::new();
    let mut returned_contexts = Vec::new();
    let mut request_grants = BTreeSet::new();
    let mut granted_acks = BTreeSet::new();
    let mut ack_returns = BTreeSet::new();
    for (&request, &(txn, seq)) in &pending {
        check!(
            requests
                .get(&request)
                .is_some_and(|(rt, rs, _)| *rt <= txn && *rs < seq),
            "pending has missing or future request"
        );
    }
    for (&grant, &(txn, seq, request, source)) in &grants {
        check!(
            request_grants.insert(request),
            "request granted more than once"
        );
        check!(
            requests
                .get(&request)
                .is_some_and(|(rt, rs, _)| *rt <= txn && *rs < seq),
            "grant has missing or future request"
        );
        match source {
            GrantSource::IdleImplicit => {
                check!(
                    !pending.contains_key(&request),
                    "idle grant for pending request"
                );
            }
            GrantSource::Release { release_id } => {
                check!(releases.get(release_id).is_some_and(|(rt,_,d)|*rt==txn && matches!(d,ReleaseDisposition::Transfer{grant_id,request_id} if *grant_id==grant && *request_id==request)),"grant release source mismatch");
                check!(
                    pending.get(&request).is_some_and(|(pt, _)| *pt < txn),
                    "transfer missing earlier pending"
                );
                edge_list.push(edge("release_decision", *release_id, "grant", grant));
            }
            GrantSource::Helper { callback_id } => {
                check!(helper_dec.get(callback_id).is_some_and(|(ht,_,d)|*ht==txn && matches!(d,HelperOutcome::Accepted{grant_id,request_id} if *grant_id==grant && *request_id==request)),"grant helper source mismatch");
                check!(
                    pending.get(&request).is_some_and(|(pt, _)| *pt < txn),
                    "helper grant missing earlier pending"
                );
                edge_list.push(edge("helper_decision", *callback_id, "grant", grant));
            }
        }
        edge_list.push(edge("request", request, "grant", grant));
    }
    for (&release, &(txn, _, disposition)) in &releases {
        match disposition {
            ReleaseDisposition::Transfer {
                grant_id,
                request_id,
            } => check!(
                grants.get(grant_id).is_some_and(|(gt, _, rq, s)| *gt == txn
                    && rq == request_id
                    && matches!(s,GrantSource::Release{release_id} if *release_id==release)),
                "release transfer missing exact grant"
            ),
            ReleaseDisposition::RawRelease { attempt_id } => {
                if let Some((_, _, parent, authority)) = raw_begin.get(attempt_id) {
                    check!(
                        matches!(parent,RawParent::Release{release_id} if *release_id==release)
                            && **authority == ResultAuthority::NativeResult,
                        "release raw obligation mismatch"
                    );
                }
            }
            ReleaseDisposition::RetainImplicit => {}
        }
    }
    for (&ack, &(txn, seq, request, grant)) in &acks {
        check!(granted_acks.insert(grant), "grant acknowledged twice");
        check!(
            grants
                .get(&grant)
                .is_some_and(|(gt, gs, rq, _)| *gt <= txn && *gs < seq && *rq == request),
            "Ack missing exact earlier grant"
        );
        edge_list.push(edge("grant", grant, "ack", ack));
    }
    for (&returned, &(txn, seq, request, grant, ack)) in &returns {
        check!(ack_returns.insert(ack), "Ack returned twice");
        check!(
            acks.get(&ack).is_some_and(|(at, as_, rq, g)| *at <= txn
                && *as_ < seq
                && *rq == request
                && *g == grant),
            "returned missing exact earlier Ack"
        );
        edge_list.push(edge("ack", ack, "acquire_returned", returned));
        if let Some((_, _, Some(context))) = requests.get(&request) {
            returned_contexts.push(ReturnedContext {
                return_id: returned,
                context: (*context).clone(),
            });
        }
    }
    for (&callback, &(txn, seq, outcome)) in &helper_dec {
        check!(
            callback_reg
                .get(&callback)
                .is_some_and(|(rt, rs)| *rt <= txn && *rs < seq),
            "helper decision missing earlier registration"
        );
        match outcome {
            HelperOutcome::Accepted {
                grant_id,
                request_id,
            } => check!(
                grants.get(grant_id).is_some_and(|(gt, _, rq, s)| *gt == txn
                    && rq == request_id
                    && matches!(s,GrantSource::Helper{callback_id} if *callback_id==callback)),
                "helper decision missing exact grant"
            ),
            HelperOutcome::Surplus { attempt_id } => {
                if let Some((_, _, p, a)) = raw_begin.get(attempt_id) {
                    check!(
                        matches!(p,RawParent::HelperSurplus{callback_id} if *callback_id==callback)
                            && **a == ResultAuthority::OpaqueDrop,
                        "surplus raw obligation mismatch"
                    );
                }
            }
            HelperOutcome::Error { .. } => {}
        }
    }
    for (&callback, &(txn, seq, disposition)) in &callback_end {
        check!(
            helper_dec
                .get(&callback)
                .is_some_and(|(ht, hs, outcome)| *ht <= txn
                    && *hs < seq
                    && matches!(
                        (outcome, disposition),
                        (
                            HelperOutcome::Accepted { .. },
                            CallbackDisposition::Consumed
                        ) | (
                            HelperOutcome::Surplus { .. },
                            CallbackDisposition::SurplusDropFinished
                        ) | (HelperOutcome::Error { .. }, CallbackDisposition::Error)
                    )),
            "callback completion mismatched decision"
        );
        if let Some((_, _, HelperOutcome::Surplus { attempt_id })) = helper_dec.get(&callback) {
            check!(
                raw_end
                    .get(attempt_id)
                    .is_some_and(|(rt, rs, outcome)| *rt <= txn
                        && *rs < seq
                        && matches!(outcome, RawOutcome::Unknown { .. })),
                "surplus callback completed before actual opaque drop endpoint"
            );
        }
    }
    for (&attempt, &(txn, seq, parent, authority)) in &raw_begin {
        match parent {RawParent::Release{release_id}=>check!(releases.get(release_id).is_some_and(|(rt,rs,d)|*rt<=txn && *rs<seq && matches!(d,ReleaseDisposition::RawRelease{attempt_id} if *attempt_id==attempt)) && authority==&ResultAuthority::NativeResult,"raw begin missing native release parent"),RawParent::HelperSurplus{callback_id}=>check!(helper_dec.get(callback_id).is_some_and(|(ht,hs,d)|*ht<=txn && *hs<seq && matches!(d,HelperOutcome::Surplus{attempt_id} if *attempt_id==attempt)) && authority==&ResultAuthority::OpaqueDrop,"raw begin missing surplus parent")}
    }
    for (&attempt, &(txn, seq, outcome)) in &raw_end {
        check!(
            raw_begin
                .get(&attempt)
                .is_some_and(|(bt, bs, _, authority)| *bt <= txn
                    && *bs < seq
                    && match authority {
                        ResultAuthority::NativeResult =>
                            matches!(outcome, RawOutcome::Ok | RawOutcome::Error { .. }),
                        ResultAuthority::OpaqueDrop =>
                            matches!(outcome, RawOutcome::Unknown { .. }),
                    }),
            "raw end missing begin or result authority mismatch"
        );
    }
    for cp in [&r.start, &r.end] {
        let c = &cp.counters;
        let derived = [
            c.callbacks_registered.checked_sub(c.callbacks_completed),
            c.raw_release_begins
                .checked_sub(c.raw_release_ok)
                .and_then(|n| n.checked_sub(c.raw_release_error))
                .and_then(|n| n.checked_sub(c.raw_release_unknown)),
            c.requests.checked_sub(c.acquire_returns),
        ];
        for (actual, computed) in [
            cp.registered_inflight.callbacks,
            cp.registered_inflight.raw_release_attempts,
            cp.registered_inflight.acquire_cores_not_returned,
        ]
        .into_iter()
        .zip(derived)
        {
            if let Some(actual) = actual {
                check!(computed == Some(actual), "inflight checkpoint mismatch");
            } else {
                partial = true;
            }
        }
    }
    let contexts: Vec<_> = requests
        .values()
        .filter_map(|(_, _, c)| c.as_ref().cloned())
        .chain(
            r.transactions
                .iter()
                .flat_map(|t| t.events.iter())
                .filter_map(|e| match e {
                    Event::ReleaseDecision { context, .. } => context.clone(),
                    _ => None,
                }),
        )
        .collect();
    let mut contextual_operations = BTreeSet::new();
    for c in &contexts {
        check!(
            contextual_operations.insert((
                c.attachment_id,
                c.query_root_namespace,
                c.wait_id,
                c.phase == Phase::QueryWaitReacquire
            )),
            "duplicate qualified wait operation/phase"
        );
    }
    let protocol_valid = issues.is_empty();
    let complete = protocol_valid && !partial;
    if r.records_complete && !complete {
        issues.insert("producer records_complete contradicted by inventories".into());
    }
    if !r.records_complete {
        partial = true;
    }
    let protocol_valid = protocol_valid && !(r.records_complete && !complete);
    let closed_requests: BTreeSet<_> = returns
        .values()
        .map(|(_, _, request, _, _)| *request)
        .collect();
    let closed_grants: BTreeSet<_> = returns.values().map(|(_, _, _, grant, _)| *grant).collect();
    let closed_acks: BTreeSet<_> = returns.values().map(|(_, _, _, _, ack)| *ack).collect();
    edge_list.retain(|e| match (e.source_kind.as_str(), e.target_kind.as_str()) {
        ("request", "grant") => {
            closed_requests.contains(&e.source_id) && closed_grants.contains(&e.target_id)
        }
        ("release_decision" | "helper_decision", "grant") => closed_grants.contains(&e.target_id),
        ("grant", "ack") => {
            closed_grants.contains(&e.source_id) && closed_acks.contains(&e.target_id)
        }
        ("ack", "acquire_returned") => closed_acks.contains(&e.source_id),
        _ => false,
    });
    if !protocol_valid || partial {
        edge_list.clear();
        returned_contexts.clear();
    }
    let assessment = ResourceAssessment {
        schema: 1,
        producer: r.producer.clone(),
        capture_id: r.capture_id.clone(),
        controller_id: r.controller_id.clone(),
        prefix_protocol_valid: protocol_valid,
        prefix_records_complete: protocol_valid && !partial,
        closed_lifecycle: false,
        producer_closed: false,
        resource_full_complete: false,
        full_pipeline_complete: false,
        raw_io_coverage: false,
        cpu_work_ns: None,
        execution_span_ns: None,
        local_resource_edges: edge_list,
        issues: issues.into_iter().collect(),
        opaque_boundaries: r.opaque_boundaries.clone(),
    };
    let returned_waits: BTreeSet<_> = returned_contexts
        .iter()
        .map(|r| {
            (
                r.context.attachment_id,
                r.context.query_root_namespace,
                r.context.wait_id,
            )
        })
        .collect();
    let context_coverage_complete = !contexts.is_empty()
        && requests.len() == returns.len()
        && expected[21] == 0
        && expected[22] == 0
        && contexts.iter().all(|c| {
            returned_waits.contains(&(c.attachment_id, c.query_root_namespace, c.wait_id))
        });
    Ok(ValidatedPrefix {
        receipt: r,
        assessment,
        returns: returned_contexts,
        contexts,
        context_coverage_complete,
    })
}
fn edge(source: &str, source_id: u64, target: &str, target_id: u64) -> ResourceEdge {
    ResourceEdge {
        source_kind: source.into(),
        source_id,
        target_kind: target.into(),
        target_id,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{Value, json};
    pub(crate) fn fixture() -> Value {
        let state = json!({"assigned":0,"pending":0,"granted_unacked":0});
        let mut counters = serde_json::Map::new();
        for name in [
            "requests",
            "pending_registrations",
            "idle_grants",
            "transfer_grants",
            "helper_grants",
            "acks",
            "acquire_returns",
            "release_calls",
            "release_transfers",
            "release_retains",
            "release_raw_obligations",
            "helper_issuances",
            "callbacks_registered",
            "callbacks_accepted",
            "callbacks_surplus",
            "callbacks_error",
            "callbacks_completed",
            "raw_release_begins",
            "raw_release_ok",
            "raw_release_error",
            "raw_release_unknown",
            "unqualified_requests",
            "unqualified_releases",
            "unidentified_events",
            "transactions_attempted",
            "events_attempted",
        ] {
            counters.insert(name.into(), json!(0));
        }
        let mut allocators = serde_json::Map::new();
        for name in [
            "request",
            "release",
            "grant",
            "ack",
            "acquire_return",
            "helper_issuance",
            "callback",
            "raw_release",
            "transaction",
            "event",
        ] {
            allocators.insert(
                name.into(),
                json!({"baseline":0,"high_water":0,"exhausted":false}),
            );
        }
        let start = json!({"label":"start","state":state,"allocators":allocators,"counters":counters,"registered_inflight":{"callbacks":0,"raw_release_attempts":0,"acquire_cores_not_returned":0}});
        let mut end = start.clone();
        end["label"] = json!("end");
        end["state"]["assigned"] = json!(1);
        for name in ["requests", "idle_grants", "acks", "acquire_returns"] {
            end["counters"][name] = json!(1);
        }
        end["counters"]["transactions_attempted"] = json!(4);
        end["counters"]["events_attempted"] = json!(6);
        for name in ["request", "grant", "ack", "acquire_return"] {
            end["allocators"][name]["high_water"] = json!(1);
        }
        end["allocators"]["transaction"]["high_water"] = json!(4);
        end["allocators"]["event"]["high_water"] = json!(6);
        let after = end["state"].clone();
        let mut receipt:Value=serde_json::from_str(r#"{"schema":1,"producer":"rustc-frontend-allowance","scope":"selected-ticketed-grant-decisions","capture_id":"resource-1","controller_id":"controller-1","capture_state":"sealed_prefix","attachment_capture_id":"attachment-1","expected_attachments":1,"full_pipeline_complete":false,"resource_full_complete":false,"producer_closed":false,"raw_io_coverage":false,"cpu_work_ns":null,"execution_span_ns":null,"pre_attach_coverage":false,"initial_assignment_origin":"opaque","physical_callbacks_inflight":null,"helper_issuance_callback_pairing":"unknown","cut_reason":"frontend_scope_returned","records_complete":true,"identity_exhausted":false,"counter_overflowed":false,"lost_transactions":0,"lost_events":0,"loss_overflowed":false}"#).unwrap();
        receipt["start"] = start;
        receipt["end"] = end;
        receipt["opaque_boundaries"] = json!(BOUNDARIES);
        receipt["transactions"] = json!([
         {"transaction_id":0,"before":state,"after":state,"first_seq":0,"next_seq":1,"events":[{"seq":0,"kind":"capture_start"}]},
         {"transaction_id":1,"before":state,"after":after,"first_seq":1,"next_seq":4,"events":[{"seq":1,"kind":"request","request_id":0,"context":{"attachment_id":1,"query_root_namespace":1,"wait_id":1,"phase":"query_wait_reacquire"},"unavailable_reason":null},{"seq":2,"kind":"grant","grant_id":0,"request_id":0,"source":{"kind":"idle_implicit"}},{"seq":3,"kind":"ack","ack_id":0,"request_id":0,"grant_id":0}]},
         {"transaction_id":2,"before":after,"after":after,"first_seq":4,"next_seq":5,"events":[{"seq":4,"kind":"acquire_returned","return_id":0,"request_id":0,"grant_id":0,"ack_id":0}]},
         {"transaction_id":3,"before":after,"after":after,"first_seq":5,"next_seq":6,"events":[{"seq":5,"kind":"capture_cut"}]}
        ]);
        receipt
    }
    pub(crate) fn selected_fixture(helper: bool) -> Value {
        let mut v = fixture();
        let initial = json!({"assigned":1,"pending":0,"granted_unacked":0});
        let queued = json!({"assigned":1,"pending":1,"granted_unacked":0});
        let granted = json!({"assigned":if helper {2}else{1},"pending":0,"granted_unacked":1});
        let done = json!({"assigned":if helper {2}else{1},"pending":0,"granted_unacked":0});
        v["start"]["state"] = initial.clone();
        let request = v["transactions"][1]["events"][0].clone();
        let mut queue = vec![request, json!({"kind":"pending","request_id":0})];
        if helper {
            queue.push(json!({"kind":"helper_issued","issuance_id":0,"trigger_request_id":0}));
        }
        let mut groups = vec![
            (
                initial.clone(),
                initial.clone(),
                vec![json!({"kind":"capture_start"})],
            ),
            (initial, queued.clone(), queue),
        ];
        if helper {
            groups.push((
                queued.clone(),
                queued.clone(),
                vec![json!({"kind":"callback_registered","callback_id":0})],
            ));
        }
        let source = if helper {
            json!({"kind":"helper","callback_id":0})
        } else {
            json!({"kind":"release","release_id":0})
        };
        let decision = if helper {
            json!({"kind":"helper_decision","callback_id":0,"outcome":{"kind":"accepted","grant_id":0,"request_id":0}})
        } else {
            json!({"kind":"release_decision","release_id":0,"context":{"attachment_id":1,"query_root_namespace":1,"wait_id":1,"phase":"query_wait_release"},"unavailable_reason":null,"disposition":{"kind":"transfer","grant_id":0,"request_id":0}})
        };
        groups.push((
            queued,
            granted.clone(),
            vec![
                json!({"kind":"grant","grant_id":0,"request_id":0,"source":source}),
                decision,
            ],
        ));
        if helper {
            groups.push((
                granted.clone(),
                granted.clone(),
                vec![json!({"kind":"callback_complete","callback_id":0,"disposition":"consumed"})],
            ));
        }
        groups.push((
            granted,
            done.clone(),
            vec![json!({"kind":"ack","ack_id":0,"request_id":0,"grant_id":0})],
        ));
        groups.push((done.clone(),done.clone(),vec![json!({"kind":"acquire_returned","return_id":0,"request_id":0,"grant_id":0,"ack_id":0})]));
        groups.push((
            done.clone(),
            done.clone(),
            vec![json!({"kind":"capture_cut"})],
        ));
        let mut seq = 0u64;
        let transactions=groups.into_iter().enumerate().map(|(id,(before,after,mut events))|{let first=seq;for e in &mut events {e["seq"]=json!(seq);seq+=1;}json!({"transaction_id":id,"before":before,"after":after,"first_seq":first,"next_seq":seq,"events":events})}).collect::<Vec<_>>();
        let txns = transactions.len();
        v["transactions"] = json!(transactions);
        v["end"]["state"] = done;
        v["end"]["counters"]["idle_grants"] = json!(0);
        v["end"]["counters"]["pending_registrations"] = json!(1);
        if helper {
            for name in [
                "helper_grants",
                "helper_issuances",
                "callbacks_registered",
                "callbacks_accepted",
                "callbacks_completed",
            ] {
                v["end"]["counters"][name] = json!(1);
            }
            for name in ["helper_issuance", "callback"] {
                v["end"]["allocators"][name]["high_water"] = json!(1);
            }
        } else {
            for name in ["transfer_grants", "release_calls", "release_transfers"] {
                v["end"]["counters"][name] = json!(1);
            }
            v["end"]["allocators"]["release"]["high_water"] = json!(1);
        }
        v["end"]["counters"]["transactions_attempted"] = json!(txns);
        v["end"]["counters"]["events_attempted"] = json!(seq);
        v["end"]["allocators"]["transaction"]["high_water"] = json!(txns);
        v["end"]["allocators"]["event"]["high_water"] = json!(seq);
        v
    }
    #[test]
    fn native_source_rows_after_grant_are_valid_atomic_authority() {
        for helper in [false, true] {
            let v = selected_fixture(helper);
            let a = assess(&v);
            assert!(
                a.prefix_protocol_valid && a.prefix_records_complete,
                "{helper}: {:?}",
                a.issues
            );
            assert_eq!(a.local_resource_edges.len(), 4);
        }
    }
    #[test]
    fn retargeted_source_or_helper_trigger_never_authorizes() {
        for helper in [false, true] {
            let mut v = selected_fixture(helper);
            let t = if helper { 3 } else { 2 };
            v["transactions"][t]["events"][0]["source"]
                [if helper { "callback_id" } else { "release_id" }] = json!(1);
            assert!(!assess(&v).prefix_protocol_valid);
        }
        let mut v = selected_fixture(true);
        v["transactions"][1]["events"][2]["trigger_request_id"] = json!(1);
        assert!(!assess(&v).prefix_protocol_valid);
    }
    // Original, unnormalized serializer bytes captured from the actual production-cfg
    // jobserver module plus real Client/helper. This is not native rustc acceptance.
    const ORIGINAL_NATIVE_SAMPLE: &[u8] = br#"{"schema":1,"producer":"rustc-frontend-allowance","scope":"selected-ticketed-grant-decisions","capture_id":"resource-bd32025b70af63e8334d1b46b5145cf7-2","controller_id":"controller-bd32025b70af63e8334d1b46b5145cf7-1","capture_state":"sealed_prefix","attachment_capture_id":null,"expected_attachments":0,"full_pipeline_complete":false,"resource_full_complete":false,"producer_closed":false,"raw_io_coverage":false,"cpu_work_ns":null,"execution_span_ns":null,"pre_attach_coverage":false,"initial_assignment_origin":"opaque","physical_callbacks_inflight":null,"helper_issuance_callback_pairing":"unknown","cut_reason":"controller_checkpoint","start":{"label":"start","state":{"assigned":1,"pending":0,"granted_unacked":0},"allocators":{"request":{"baseline":0,"high_water":0,"exhausted":false},"release":{"baseline":0,"high_water":0,"exhausted":false},"grant":{"baseline":0,"high_water":0,"exhausted":false},"ack":{"baseline":0,"high_water":0,"exhausted":false},"acquire_return":{"baseline":0,"high_water":0,"exhausted":false},"helper_issuance":{"baseline":0,"high_water":0,"exhausted":false},"callback":{"baseline":0,"high_water":0,"exhausted":false},"raw_release":{"baseline":0,"high_water":0,"exhausted":false},"transaction":{"baseline":0,"high_water":0,"exhausted":false},"event":{"baseline":0,"high_water":0,"exhausted":false}},"counters":{"requests":0,"pending_registrations":0,"idle_grants":0,"transfer_grants":0,"helper_grants":0,"acks":0,"acquire_returns":0,"release_calls":0,"release_transfers":0,"release_retains":0,"release_raw_obligations":0,"helper_issuances":0,"callbacks_registered":0,"callbacks_accepted":0,"callbacks_surplus":0,"callbacks_error":0,"callbacks_completed":0,"raw_release_begins":0,"raw_release_ok":0,"raw_release_error":0,"raw_release_unknown":0,"unqualified_requests":0,"unqualified_releases":0,"unidentified_events":0,"transactions_attempted":0,"events_attempted":0},"registered_inflight":{"callbacks":0,"raw_release_attempts":0,"acquire_cores_not_returned":0}},"end":{"label":"end","state":{"assigned":0,"pending":0,"granted_unacked":0},"allocators":{"request":{"baseline":0,"high_water":2,"exhausted":false},"release":{"baseline":0,"high_water":3,"exhausted":false},"grant":{"baseline":0,"high_water":2,"exhausted":false},"ack":{"baseline":0,"high_water":2,"exhausted":false},"acquire_return":{"baseline":0,"high_water":2,"exhausted":false},"helper_issuance":{"baseline":0,"high_water":1,"exhausted":false},"callback":{"baseline":0,"high_water":1,"exhausted":false},"raw_release":{"baseline":0,"high_water":1,"exhausted":false},"transaction":{"baseline":0,"high_water":15,"exhausted":false},"event":{"baseline":0,"high_water":20,"exhausted":false}},"counters":{"requests":2,"pending_registrations":1,"idle_grants":1,"transfer_grants":0,"helper_grants":1,"acks":2,"acquire_returns":2,"release_calls":3,"release_transfers":0,"release_retains":2,"release_raw_obligations":1,"helper_issuances":1,"callbacks_registered":1,"callbacks_accepted":1,"callbacks_surplus":0,"callbacks_error":0,"callbacks_completed":1,"raw_release_begins":1,"raw_release_ok":1,"raw_release_error":0,"raw_release_unknown":0,"unqualified_requests":2,"unqualified_releases":3,"unidentified_events":0,"transactions_attempted":15,"events_attempted":20},"registered_inflight":{"callbacks":0,"raw_release_attempts":0,"acquire_cores_not_returned":0}},"transactions":[{"transaction_id":0,"before":{"assigned":1,"pending":0,"granted_unacked":0},"after":{"assigned":1,"pending":0,"granted_unacked":0},"first_seq":0,"next_seq":1,"events":[{"seq":0,"kind":"capture_start"}]},{"transaction_id":1,"before":{"assigned":1,"pending":0,"granted_unacked":0},"after":{"assigned":1,"pending":1,"granted_unacked":0},"first_seq":1,"next_seq":4,"events":[{"seq":1,"kind":"request","request_id":0,"context":null,"unavailable_reason":"not_supplied"},{"seq":2,"kind":"pending","request_id":0},{"seq":3,"kind":"helper_issued","issuance_id":0,"trigger_request_id":0}]},{"transaction_id":2,"before":{"assigned":1,"pending":1,"granted_unacked":0},"after":{"assigned":1,"pending":1,"granted_unacked":0},"first_seq":4,"next_seq":5,"events":[{"seq":4,"kind":"callback_registered","callback_id":0}]},{"transaction_id":3,"before":{"assigned":1,"pending":1,"granted_unacked":0},"after":{"assigned":2,"pending":0,"granted_unacked":1},"first_seq":5,"next_seq":7,"events":[{"seq":5,"kind":"grant","grant_id":0,"request_id":0,"source":{"kind":"helper","callback_id":0}},{"seq":6,"kind":"helper_decision","callback_id":0,"outcome":{"kind":"accepted","grant_id":0,"request_id":0}}]},{"transaction_id":4,"before":{"assigned":2,"pending":0,"granted_unacked":1},"after":{"assigned":2,"pending":0,"granted_unacked":1},"first_seq":7,"next_seq":8,"events":[{"seq":7,"kind":"callback_complete","callback_id":0,"disposition":"consumed"}]},{"transaction_id":5,"before":{"assigned":2,"pending":0,"granted_unacked":1},"after":{"assigned":2,"pending":0,"granted_unacked":0},"first_seq":8,"next_seq":9,"events":[{"seq":8,"kind":"ack","ack_id":0,"request_id":0,"grant_id":0}]},{"transaction_id":6,"before":{"assigned":2,"pending":0,"granted_unacked":0},"after":{"assigned":2,"pending":0,"granted_unacked":0},"first_seq":9,"next_seq":10,"events":[{"seq":9,"kind":"acquire_returned","return_id":0,"request_id":0,"grant_id":0,"ack_id":0}]},{"transaction_id":7,"before":{"assigned":2,"pending":0,"granted_unacked":0},"after":{"assigned":1,"pending":0,"granted_unacked":0},"first_seq":10,"next_seq":11,"events":[{"seq":10,"kind":"release_decision","release_id":0,"context":null,"unavailable_reason":"not_supplied","disposition":{"kind":"raw_release","attempt_id":0}}]},{"transaction_id":8,"before":{"assigned":1,"pending":0,"granted_unacked":0},"after":{"assigned":1,"pending":0,"granted_unacked":0},"first_seq":11,"next_seq":12,"events":[{"seq":11,"kind":"raw_release_begin","attempt_id":0,"parent":{"kind":"release","release_id":0},"result_authority":"native_result"}]},{"transaction_id":9,"before":{"assigned":1,"pending":0,"granted_unacked":0},"after":{"assigned":1,"pending":0,"granted_unacked":0},"first_seq":12,"next_seq":13,"events":[{"seq":12,"kind":"raw_release_end","attempt_id":0,"outcome":{"kind":"ok"}}]},{"transaction_id":10,"before":{"assigned":1,"pending":0,"granted_unacked":0},"after":{"assigned":0,"pending":0,"granted_unacked":0},"first_seq":13,"next_seq":14,"events":[{"seq":13,"kind":"release_decision","release_id":1,"context":null,"unavailable_reason":"not_supplied","disposition":{"kind":"retain_implicit"}}]},{"transaction_id":11,"before":{"assigned":0,"pending":0,"granted_unacked":0},"after":{"assigned":1,"pending":0,"granted_unacked":0},"first_seq":14,"next_seq":17,"events":[{"seq":14,"kind":"request","request_id":1,"context":null,"unavailable_reason":"not_supplied"},{"seq":15,"kind":"grant","grant_id":1,"request_id":1,"source":{"kind":"idle_implicit"}},{"seq":16,"kind":"ack","ack_id":1,"request_id":1,"grant_id":1}]},{"transaction_id":12,"before":{"assigned":1,"pending":0,"granted_unacked":0},"after":{"assigned":1,"pending":0,"granted_unacked":0},"first_seq":17,"next_seq":18,"events":[{"seq":17,"kind":"acquire_returned","return_id":1,"request_id":1,"grant_id":1,"ack_id":1}]},{"transaction_id":13,"before":{"assigned":1,"pending":0,"granted_unacked":0},"after":{"assigned":0,"pending":0,"granted_unacked":0},"first_seq":18,"next_seq":19,"events":[{"seq":18,"kind":"release_decision","release_id":2,"context":null,"unavailable_reason":"not_supplied","disposition":{"kind":"retain_implicit"}}]},{"transaction_id":14,"before":{"assigned":0,"pending":0,"granted_unacked":0},"after":{"assigned":0,"pending":0,"granted_unacked":0},"first_seq":19,"next_seq":20,"events":[{"seq":19,"kind":"capture_cut"}]}],"records_complete":true,"identity_exhausted":false,"counter_overflowed":false,"lost_transactions":0,"lost_events":0,"loss_overflowed":false,"opaque_boundaries":["pre-attach-history","initial-assignment-owner","unqualified-calls","helper-demand-delivery-pairing","physical-helper-entries","helper-lifecycle-tail","surplus-release-result","external-jobserver-origin","native-token-holders","backend-and-other-pools","post-cutoff-activity","nonquery-scheduling","cpu-and-span"]}"#;
    #[test]
    fn actual_native_serializer_original_bytes_are_strictly_accepted() {
        let p = collect_bytes(ORIGINAL_NATIVE_SAMPLE).unwrap();
        assert!(
            p.assessment.prefix_protocol_valid && p.assessment.prefix_records_complete,
            "{:?}",
            p.assessment.issues
        );
        assert_eq!(p.assessment.local_resource_edges.len(), 7);
        assert!(p.returns.is_empty());
    }
    #[test]
    fn duplicate_qualified_phase_cannot_alias_one_wait_endpoint() {
        for phase in ["query_wait_reacquire", "query_wait_release"] {
            let mut v: Value = serde_json::from_slice(ORIGINAL_NATIVE_SAMPLE).unwrap();
            let kind = if phase == "query_wait_reacquire" {
                "request"
            } else {
                "release_decision"
            };
            let mut replaced = 0;
            for t in v["transactions"].as_array_mut().unwrap() {
                for e in t["events"].as_array_mut().unwrap() {
                    if e["kind"] == kind && replaced < 2 {
                        e["context"] = json!({"attachment_id":1,"query_root_namespace":1,"wait_id":1,"phase":phase});
                        e["unavailable_reason"] = Value::Null;
                        replaced += 1;
                    }
                }
            }
            v["end"]["counters"][if phase == "query_wait_reacquire" {
                "unqualified_requests"
            } else {
                "unqualified_releases"
            }] = json!(if phase == "query_wait_reacquire" {
                0
            } else {
                1
            });
            let a = assess(&v);
            assert!(
                !a.prefix_protocol_valid && a.local_resource_edges.is_empty(),
                "{phase}"
            );
        }
    }
    fn assess(v: &Value) -> ResourceAssessment {
        assess_bytes(&serde_json::to_vec(v).unwrap()).unwrap()
    }
    #[test]
    fn zero_ids_exact_independent_prefix() {
        let a = assess(&fixture());
        assert!(
            a.prefix_protocol_valid && a.prefix_records_complete,
            "{:?}",
            a.issues
        );
        assert_eq!(a.local_resource_edges.len(), 3);
        assert!(!a.closed_lifecycle && !a.resource_full_complete);
    }
    #[test]
    fn impossible_idle_source_cannot_authorize_a_closed_chain() {
        let mut v = fixture();
        v["start"]["state"]["assigned"] = json!(1);
        v["end"]["state"]["assigned"] = json!(2);
        for t in v["transactions"].as_array_mut().unwrap() {
            for point in ["before", "after"] {
                let assigned = t[point]["assigned"].as_u64().unwrap();
                t[point]["assigned"] = json!(assigned + 1);
            }
        }
        let a = assess(&v);
        assert!(!a.prefix_protocol_valid && a.local_resource_edges.is_empty());
    }
    #[test]
    fn each_missing_inventory_category_is_detected() {
        for i in 0..4 {
            let mut v = fixture();
            v["transactions"].as_array_mut().unwrap().remove(i);
            assert!(!assess(&v).prefix_records_complete);
        }
    }
    #[test]
    fn required_nullable_unknown_duplicate_and_invalid_ids() {
        for name in [
            "cpu_work_ns",
            "physical_callbacks_inflight",
            "attachment_capture_id",
        ] {
            let mut v = fixture();
            v.as_object_mut().unwrap().remove(name);
            assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        let raw = serde_json::to_string(&fixture()).unwrap();
        assert!(
            assess_bytes(
                raw.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1)
                    .as_bytes()
            )
            .is_err()
        );
        let mut v = fixture();
        v["unknown"] = json!(1);
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = fixture();
        v["transactions"][1]["events"][0]["request_id"] = Value::Null;
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn changed_ack_and_state_never_create_edges() {
        for field in ["grant_id", "request_id"] {
            let mut v = fixture();
            v["transactions"][1]["events"][2][field] = json!(1);
            let a = assess(&v);
            assert!(!a.prefix_protocol_valid && a.local_resource_edges.is_empty());
        }
        let mut v = fixture();
        v["transactions"][1]["after"]["assigned"] = json!(3);
        assert!(!assess(&v).prefix_protocol_valid);
    }
    #[test]
    fn loss_overflow_and_huge_range_do_not_promote() {
        for field in [
            "identity_exhausted",
            "counter_overflowed",
            "loss_overflowed",
        ] {
            let mut v = fixture();
            v[field] = json!(true);
            v["records_complete"] = json!(false);
            assert!(!assess(&v).prefix_records_complete);
        }
        let mut v = fixture();
        v["end"]["allocators"]["request"]["high_water"] = json!(u64::MAX);
        assert!(!assess(&v).prefix_records_complete);
    }
    #[test]
    fn unfinished_tail_is_valid_not_closed() {
        let mut v = fixture();
        v["transactions"].as_array_mut().unwrap().remove(2);
        v["transactions"][2]["transaction_id"] = json!(2);
        v["transactions"][2]["first_seq"] = json!(4);
        v["transactions"][2]["next_seq"] = json!(5);
        v["transactions"][2]["events"][0]["seq"] = json!(4);
        v["end"]["counters"]["acquire_returns"] = json!(0);
        v["end"]["counters"]["transactions_attempted"] = json!(3);
        v["end"]["counters"]["events_attempted"] = json!(5);
        v["end"]["allocators"]["acquire_return"]["high_water"] = json!(0);
        v["end"]["allocators"]["transaction"]["high_water"] = json!(3);
        v["end"]["allocators"]["event"]["high_water"] = json!(5);
        v["end"]["registered_inflight"]["acquire_cores_not_returned"] = json!(1);
        let a = assess(&v);
        assert!(
            a.prefix_protocol_valid && a.prefix_records_complete,
            "{:?}",
            a.issues
        );
        assert!(!a.closed_lifecycle);
    }
}
