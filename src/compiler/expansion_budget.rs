//! Shared, poisoned expansion accounting. This is infrastructure, not macro support.
//!
//! Logical charges never roll back. Only live storage and matcher-state leases
//! release on drop. Every operation involving an invocation locks the session
//! before the invocation. Depth arguments are absolute branch depths.
use super::resource_accounting::ReservationAccounting;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Counter {
    InputTokens,
    Invocations,
    Work,
    CaptureElements,
    OutputTokens,
    ExpansionRecords,
    SpanRecords,
    HygieneContexts,
    ScopeTransitions,
    DiagnosticBytes,
}
impl Counter {
    pub const ALL: [Self; 10] = [
        Self::InputTokens,
        Self::Invocations,
        Self::Work,
        Self::CaptureElements,
        Self::OutputTokens,
        Self::ExpansionRecords,
        Self::SpanRecords,
        Self::HygieneContexts,
        Self::ScopeTransitions,
        Self::DiagnosticBytes,
    ];
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Depth {
    OriginalGroups,
    GeneratedGroups,
    Expansion,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    Retained,
    Temporary,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpansionLimits {
    pub counters: [u64; 10],
    pub depths: [u32; 3],
    pub bytes: u64,
    pub matcher_states: u64,
    pub invocation_output: u64,
    /// A rendering truncation policy, not a poisoned depth admission.
    pub backtrace_frames: u32,
}
impl Default for ExpansionLimits {
    fn default() -> Self {
        Self {
            counters: [
                4_000_000, 65_536, 16_000_000, 1_000_000, 4_000_000, 65_536, 8_000_000, 1_000_000,
                1_000_000, 1_048_576,
            ],
            depths: [512, 256, 128],
            bytes: 256 * 1024 * 1024,
            matcher_states: 65_536,
            invocation_output: 1_000_000,
            backtrace_frames: 64,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetFailureKind {
    Limit,
    Overflow,
    Allocation,
    Infrastructure,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetResource {
    Counter(Counter),
    Depth(Depth),
    Bytes,
    MatcherStates,
    InvocationOutput,
    Index,
    Allocation,
    Authority,
    Limits,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetFailure {
    pub kind: BudgetFailureKind,
    pub resource: BudgetResource,
    pub used: u64,
    pub requested: u64,
    pub limit: u64,
}
impl fmt::Display for BudgetFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "expansion budget {:?} for {:?}: used {}, requested {}, limit {}",
            self.kind, self.resource, self.used, self.requested, self.limit
        )
    }
}
impl std::error::Error for BudgetFailure {}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetSnapshot {
    pub counters: [u64; 10],
    pub retained_bytes: u64,
    pub temporary_bytes: u64,
    pub peak_bytes: u64,
    /// Logical reservation telemetry, not allocator instrumentation or RSS.
    pub reservations: ReservationAccounting,
    pub failure: Option<BudgetFailure>,
}
#[derive(Debug)]
struct State {
    limits: ExpansionLimits,
    snapshot: BudgetSnapshot,
}
impl State {
    fn latch(&mut self, failure: BudgetFailure) -> BudgetFailure {
        self.snapshot.failure.get_or_insert(failure).clone()
    }
    fn healthy(&self) -> Result<(), BudgetFailure> {
        self.snapshot.failure.clone().map_or(Ok(()), Err)
    }
    fn checked(
        &mut self,
        resource: BudgetResource,
        used: u64,
        requested: u64,
        limit: u64,
    ) -> Result<u64, BudgetFailure> {
        let Some(next) = used.checked_add(requested) else {
            return Err(self.latch(BudgetFailure {
                kind: BudgetFailureKind::Overflow,
                resource,
                used,
                requested,
                limit,
            }));
        };
        if next > limit {
            return Err(self.latch(BudgetFailure {
                kind: BudgetFailureKind::Limit,
                resource,
                used,
                requested,
                limit,
            }));
        }
        Ok(next)
    }
    fn bytes(&mut self, add: u64) -> Result<u64, BudgetFailure> {
        let total = self
            .snapshot
            .retained_bytes
            .checked_add(self.snapshot.temporary_bytes)
            .ok_or_else(|| self.latch(infrastructure()))?;
        self.checked(BudgetResource::Bytes, total, add, self.limits.bytes)
    }
    fn add_bytes(&mut self, storage: Storage, bytes: u64, total: u64) {
        match storage {
            Storage::Retained => self.snapshot.retained_bytes += bytes,
            Storage::Temporary => self.snapshot.temporary_bytes += bytes,
        }
        self.snapshot.peak_bytes = self.snapshot.peak_bytes.max(total);
    }
    fn release_bytes(&mut self, storage: Storage, bytes: u64) {
        let slot = match storage {
            Storage::Retained => &mut self.snapshot.retained_bytes,
            Storage::Temporary => &mut self.snapshot.temporary_bytes,
        };
        if let Some(next) = slot.checked_sub(bytes) {
            *slot = next;
        } else {
            self.latch(infrastructure());
        }
    }
}
fn infrastructure() -> BudgetFailure {
    BudgetFailure {
        kind: BudgetFailureKind::Infrastructure,
        resource: BudgetResource::Authority,
        used: 0,
        requested: 0,
        limit: 0,
    }
}
#[derive(Clone, Debug)]
pub struct ExpansionBudget {
    state: Arc<Mutex<State>>,
}
impl Default for ExpansionBudget {
    fn default() -> Self {
        Self::new(ExpansionLimits::default())
    }
}
impl ExpansionBudget {
    /// Identity of the shared admission ledger, not equality of configured limits.
    pub fn same_authority(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
    /// Raised caps are rejected by pre-poisoning, never silently clamped.
    /// Call `check` to observe construction failure before allocating payloads.
    pub fn new(limits: ExpansionLimits) -> Self {
        let max = ExpansionLimits::default();
        let invalid = limits
            .counters
            .iter()
            .zip(max.counters)
            .any(|(a, b)| *a > b)
            || limits.depths.iter().zip(max.depths).any(|(a, b)| *a > b)
            || limits.bytes > max.bytes
            || limits.matcher_states > max.matcher_states
            || limits.invocation_output > max.invocation_output
            || limits.backtrace_frames > max.backtrace_frames;
        let failure = invalid.then_some(BudgetFailure {
            kind: BudgetFailureKind::Limit,
            resource: BudgetResource::Limits,
            used: 0,
            requested: 1,
            limit: 0,
        });
        Self {
            state: Arc::new(Mutex::new(State {
                limits,
                snapshot: BudgetSnapshot {
                    counters: [0; 10],
                    retained_bytes: 0,
                    temporary_bytes: 0,
                    peak_bytes: 0,
                    reservations: ReservationAccounting::default(),
                    failure,
                },
            })),
        }
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(error) => {
                let mut state = error.into_inner();
                state.latch(infrastructure());
                state
            }
        }
    }
    pub fn limits(&self) -> ExpansionLimits {
        self.lock().limits
    }
    pub fn snapshot(&self) -> BudgetSnapshot {
        self.lock().snapshot.clone()
    }
    pub fn check(&self) -> Result<(), BudgetFailure> {
        self.lock().healthy()
    }
    pub fn charge(&self, counter: Counter, n: u64) -> Result<(), BudgetFailure> {
        let mut s = self.lock();
        s.healthy()?;
        let index = counter as usize;
        let used = s.snapshot.counters[index];
        let limit = s.limits.counters[index];
        let next = s.checked(BudgetResource::Counter(counter), used, n, limit)?;
        s.snapshot.counters[index] = next;
        Ok(())
    }
    pub fn check_depth(&self, kind: Depth, absolute_depth: u32) -> Result<(), BudgetFailure> {
        let mut s = self.lock();
        s.healthy()?;
        let limit = u64::from(s.limits.depths[kind as usize]);
        s.checked(
            BudgetResource::Depth(kind),
            0,
            u64::from(absolute_depth),
            limit,
        )?;
        Ok(())
    }
    pub fn reserve(&self, storage: Storage, bytes: u64) -> Result<ByteLease, BudgetFailure> {
        {
            let mut s = self.lock();
            s.healthy()?;
            let total = s.bytes(bytes)?;
            s.add_bytes(storage, bytes, total);
            s.snapshot.reservations.record(bytes as usize, false);
            if storage == Storage::Retained {
                s.snapshot.reservations.record_retention();
            }
        }
        Ok(ByteLease {
            budget: self.clone(),
            storage,
            bytes,
        })
    }
    pub fn reserve_array(
        &self,
        storage: Storage,
        count: u64,
        size: u64,
    ) -> Result<ByteLease, BudgetFailure> {
        let bytes = {
            let mut s = self.lock();
            s.healthy()?;
            let limit = s.limits.bytes;
            count.checked_mul(size).ok_or_else(|| {
                s.latch(BudgetFailure {
                    kind: BudgetFailureKind::Overflow,
                    resource: BudgetResource::Bytes,
                    used: count,
                    requested: size,
                    limit,
                })
            })?
        };
        self.reserve(storage, bytes)
    }
    pub fn checked_u32(&self, index: u64) -> Result<u32, BudgetFailure> {
        let mut s = self.lock();
        s.healthy()?;
        u32::try_from(index).map_err(|_| {
            s.latch(BudgetFailure {
                kind: BudgetFailureKind::Overflow,
                resource: BudgetResource::Index,
                used: 0,
                requested: index,
                limit: u64::from(u32::MAX),
            })
        })
    }
    pub fn poison_allocation(&self) -> BudgetFailure {
        self.lock().latch(BudgetFailure {
            kind: BudgetFailureKind::Allocation,
            resource: BudgetResource::Allocation,
            used: 0,
            requested: 0,
            limit: 0,
        })
    }
    pub fn begin_invocation(&self) -> Result<InvocationBudget, BudgetFailure> {
        self.charge(Counter::Invocations, 1)?;
        Ok(InvocationBudget {
            budget: self.clone(),
            state: Arc::new(Mutex::new(InvocationState {
                output: 0,
                states: 0,
            })),
        })
    }
}
#[derive(Debug)]
pub struct ByteLease {
    budget: ExpansionBudget,
    storage: Storage,
    bytes: u64,
}
impl ByteLease {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn resize(&mut self, new_bytes: u64) -> Result<(), BudgetFailure> {
        let mut s = self.budget.lock();
        s.healthy()?;
        if new_bytes >= self.bytes {
            let add = new_bytes - self.bytes;
            let total = s.bytes(add)?;
            s.add_bytes(self.storage, add, total);
        } else {
            s.release_bytes(self.storage, self.bytes - new_bytes);
        }
        s.snapshot
            .reservations
            .resize_bytes(self.bytes as usize, new_bytes as usize);
        self.bytes = new_bytes;
        Ok(())
    }
    pub fn promote_retained(&mut self) -> Result<(), BudgetFailure> {
        let mut s = self.budget.lock();
        s.healthy()?;
        if self.storage == Storage::Temporary {
            s.release_bytes(Storage::Temporary, self.bytes);
            s.snapshot.retained_bytes += self.bytes;
            s.snapshot.reservations.record_retention();
            self.storage = Storage::Retained;
        }
        Ok(())
    }
}
impl Drop for ByteLease {
    fn drop(&mut self) {
        let mut state = self.budget.lock();
        state.release_bytes(self.storage, self.bytes);
        state.snapshot.reservations.release(self.bytes as usize);
    }
}
#[derive(Debug)]
struct InvocationState {
    output: u64,
    states: u64,
}
#[derive(Clone, Debug)]
pub struct InvocationBudget {
    budget: ExpansionBudget,
    state: Arc<Mutex<InvocationState>>,
}
impl InvocationBudget {
    fn lock<'a>(&'a self, session: &mut State) -> MutexGuard<'a, InvocationState> {
        match self.state.lock() {
            Ok(s) => s,
            Err(e) => {
                session.latch(infrastructure());
                e.into_inner()
            }
        }
    }
    pub fn emit(&self, n: u64) -> Result<(), BudgetFailure> {
        let mut s = self.budget.lock();
        let mut local = self.lock(&mut s);
        s.healthy()?;
        let limit = s.limits.invocation_output;
        let output = s.checked(BudgetResource::InvocationOutput, local.output, n, limit)?;
        let i = Counter::OutputTokens as usize;
        let used = s.snapshot.counters[i];
        let limit = s.limits.counters[i];
        let total = s.checked(
            BudgetResource::Counter(Counter::OutputTokens),
            used,
            n,
            limit,
        )?;
        local.output = output;
        s.snapshot.counters[i] = total;
        Ok(())
    }
    pub fn reserve_states(
        &self,
        n: u64,
        bytes_per_state: u64,
    ) -> Result<StateLease, BudgetFailure> {
        let bytes;
        {
            let mut s = self.budget.lock();
            let mut local = self.lock(&mut s);
            s.healthy()?;
            let limit = s.limits.bytes;
            bytes = n.checked_mul(bytes_per_state).ok_or_else(|| {
                s.latch(BudgetFailure {
                    kind: BudgetFailureKind::Overflow,
                    resource: BudgetResource::Bytes,
                    used: n,
                    requested: bytes_per_state,
                    limit,
                })
            })?;
            let limit = s.limits.matcher_states;
            let states = s.checked(BudgetResource::MatcherStates, local.states, n, limit)?;
            let total = s.bytes(bytes)?;
            let i = Counter::Work as usize;
            let used = s.snapshot.counters[i];
            let limit = s.limits.counters[i];
            let work = s.checked(BudgetResource::Counter(Counter::Work), used, n, limit)?;
            local.states = states;
            s.snapshot.counters[i] = work;
            s.add_bytes(Storage::Temporary, bytes, total);
            s.snapshot.reservations.record(bytes as usize, false);
        }
        Ok(StateLease {
            invocation: self.clone(),
            states: n,
            bytes,
        })
    }
}
#[derive(Debug)]
pub struct StateLease {
    invocation: InvocationBudget,
    states: u64,
    bytes: u64,
}
impl Drop for StateLease {
    fn drop(&mut self) {
        let mut s = self.invocation.budget.lock();
        let mut local = self.invocation.lock(&mut s);
        if let Some(next) = local.states.checked_sub(self.states) {
            local.states = next;
        } else {
            s.latch(infrastructure());
        }
        s.release_bytes(Storage::Temporary, self.bytes);
        s.snapshot.reservations.release(self.bytes as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    fn small() -> ExpansionLimits {
        ExpansionLimits {
            counters: [8; 10],
            depths: [3; 3],
            bytes: 16,
            matcher_states: 4,
            invocation_output: 4,
            backtrace_frames: 2,
        }
    }
    #[test]
    fn authority_identity_distinguishes_equal_limits_from_shared_clones() {
        let budget = ExpansionBudget::default();
        assert!(budget.same_authority(&budget.clone()));
        assert!(!budget.same_authority(&ExpansionBudget::default()));
        budget.charge(Counter::Work, 7).unwrap();
        assert_eq!(budget.snapshot().counters[Counter::Work as usize], 7);
    }

    #[test]
    fn published_caps_are_exact() {
        let l = ExpansionLimits::default();
        assert_eq!(
            l.counters,
            [
                4_000_000, 65_536, 16_000_000, 1_000_000, 4_000_000, 65_536, 8_000_000, 1_000_000,
                1_000_000, 1_048_576
            ]
        );
        assert_eq!(l.depths, [512, 256, 128]);
        assert_eq!(
            (
                l.bytes,
                l.matcher_states,
                l.invocation_output,
                l.backtrace_frames
            ),
            (268_435_456, 65_536, 1_000_000, 64)
        );
        assert_eq!(ExpansionBudget::default().check(), Ok(()));
    }
    #[test]
    fn each_raised_field_prepoisons_without_clamping() {
        let max = ExpansionLimits::default();
        let mut cases = Vec::new();
        for i in 0..10 {
            let mut l = max;
            l.counters[i] += 1;
            cases.push(l);
        }
        for i in 0..3 {
            let mut l = max;
            l.depths[i] += 1;
            cases.push(l);
        }
        let mut l = max;
        l.bytes += 1;
        cases.push(l);
        let mut l = max;
        l.matcher_states += 1;
        cases.push(l);
        let mut l = max;
        l.invocation_output += 1;
        cases.push(l);
        let mut l = max;
        l.backtrace_frames += 1;
        cases.push(l);
        for limits in cases {
            let b = ExpansionBudget::new(limits);
            let e = b.check().unwrap_err();
            assert_eq!(
                (e.kind, e.resource),
                (BudgetFailureKind::Limit, BudgetResource::Limits)
            );
            assert_eq!(b.limits(), limits);
            assert_eq!(b.charge(Counter::Work, 0), Err(e.clone()));
            assert_eq!(b.reserve(Storage::Temporary, 0).unwrap_err(), e);
            assert_eq!(b.snapshot().counters, [0; 10]);
        }
    }
    #[test]
    fn all_zero_limits_are_valid_and_reject_positive_admission() {
        let l = ExpansionLimits {
            counters: [0; 10],
            depths: [0; 3],
            bytes: 0,
            matcher_states: 0,
            invocation_output: 0,
            backtrace_frames: 0,
        };
        let b = ExpansionBudget::new(l);
        b.check().unwrap();
        b.charge(Counter::Work, 0).unwrap();
        b.check_depth(Depth::Expansion, 0).unwrap();
        drop(b.reserve(Storage::Retained, 0).unwrap());
        assert_eq!(
            b.charge(Counter::Work, 1).unwrap_err().kind,
            BudgetFailureKind::Limit
        );
    }
    #[test]
    fn counters_are_independent_monotonic_and_first_failure_latches() {
        for c in Counter::ALL {
            let b = ExpansionBudget::new(small());
            b.charge(c, 8).unwrap();
            for other in Counter::ALL {
                assert_eq!(
                    b.snapshot().counters[other as usize],
                    if c == other { 8 } else { 0 }
                );
            }
            let e = b.charge(c, 1).unwrap_err();
            assert_eq!(e.resource, BudgetResource::Counter(c));
            assert_eq!(b.poison_allocation(), e);
            assert_eq!(b.charge(Counter::Work, 0), Err(e));
            assert_eq!(b.snapshot().counters[c as usize], 8);
        }
    }
    #[test]
    fn every_zero_resource_rejects_its_first_positive_request() {
        for counter in Counter::ALL {
            let mut limits = small();
            limits.counters[counter as usize] = 0;
            let budget = ExpansionBudget::new(limits);
            assert_eq!(
                budget.charge(counter, 1).unwrap_err().resource,
                BudgetResource::Counter(counter)
            );
        }
        for depth in [
            Depth::OriginalGroups,
            Depth::GeneratedGroups,
            Depth::Expansion,
        ] {
            let mut limits = small();
            limits.depths[depth as usize] = 0;
            let budget = ExpansionBudget::new(limits);
            assert_eq!(
                budget.check_depth(depth, 1).unwrap_err().resource,
                BudgetResource::Depth(depth)
            );
        }
        let mut limits = small();
        limits.bytes = 0;
        let budget = ExpansionBudget::new(limits);
        assert_eq!(
            budget.reserve(Storage::Retained, 1).unwrap_err().resource,
            BudgetResource::Bytes
        );
        let mut limits = small();
        limits.matcher_states = 0;
        let budget = ExpansionBudget::new(limits);
        let invocation = budget.begin_invocation().unwrap();
        assert_eq!(
            invocation.reserve_states(1, 0).unwrap_err().resource,
            BudgetResource::MatcherStates
        );
        let mut limits = small();
        limits.invocation_output = 0;
        let budget = ExpansionBudget::new(limits);
        let invocation = budget.begin_invocation().unwrap();
        assert_eq!(
            invocation.emit(1).unwrap_err().resource,
            BudgetResource::InvocationOutput
        );
    }
    #[test]
    fn simultaneous_matcher_leases_share_invocation_state_and_bytes() {
        let budget = ExpansionBudget::new(small());
        let invocation = budget.begin_invocation().unwrap();
        let gate = Arc::new(Barrier::new(2));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let invocation = invocation.clone();
                let gate = gate.clone();
                thread::spawn(move || {
                    let lease = invocation.reserve_states(4, 4);
                    gate.wait();
                    lease.is_ok()
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .filter(|success| *success)
                .count(),
            1
        );
        assert_eq!(budget.snapshot().temporary_bytes, 0);
        assert_eq!(budget.snapshot().counters[Counter::Work as usize], 4);
        assert_eq!(invocation.state.lock().unwrap().states, 0);
    }
    #[test]
    fn flat_lease_drop_runs_on_an_ordinary_small_stack() {
        thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                let budget = ExpansionBudget::default();
                let leases: Vec<_> = (0..4096)
                    .map(|_| budget.reserve(Storage::Retained, 1).unwrap())
                    .collect();
                assert_eq!(budget.snapshot().retained_bytes, 4096);
                drop(leases);
                assert_eq!(budget.snapshot().retained_bytes, 0);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn absolute_depth_does_not_accumulate_across_branches() {
        for kind in [
            Depth::OriginalGroups,
            Depth::GeneratedGroups,
            Depth::Expansion,
        ] {
            let b = ExpansionBudget::default();
            let limit = b.limits().depths[kind as usize];
            for _ in 0..10 {
                b.check_depth(kind, limit).unwrap();
            }
            let e = b.check_depth(kind, limit + 1).unwrap_err();
            assert_eq!(e.resource, BudgetResource::Depth(kind));
            assert_eq!(b.snapshot().counters, [0; 10]);
        }
    }
    #[test]
    fn retained_and_temporary_share_one_live_ceiling() {
        let b = ExpansionBudget::new(small());
        let mut a = b.reserve(Storage::Retained, 8).unwrap();
        let mut c = b.reserve_array(Storage::Temporary, 2, 4).unwrap();
        assert_eq!(b.snapshot().peak_bytes, 16);
        a.resize(4).unwrap();
        c.promote_retained().unwrap();
        assert_eq!(c.bytes(), 8);
        assert_eq!(
            (b.snapshot().retained_bytes, b.snapshot().temporary_bytes),
            (12, 0)
        );
        c.resize(12).unwrap();
        assert_eq!(b.snapshot().retained_bytes, 16);
        let e = c.resize(13).unwrap_err();
        assert_eq!(e.resource, BudgetResource::Bytes);
        assert_eq!(c.bytes(), 12);
        drop(a);
        drop(c);
        assert_eq!(
            (b.snapshot().retained_bytes, b.snapshot().temporary_bytes),
            (0, 0)
        );
        assert_eq!(b.snapshot().peak_bytes, 16);
    }
    #[test]
    fn common_reservation_telemetry_tracks_lease_lifecycle() {
        let budget = ExpansionBudget::new(small());
        let mut lease = budget.reserve(Storage::Temporary, 4).unwrap();
        lease.resize(8).unwrap();
        lease.resize(2).unwrap();
        lease.promote_retained().unwrap();
        lease.promote_retained().unwrap();
        let accounting = budget.snapshot().reservations;
        assert_eq!(accounting.reservations, 1);
        assert_eq!(accounting.current_reservations, 1);
        assert_eq!(accounting.retentions, 1);
        assert_eq!(accounting.current_bytes, 2);
        assert_eq!(accounting.peak_bytes, 8);
        assert_eq!(accounting.cumulative_bytes, 8);
        budget.poison_allocation();
        drop(lease);
        let accounting = budget.snapshot().reservations;
        assert_eq!(accounting.current_reservations, 0);
        assert_eq!(accounting.current_bytes, 0);
        assert_eq!(accounting.cumulative_bytes, 8);
        assert!(!accounting.counters_saturated);
    }
    #[test]
    fn live_storage_is_released_after_poison_without_refunding_work() {
        let b = ExpansionBudget::new(small());
        let i = b.begin_invocation().unwrap();
        i.emit(3).unwrap();
        let states = i.reserve_states(2, 4).unwrap();
        let lease = b.reserve(Storage::Retained, 8).unwrap();
        let e = b.poison_allocation();
        drop(states);
        drop(lease);
        drop(i);
        let s = b.snapshot();
        assert_eq!((s.retained_bytes, s.temporary_bytes), (0, 0));
        assert_eq!(s.counters[Counter::Work as usize], 2);
        assert_eq!(s.counters[Counter::OutputTokens as usize], 3);
        assert_eq!(s.counters[Counter::Invocations as usize], 1);
        assert_eq!(b.check(), Err(e));
    }
    #[test]
    fn arithmetic_and_id_failures_are_typed_and_latched() {
        let b = ExpansionBudget::default();
        let e = b
            .reserve_array(Storage::Temporary, u64::MAX, 2)
            .unwrap_err();
        assert_eq!(e.kind, BudgetFailureKind::Overflow);
        assert_eq!(b.checked_u32(u64::MAX), Err(e));
        let b = ExpansionBudget::default();
        assert_eq!(b.checked_u32(u64::from(u32::MAX)).unwrap(), u32::MAX);
        assert_eq!(
            b.checked_u32(u64::from(u32::MAX) + 1).unwrap_err().resource,
            BudgetResource::Index
        );
        let b = ExpansionBudget::default();
        b.charge(Counter::Work, 1).unwrap();
        assert_eq!(
            b.charge(Counter::Work, u64::MAX).unwrap_err().kind,
            BudgetFailureKind::Overflow
        );
        let b = ExpansionBudget::default();
        assert_eq!(b.poison_allocation().kind, BudgetFailureKind::Allocation);
    }
    #[test]
    fn invocation_clones_share_local_output_and_session_output_is_atomic() {
        let mut l = small();
        l.counters[Counter::OutputTokens as usize] = 5;
        let b = ExpansionBudget::new(l);
        let a = b.begin_invocation().unwrap();
        let c = b.begin_invocation().unwrap();
        a.clone().emit(3).unwrap();
        c.emit(2).unwrap();
        let e = c.emit(1).unwrap_err();
        assert_eq!(e.resource, BudgetResource::Counter(Counter::OutputTokens));
        assert_eq!(c.state.lock().unwrap().output, 2);
        assert_eq!(a.state.lock().unwrap().output, 3);
        assert_eq!(b.snapshot().counters[Counter::OutputTokens as usize], 5);
        let b = ExpansionBudget::new(small());
        let a = b.begin_invocation().unwrap();
        a.emit(4).unwrap();
        assert_eq!(
            a.clone().emit(1).unwrap_err().resource,
            BudgetResource::InvocationOutput
        );
        assert_eq!(b.snapshot().counters[Counter::OutputTokens as usize], 4);
    }
    #[test]
    fn matcher_admission_is_atomic_for_every_resource() {
        for resource in [
            BudgetResource::MatcherStates,
            BudgetResource::Bytes,
            BudgetResource::Counter(Counter::Work),
        ] {
            let mut l = small();
            match resource {
                BudgetResource::MatcherStates => l.matcher_states = 1,
                BudgetResource::Bytes => l.bytes = 1,
                _ => l.counters[Counter::Work as usize] = 1,
            }
            let b = ExpansionBudget::new(l);
            let i = b.begin_invocation().unwrap();
            let e = i.reserve_states(2, 1).unwrap_err();
            assert_eq!(e.resource, resource);
            assert_eq!(i.state.lock().unwrap().states, 0);
            assert_eq!(b.snapshot().temporary_bytes, 0);
            assert_eq!(b.snapshot().counters[Counter::Work as usize], 0);
        }
        let b = ExpansionBudget::new(small());
        let i = b.begin_invocation().unwrap();
        assert_eq!(
            i.reserve_states(u64::MAX, 2).unwrap_err().kind,
            BudgetFailureKind::Overflow
        );
        assert_eq!(i.state.lock().unwrap().states, 0);
        assert_eq!(b.snapshot().temporary_bytes, 0);
    }
    #[test]
    fn state_drop_releases_live_count_but_not_visited_work() {
        let b = ExpansionBudget::new(small());
        let i = b.begin_invocation().unwrap();
        let a = i.reserve_states(4, 1).unwrap();
        drop(a);
        let a = i.clone().reserve_states(4, 1).unwrap();
        drop(a);
        assert_eq!(b.snapshot().counters[Counter::Work as usize], 8);
        assert_eq!(i.state.lock().unwrap().states, 0);
        assert_eq!(b.snapshot().temporary_bytes, 0);
        assert_eq!(
            i.reserve_states(1, 1).unwrap_err().resource,
            BudgetResource::Counter(Counter::Work)
        );
    }
    #[test]
    fn concurrent_clones_cannot_reset_logical_counters() {
        let b = ExpansionBudget::new(small());
        let gate = Arc::new(Barrier::new(16));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let b = b.clone();
                let gate = gate.clone();
                thread::spawn(move || {
                    gate.wait();
                    b.charge(Counter::InputTokens, 1)
                })
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 8);
        let first = b.check().unwrap_err();
        assert!(
            results
                .into_iter()
                .filter_map(Result::err)
                .all(|e| e == first)
        );
        assert_eq!(b.snapshot().counters[Counter::InputTokens as usize], 8);
    }
    #[test]
    fn concurrent_live_leases_cannot_each_claim_the_whole_ceiling() {
        let b = ExpansionBudget::new(small());
        let gate = Arc::new(Barrier::new(2));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let b = b.clone();
                let gate = gate.clone();
                thread::spawn(move || {
                    let result = b.reserve(Storage::Temporary, 16);
                    gate.wait();
                    result.is_ok()
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .map(|t| t.join().unwrap())
                .filter(|success| *success)
                .count(),
            1
        );
        assert_eq!(b.snapshot().temporary_bytes, 0);
        assert_eq!(b.snapshot().peak_bytes, 16);
    }
    #[test]
    fn concurrent_invocation_emission_shares_local_cap() {
        let b = ExpansionBudget::new(small());
        let i = b.begin_invocation().unwrap();
        let gate = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let i = i.clone();
                let gate = gate.clone();
                thread::spawn(move || {
                    gate.wait();
                    i.emit(1)
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .map(|t| t.join().unwrap())
                .filter(Result::is_ok)
                .count(),
            4
        );
        assert_eq!(b.snapshot().counters[Counter::OutputTokens as usize], 4);
    }
    #[test]
    fn poisoned_mutex_is_infrastructure_and_cleanup_is_safe() {
        let b = ExpansionBudget::new(small());
        let lease = b.reserve(Storage::Retained, 4).unwrap();
        let shared = b.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = shared.state.lock().unwrap();
            panic!("test lock poisoning");
        });
        assert_eq!(
            b.check().unwrap_err().kind,
            BudgetFailureKind::Infrastructure
        );
        drop(lease);
        assert_eq!(b.snapshot().retained_bytes, 0);
        let b = ExpansionBudget::new(small());
        let first = b.poison_allocation();
        let shared = b.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = shared.state.lock().unwrap();
            panic!("test existing poison");
        });
        assert_eq!(b.check(), Err(first));
    }
    #[test]
    fn invocation_mutex_poison_does_not_refund_or_block_drop() {
        let b = ExpansionBudget::new(small());
        let i = b.begin_invocation().unwrap();
        let lease = i.reserve_states(2, 2).unwrap();
        let shared = i.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = shared.state.lock().unwrap();
            panic!("test local poison");
        });
        assert_eq!(
            i.emit(0).unwrap_err().kind,
            BudgetFailureKind::Infrastructure
        );
        drop(lease);
        assert_eq!(b.snapshot().temporary_bytes, 0);
        assert_eq!(b.snapshot().counters[Counter::Work as usize], 2);
    }
    #[test]
    fn leases_release_on_unwind_and_cross_thread_drop() {
        let b = ExpansionBudget::new(small());
        let copy = b.clone();
        let _ = std::panic::catch_unwind(move || {
            let _lease = copy.reserve(Storage::Temporary, 8).unwrap();
            panic!("test rollback");
        });
        assert_eq!(b.snapshot().temporary_bytes, 0);
        let lease = b.reserve(Storage::Retained, 8).unwrap();
        thread::spawn(move || drop(lease)).join().unwrap();
        assert_eq!(b.snapshot().retained_bytes, 0);
        b.check().unwrap();
    }
    #[test]
    fn backtrace_cap_is_only_a_lowerable_rendering_policy() {
        let b = ExpansionBudget::new(small());
        assert_eq!(b.limits().backtrace_frames, 2);
        b.check().unwrap();
        // Rendering truncates to this number without charging a semantic depth.
        assert_eq!(
            (0..10).take(b.limits().backtrace_frames as usize).count(),
            2
        );
        b.check().unwrap();
    }
}
