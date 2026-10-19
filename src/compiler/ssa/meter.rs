//! Paid allocation for optimizer producers (OPTIMIZATION.md 13, accounting
//! audit `data/runs/graph-recovery/OPTIMIZER-ACCOUNTING-AUDIT.md`).
//!
//! Contract:
//! - Bytes are admitted through the task's `PlanBudget` BEFORE allocating, for
//!   the exact capacity requested. After the fallible allocation the actual
//!   capacity is reconciled: any surplus is admitted too, or the producer fails.
//! - Scratch (`Table`, `List`) holds a lease that refunds on drop, success or
//!   failure, and lives exactly as long as its buffer.
//! - Published outputs carry an immutable `Receipt`, sealed only after the
//!   output is complete. No mutable budget is stored in a fact.
//! - Work is charged per real element, round, hop or byte visited, with checked
//!   arithmetic. Resource failures stay sticky in the ledger.
//! - Payload before refund: on every failure path the paid payload is
//!   destroyed before the lease paying for it refunds (`Owned`, `Buf`,
//!   `try_seal`, `try_absorb`).
//! - DIAGNOSTIC ALLOCATION DEBT: error-message `String`s built on failure
//!   paths are not admitted to the ledger. They are labeled debt, not paid.
//! - Peak statements are about ledger-reserved (requested/admitted) bytes,
//!   never about physical allocator peak or allocator-internal rounding.
use crate::compiler::ir::{PlanAuthority, PlanBudget, PlanReservation};

/// Task-local view of the planning budget. Never stored in a published fact.
#[derive(Clone, Copy)]
pub struct Meter<'a> {
    budget: &'a PlanBudget,
}

fn overflow() -> String {
    "optimizer accounting arithmetic overflow".into()
}

pub fn mul(a: usize, b: usize) -> Result<usize, String> {
    a.checked_mul(b).ok_or_else(overflow)
}

pub fn add(a: usize, b: usize) -> Result<usize, String> {
    a.checked_add(b).ok_or_else(overflow)
}

/// A private budget at the maximum limits, for unmetered entry points (the
/// reference interpreter and unit tests). Never used by graph producers,
/// which always meter against their snapshot's `PlanBudget`.
pub fn unlimited() -> PlanBudget {
    PlanBudget::default()
}

impl<'a> Meter<'a> {
    pub fn new(budget: &'a PlanBudget) -> Self {
        Self { budget }
    }

    pub fn budget(&self) -> &'a PlanBudget {
        self.budget
    }

    /// Sticky failure: once this ledger has recorded any resource failure,
    /// every later operation (including zero-sized ones) fails too.
    fn live(&self) -> Result<(), String> {
        if self.budget.first_resource_failure().is_some() || self.budget.usage_snapshot().poisoned {
            return Err("generic planning resource limit exceeded (sticky)".into());
        }
        Ok(())
    }

    fn same(&self, authority: &PlanAuthority) -> Result<(), String> {
        if !self.budget.authority().same_authority(authority) {
            return Err("optimizer lease used with a different planning authority".into());
        }
        Ok(())
    }

    #[track_caller]
    pub fn work(&self, amount: usize) -> Result<(), String> {
        self.live()?;
        self.budget.charge_work(amount)
    }

    #[track_caller]
    pub fn work2(&self, a: usize, b: usize) -> Result<(), String> {
        self.work(mul(a, b)?)
    }

    #[track_caller]
    fn reserve(&self, bytes: usize) -> Result<PlanReservation, String> {
        self.live()?;
        self.budget.reserve_bytes(bytes)
    }

    #[track_caller]
    pub fn lease(&self, bytes: usize) -> Result<Lease, String> {
        let reservation = self.reserve(bytes)?;
        Ok(Lease::new(
            self.budget.authority().clone(),
            reservation,
            bytes,
        ))
    }

    /// Fixed-length scratch table filled with `fill`.
    #[track_caller]
    pub fn table<T: Clone>(&self, len: usize, fill: T) -> Result<Table<T>, String> {
        let mut list = self.list(len)?;
        self.work(len)?;
        list.data.resize(len, fill);
        Ok(Table {
            data: list.data,
            _lease: list.lease,
        })
    }

    /// Scratch list with a fixed admitted capacity. Pushing past it fails.
    #[track_caller]
    pub fn list<T>(&self, capacity: usize) -> Result<List<T>, String> {
        let (data, lease) = self.vec(capacity)?;
        Ok(List {
            data,
            limit: capacity,
            lease,
        })
    }

    /// An empty vector with exactly-admitted capacity, plus its lease. Any
    /// allocator surplus is admitted immediately. The caller must never push
    /// past `capacity` (use `List`/`Buf` for enforced limits).
    #[track_caller]
    pub fn vec<T>(&self, capacity: usize) -> Result<(Vec<T>, Lease), String> {
        let element = std::mem::size_of::<T>();
        let mut lease = self.lease(mul(capacity, element)?)?;
        let mut data = Vec::new();
        data.try_reserve_exact(capacity)
            .map_err(|_| "optimizer allocation failed".to_string())?;
        if element != 0 && data.capacity() > capacity {
            lease.admit(self, mul(data.capacity() - capacity, element)?)?;
        }
        Ok((data, lease))
    }

    /// A growable buffer whose logical limit is paid independently of the
    /// allocator capacity. See `Buf`.
    #[track_caller]
    pub fn buf<T>(&self, limit: usize) -> Result<Buf<T>, String> {
        let (data, lease) = self.vec(limit)?;
        Ok(Buf { data, limit, lease })
    }

    /// Fallible clone of a string into exactly-admitted storage: bytes are
    /// admitted BEFORE the allocation, surplus capacity right after it.
    #[track_caller]
    pub fn clone_string(&self, from: &str, lease: &mut Lease) -> Result<String, String> {
        self.work(from.len())?;
        lease.admit(self, from.len())?;
        let mut text = String::new();
        text.try_reserve_exact(from.len())
            .map_err(|_| "optimizer allocation failed".to_string())?;
        if text.capacity() > from.len() {
            lease.admit(self, text.capacity() - from.len())?;
        }
        text.push_str(from);
        Ok(text)
    }
}

/// One paid reservation and the bytes it charged.
#[derive(Debug)]
struct Chunk {
    reservation: PlanReservation,
    bytes: usize,
}

/// Paid bytes, refunded on drop unless sealed into a receipt.
///
/// Bound to one planning authority: every operation checks the meter's
/// authority and the sticky ledger state before touching anything, including
/// no-op and zero-sized operations.
///
/// Growth adds a delta chunk instead of re-admitting the existing total, and
/// absorbing moves the other lease's chunks unchanged, so neither charges a
/// phantom peak. Chunks after the first live in `extra`, whose own storage is
/// paid by `book` (the requested capacity, plus any allocator surplus as a
/// second reservation). When `extra` reallocates, the full new vector is
/// reserved while the old reservation (already charged) is still held, and
/// the old one is released afterwards: the ledger-reserved peak is old +
/// new requested/admitted bytes (ledger accounting, not a physical allocator
/// peak claim).
///
/// `total` caches the data bytes, so `bytes()` is O(1). Shrinking pops each
/// chunk it empties, so every chunk is visited by a shrink at most once over
/// the lease's lifetime (amortized linear). Every chunk moved, visited or
/// retained is charged one unit of work.
#[derive(Debug)]
pub struct Lease {
    authority: PlanAuthority,
    first: Chunk,
    extra: Vec<Chunk>,
    book: [Option<Chunk>; 2],
    total: usize,
}

impl Lease {
    fn new(authority: PlanAuthority, reservation: PlanReservation, bytes: usize) -> Self {
        Lease {
            authority,
            first: Chunk { reservation, bytes },
            extra: Vec::new(),
            book: [None, None],
            total: bytes,
        }
    }

    /// Bytes currently paid for the leased storage (excluding the chunk
    /// bookkeeping, which `book` pays separately).
    pub fn bytes(&self) -> usize {
        self.total
    }

    /// Bytes paid for chunk bookkeeping (not part of `bytes()`).
    pub fn bookkeeping_bytes(&self) -> usize {
        self.book.iter().flatten().map(|chunk| chunk.bytes).sum()
    }

    /// Validate authority and sticky state; every public operation starts here.
    fn check(&self, meter: &Meter<'_>) -> Result<(), String> {
        meter.same(&self.authority)?;
        meter.live()
    }

    /// Ensure room for `more` chunks in `extra`, paying the exact old + new
    /// ledger-reserved reallocation peak. Transactional: on failure `extra`,
    /// `book` and the live ledger fields (`bytes`, `current_reservations`)
    /// are as before. Historical telemetry may advance: work, peak, sticky
    /// failure, and `cumulative_bytes` (a temporary reservation that
    /// succeeded before a later allocator or surplus failure stays counted).
    fn room(&mut self, meter: &Meter<'_>, more: usize) -> Result<(), String> {
        let needed = add(self.extra.len(), more)?;
        if needed <= self.extra.capacity() {
            return Ok(());
        }
        let element = std::mem::size_of::<Chunk>();
        let new_cap = needed.max(add(mul(self.extra.capacity(), 2)?, 2)?);
        let requested = mul(new_cap, element)?;
        // Charge moving the existing chunks before any allocation.
        meter.work(self.extra.len())?;
        // The old vector stays charged by the current `book` until commit.
        let reservation = meter.reserve(requested)?;
        let mut next: Vec<Chunk> = Vec::new();
        next.try_reserve_exact(new_cap)
            .map_err(|_| "optimizer allocation failed".to_string())?;
        let actual = mul(next.capacity(), element)?;
        let surplus = if actual > requested {
            let bytes = actual - requested;
            Some(Chunk {
                reservation: meter.reserve(bytes)?,
                bytes,
            })
        } else {
            None
        };
        // Commit point: nothing below can fail.
        next.append(&mut self.extra);
        self.extra = next;
        self.book = [
            Some(Chunk {
                reservation,
                bytes: requested,
            }),
            surplus,
        ];
        Ok(())
    }

    /// Append data chunks; room must already exist.
    fn append(&mut self, chunk: Chunk) {
        self.total += chunk.bytes;
        self.extra.push(chunk);
    }

    /// Reconcile to exactly `total` paid bytes. Growth admits only the
    /// delta; shrink is exact and decrease-only (never admits).
    #[track_caller]
    pub fn resize(&mut self, meter: &Meter<'_>, total: usize) -> Result<(), String> {
        self.check(meter)?;
        let current = self.total;
        if total > current {
            return self.admit(meter, total - current);
        }
        let mut excess = current - total;
        while excess > 0 {
            meter.work(1)?;
            let chunk = match self.extra.last_mut() {
                Some(chunk) => chunk,
                None => &mut self.first,
            };
            let release = excess.min(chunk.bytes);
            chunk.reservation.shrink(release)?;
            chunk.bytes -= release;
            self.total -= release;
            excess -= release;
            if chunk.bytes == 0 && self.extra.pop().is_none() && excess > 0 {
                return Err("optimizer lease shrink accounting mismatch".into());
            }
        }
        Ok(())
    }

    /// Admit `bytes` more as a delta chunk.
    #[track_caller]
    pub fn admit(&mut self, meter: &Meter<'_>, bytes: usize) -> Result<(), String> {
        self.check(meter)?;
        if bytes == 0 {
            return Ok(());
        }
        add(self.total, bytes)?;
        self.room(meter, 1)?;
        let reservation = meter.reserve(bytes)?;
        self.append(Chunk { reservation, bytes });
        Ok(())
    }

    /// Absorb another lease of the same authority without re-admitting it.
    ///
    /// Non-consuming on failure: every fallible step (authority, sticky state,
    /// overflow, work, chunk-vector room) runs before anything moves, and on
    /// error `other` is returned unchanged with every reservation still
    /// charged, so the caller can destroy the payload it pays for first.
    /// `self` is unchanged on error except for possibly having grown its
    /// paid chunk-vector room.
    #[allow(
        clippy::result_large_err,
        reason = "the error returns the intact paid lease by value; boxing it would allocate unpaid storage on the failure path"
    )]
    #[track_caller]
    pub fn try_absorb(&mut self, meter: &Meter<'_>, other: Lease) -> Result<(), (Lease, String)> {
        let pre = (|| -> Result<(), String> {
            self.check(meter)?;
            other.check(meter)?;
            let moved = add(other.extra.len(), 1)?;
            add(self.total, other.total)?;
            meter.work(moved)?;
            self.room(meter, moved)
        })();
        if let Err(error) = pre {
            return Err((other, error));
        }
        // Commit: pure moves into reserved room, nothing below can fail.
        let Lease {
            first, extra, book, ..
        } = other;
        self.append(first);
        for chunk in extra {
            self.append(chunk);
        }
        // The absorbed lease's chunk vector was consumed and freed above,
        // so its bookkeeping bytes are released, not carried over.
        drop(book);
        Ok(())
    }

    /// Seal into an immutable receipt.
    ///
    /// Two phases. Precommit (authority, sticky state, per-chunk work) can
    /// fail and then returns the whole lease unchanged as
    /// `Err((Some(lease), error))`, every reservation still charged, so the
    /// caller destroys the payload before the lease refunds.
    ///
    /// Commit converts chunks with `PlanReservation::retain`, last chunk
    /// first. At this checkpoint retain is infallible on the metered path,
    /// and on the unmetered path fails only when the ledger mutex is poisoned.
    /// Its destructor then retries the same poisoned mutex and refunds only on
    /// success, and nothing in this repository clears ledger poison, so the
    /// failing chunk stays charged (conservative, never under-charged). The
    /// chunks not yet converted come back as `Some(remaining)`, still
    /// charged, for the caller to drop after the payload. Chunks already
    /// converted are permanently retained.
    #[allow(
        clippy::result_large_err,
        reason = "the error returns the intact paid lease by value; boxing it would allocate unpaid storage on the failure path"
    )]
    pub fn try_seal(self, meter: &Meter<'_>) -> Result<Receipt, (Option<Lease>, String)> {
        let pre = self
            .check(meter)
            .and_then(|()| meter.work(add(self.extra.len(), 1)?));
        if let Err(error) = pre {
            return Err((Some(self), error));
        }
        let bytes = self.total;
        let mut lease = self;
        // Convert `extra` from the tail: popping never reallocates, and the
        // remaining lease stays a well-formed lease at every step.
        while let Some(chunk) = lease.extra.pop() {
            lease.total -= chunk.bytes;
            if let Err(error) = chunk.reservation.retain() {
                // Poisoned ledger: the failing chunk stays charged (above).
                return Err((Some(lease), error));
            }
        }
        let Lease {
            authority,
            first,
            extra,
            book,
            total: _,
        } = lease;
        if let Err(error) = first.reservation.retain() {
            // Poisoned ledger: nothing left unconverted except bookkeeping,
            // which the destructors below refund only if the ledger allows.
            drop(extra);
            drop(book);
            return Err((None, error));
        }
        // The `extra` vector is freed here, so its bookkeeping refunds.
        drop(extra);
        drop(book);
        Ok(Receipt { authority, bytes })
    }

    /// Consuming seal for callers that hold no payload (envelope-only
    /// receipts); a failure refunds immediately, which is correct there.
    pub fn seal(self, meter: &Meter<'_>) -> Result<Receipt, String> {
        self.try_seal(meter).map_err(|(_, error)| error)
    }

    /// Consuming absorb for callers whose payload is already destroyed or
    /// absent.
    pub fn absorb(&mut self, meter: &Meter<'_>, other: Lease) -> Result<(), String> {
        self.try_absorb(meter, other).map_err(|(_, error)| error)
    }
}

/// Immutable evidence that a published optimizer output's storage was paid.
/// Holds no ledger or meter back-reference.
#[derive(Clone, Debug)]
pub struct Receipt {
    authority: PlanAuthority,
    bytes: usize,
}

impl Receipt {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn same_authority(&self, budget: &PlanBudget) -> bool {
        self.authority.same_authority(budget.authority())
    }
}

/// Fixed-length scratch table. Its lease refunds when it drops.
#[derive(Debug)]
pub struct Table<T> {
    data: Vec<T>,
    _lease: Lease,
}

impl<T: Clone> Table<T> {
    pub fn to_vec(&self) -> Vec<T> {
        self.data.clone()
    }
}

impl<T> std::ops::Deref for Table<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.data
    }
}

impl<T> std::ops::DerefMut for Table<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.data
    }
}

/// Scratch list that never grows past its admitted capacity.
#[derive(Debug)]
pub struct List<T> {
    data: Vec<T>,
    /// Paid element limit, independent of the allocator's capacity.
    limit: usize,
    lease: Lease,
}

impl<T> List<T> {
    pub fn push(&mut self, value: T) -> Result<(), String> {
        if self.data.len() >= self.limit || self.data.len() == self.data.capacity() {
            return Err("optimizer scratch exceeded its admitted capacity".into());
        }
        self.data.push(value);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        self.data.pop()
    }

    pub fn clear(&mut self) {
        self.data.clear();
    }

    pub fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.data.retain(keep);
    }

    pub fn truncate(&mut self, len: usize) {
        self.data.truncate(len);
    }

    pub fn swap_remove(&mut self, index: usize) -> T {
        self.data.swap_remove(index)
    }

    pub fn reverse(&mut self) {
        self.data.reverse();
    }

    pub fn sort_unstable_by_key<K: Ord>(&mut self, key: impl FnMut(&T) -> K) {
        self.data.sort_unstable_by_key(key);
    }

    /// Give up the buffer as a published part, transferring its lease.
    pub fn into_parts(self) -> (Vec<T>, Lease) {
        (self.data, self.lease)
    }
}

impl<T> std::ops::Deref for List<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.data
    }
}

impl<T> std::ops::DerefMut for List<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.data
    }
}

/// Growable output buffer with an independently paid logical `limit`.
/// `data` is declared before `lease`, so a dropped buffer destroys its
/// contents before refunding.
///
/// Appends check the limit before writing. Growth is the only way to raise
/// the limit, and it is transactional: authority and sticky state are checked
/// before any change (fast path included), the full new buffer is reserved
/// while the old one (already charged) is still held, so the ledger-reserved
/// peak is old + new requested/admitted bytes (not a physical allocator peak
/// claim), the copy is charged as work, and the new buffer and
/// lease replace the old pair only after every fallible step succeeded. On
/// any failure the data, pointer, capacity, limit, lease and the live ledger
/// fields (`bytes`, `current_reservations`) are unchanged. Historical
/// telemetry may advance: work, peak, sticky failure, and `cumulative_bytes`
/// (a temporary reservation that succeeded before a later allocator or
/// surplus failure stays counted).
#[derive(Debug)]
pub struct Buf<T> {
    data: Vec<T>,
    limit: usize,
    lease: Lease,
}

impl<T> Buf<T> {
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Paid element limit (independent of allocator capacity).
    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }

    pub fn lease_bytes(&self) -> usize {
        self.lease.bytes()
    }

    pub fn push(&mut self, value: T) -> Result<(), String> {
        if self.data.len() >= self.limit || self.data.len() == self.data.capacity() {
            return Err("optimizer buffer exceeded its paid limit".into());
        }
        self.data.push(value);
        Ok(())
    }

    /// Raise the paid limit by at least `more` elements.
    pub fn grow(&mut self, meter: &Meter<'_>, more: usize) -> Result<(), String> {
        self.lease.check(meter)?;
        let element = std::mem::size_of::<T>();
        let new_limit = add(self.limit, more)?;
        if new_limit <= self.data.capacity() {
            self.limit = new_limit;
            return Ok(());
        }
        let requested = mul(new_limit, element)?;
        // Copy work, charged before allocating.
        meter.work(self.data.len())?;
        // New lease while the old one is still held: ledger-reserved peak is
        // old + new admitted bytes.
        let mut lease = meter.lease(requested)?;
        let mut next: Vec<T> = Vec::new();
        next.try_reserve_exact(new_limit)
            .map_err(|_| "optimizer allocation failed".to_string())?;
        let actual = mul(next.capacity(), element)?;
        if actual > requested {
            lease.admit(meter, actual - requested)?;
        }
        // Commit point: nothing below can fail.
        next.append(&mut self.data);
        self.data = next;
        self.lease = lease;
        self.limit = new_limit;
        Ok(())
    }

    /// Ensure room for `more` elements, growing geometrically (paid).
    pub fn reserve_growing(&mut self, meter: &Meter<'_>, more: usize) -> Result<(), String> {
        self.lease.check(meter)?;
        let end = add(self.data.len(), more)?;
        if end > self.limit {
            let target = end.max(add(mul(self.limit, 2)?, 8)?);
            self.grow(meter, target - self.limit)?;
        }
        Ok(())
    }

    /// Push, growing first (paid, transactional) when the limit is reached.
    pub fn push_growing(&mut self, meter: &Meter<'_>, value: T) -> Result<(), String> {
        self.reserve_growing(meter, 1)?;
        self.push(value)
    }

    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    pub fn clear(&mut self) {
        self.data.clear();
    }

    pub fn truncate(&mut self, len: usize) {
        self.data.truncate(len);
    }

    pub fn swap_remove(&mut self, index: usize) -> T {
        self.data.swap_remove(index)
    }

    pub fn into_parts(self) -> (Vec<T>, Lease) {
        (self.data, self.lease)
    }

    /// Mutable access to the paying lease (to absorb other leases into it).
    pub fn lease_mut(&mut self) -> &mut Lease {
        &mut self.lease
    }

    /// Move `other`'s payment into this buffer's lease and hand back its
    /// data, now paid by `self`. Non-consuming on failure: `other` comes back
    /// coupled (data with its own unchanged lease).
    #[allow(
        clippy::result_large_err,
        reason = "the error returns the intact paid lease by value; boxing it would allocate unpaid storage on the failure path"
    )]
    pub fn absorb_from<U>(
        &mut self,
        meter: &Meter<'_>,
        other: Buf<U>,
    ) -> Result<Vec<U>, (Buf<U>, String)> {
        let Buf { data, limit, lease } = other;
        match self.lease.try_absorb(meter, lease) {
            Ok(()) => Ok(data),
            Err((lease, error)) => Err((Buf { data, limit, lease }, error)),
        }
    }
}

impl<T> std::ops::DerefMut for Buf<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.data
    }
}

impl<T: Copy> Buf<T> {
    pub fn extend(&mut self, values: &[T]) -> Result<(), String> {
        let end = add(self.data.len(), values.len())?;
        if end > self.limit || end > self.data.capacity() {
            return Err("optimizer buffer exceeded its paid limit".into());
        }
        self.data.extend_from_slice(values);
        Ok(())
    }

    /// Append, growing first (paid, transactional) when `values` would not
    /// fit. Authority and sticky state are checked before anything else.
    pub fn extend_growing(&mut self, meter: &Meter<'_>, values: &[T]) -> Result<(), String> {
        self.reserve_growing(meter, values.len())?;
        self.extend(values)
    }
}

impl<T> std::ops::Deref for Buf<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.data
    }
}
