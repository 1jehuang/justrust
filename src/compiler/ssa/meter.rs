//! Paid allocation for optimizer producers (OPTIMIZATION.md 13, accounting
//! audit `data/runs/graph-recovery/OPTIMIZER-ACCOUNTING-AUDIT.md`).
//!
//! Contract:
//! - Bytes are admitted through the task's `PlanBudget` BEFORE allocating, for
//!   the exact capacity requested. After the fallible allocation the actual
//!   capacity is reconciled: any surplus is admitted too, or the producer fails.
//! - Scratch (`Table`, `List`) holds a lease that refunds on drop, success or
//!   failure, and lives exactly as long as its buffer.
//! - Published outputs carry an immutable `ir::RetainedStorage` receipt, sealed
//!   only after the output is complete. No mutable budget is stored in a fact.
//! - Work is charged per real element, round, hop or byte visited, with checked
//!   arithmetic. Resource failures stay sticky in the ledger.
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
        Ok(Lease {
            authority: self.budget.authority().clone(),
            first: Chunk {
                reservation,
                bytes,
                book: false,
            },
            extra: Vec::new(),
            book: None,
        })
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
    /// Bookkeeping (chunk-vector storage) rather than leased data bytes.
    book: bool,
}

/// Paid bytes, refunded on drop unless sealed into a receipt.
///
/// Bound to one planning authority: every operation with a different
/// authority is rejected before any ledger change, and every operation fails
/// once the ledger has a sticky resource failure.
///
/// Growth adds a delta chunk instead of re-admitting the existing total, and
/// absorbing moves the other lease's chunks over unchanged, so neither causes
/// a phantom peak. The first chunk is inline; further chunks live in `extra`,
/// a vector whose own storage is paid by the `book` chunk (old and new chunk
/// vectors are both paid while the vector reallocates).
#[derive(Debug)]
pub struct Lease {
    authority: PlanAuthority,
    first: Chunk,
    extra: Vec<Chunk>,
    book: Option<Chunk>,
}

impl Lease {
    /// Bytes currently paid for the leased storage (excluding the chunk
    /// bookkeeping, which is paid separately by `book`).
    pub fn bytes(&self) -> usize {
        self.first.bytes
            + self
                .extra
                .iter()
                .filter(|chunk| !chunk.book)
                .map(|chunk| chunk.bytes)
                .sum::<usize>()
    }

    fn push(&mut self, meter: &Meter<'_>, chunk: Chunk) -> Result<(), String> {
        if self.extra.len() == self.extra.capacity() {
            let element = std::mem::size_of::<Chunk>();
            let new_cap = add(mul(self.extra.capacity(), 2)?, 2)?;
            let old_bytes = mul(self.extra.capacity(), element)?;
            // Old and new chunk vectors coexist during reallocation.
            let peak = meter.reserve(add(old_bytes, mul(new_cap, element)?)?)?;
            self.extra
                .try_reserve_exact(new_cap - self.extra.len())
                .map_err(|_| "optimizer allocation failed".to_string())?;
            let steady_bytes = mul(self.extra.capacity(), element)?;
            let steady = meter.reserve(steady_bytes)?;
            drop(peak);
            self.book = Some(Chunk {
                reservation: steady,
                bytes: steady_bytes,
                book: true,
            });
        }
        self.extra.push(chunk);
        Ok(())
    }

    /// Reconcile to at least `total` paid bytes. Growth admits only the delta.
    /// A shrink keeps the existing charge: conservative (it may over-report
    /// by the released surplus until the lease drops), never under-reporting.
    #[track_caller]
    pub fn resize(&mut self, meter: &Meter<'_>, total: usize) -> Result<(), String> {
        meter.same(&self.authority)?;
        meter.live()?;
        let current = self.bytes();
        if total > current {
            return self.admit(meter, total - current);
        }
        // Shrink exactly, decrease-only: release trailing data chunks, then
        // shrink the last needed chunk in place. Never admits anything.
        let mut excess = current - total;
        while excess > 0 {
            let chunk = match self
                .extra
                .iter_mut()
                .rev()
                .find(|chunk| !chunk.book && chunk.bytes > 0)
            {
                Some(chunk) => chunk,
                None => &mut self.first,
            };
            let release = excess.min(chunk.bytes);
            if release == 0 {
                return Err("optimizer lease shrink accounting mismatch".into());
            }
            chunk.reservation.shrink(release)?;
            chunk.bytes -= release;
            excess -= release;
        }
        Ok(())
    }

    /// Admit `bytes` more as a delta chunk.
    #[track_caller]
    pub fn admit(&mut self, meter: &Meter<'_>, bytes: usize) -> Result<(), String> {
        meter.same(&self.authority)?;
        meter.live()?;
        if bytes == 0 {
            return Ok(());
        }
        let reservation = meter.reserve(bytes)?;
        self.push(
            meter,
            Chunk {
                reservation,
                bytes,
                book: false,
            },
        )
    }

    /// Absorb another lease of the same authority without re-admitting it.
    #[track_caller]
    pub fn absorb(&mut self, meter: &Meter<'_>, other: Lease) -> Result<(), String> {
        meter.same(&self.authority)?;
        meter.same(&other.authority)?;
        meter.live()?;
        let Lease {
            first, extra, book, ..
        } = other;
        self.push(meter, first)?;
        for chunk in extra {
            self.push(meter, chunk)?;
        }
        if let Some(book) = book {
            self.push(meter, book)?;
        }
        Ok(())
    }

    /// Seal into an immutable receipt. Same authority and no sticky failure
    /// required.
    pub fn seal(self, meter: &Meter<'_>) -> Result<Receipt, String> {
        meter.same(&self.authority)?;
        meter.live()?;
        let bytes = self.bytes();
        let Lease {
            authority,
            first,
            extra,
            book,
        } = self;
        // Retained reservations stay charged for the task's lifetime; the
        // receipt itself holds no heap storage, so its size is static.
        drop(first.reservation.retain()?);
        for chunk in extra {
            drop(chunk.reservation.retain()?);
        }
        if let Some(book) = book {
            drop(book.reservation.retain()?);
        }
        Ok(Receipt { authority, bytes })
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

/// Growable output buffer with an independently paid logical `limit`. Every
/// append checks the limit before writing; raising the limit (`grow`) admits
/// the old+new capacity peak before reallocating and reconciles the real
/// capacity immediately after. Never grows silently.
#[derive(Debug)]
pub struct Buf<T> {
    data: Vec<T>,
    limit: usize,
    lease: Lease,
}

impl<T: Copy> Buf<T> {
    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn push(&mut self, value: T) -> Result<(), String> {
        if self.data.len() >= self.limit || self.data.len() == self.data.capacity() {
            return Err("optimizer buffer exceeded its paid limit".into());
        }
        self.data.push(value);
        Ok(())
    }

    pub fn extend(&mut self, values: &[T]) -> Result<(), String> {
        let end = add(self.data.len(), values.len())?;
        if end > self.limit || end > self.data.capacity() {
            return Err("optimizer buffer exceeded its paid limit".into());
        }
        self.data.extend_from_slice(values);
        Ok(())
    }

    /// Raise the paid limit by `more` elements, paying the reallocation peak.
    pub fn grow(&mut self, meter: &Meter<'_>, more: usize) -> Result<(), String> {
        let element = std::mem::size_of::<T>();
        let new_limit = add(self.limit, more)?;
        if new_limit <= self.data.capacity() {
            self.limit = new_limit;
            return Ok(());
        }
        let old_bytes = mul(self.data.capacity(), element)?;
        let new_bytes = mul(new_limit, element)?;
        // Old and new buffers coexist during reallocation.
        let peak = meter.reserve(add(old_bytes, new_bytes)?)?;
        self.data
            .try_reserve_exact(new_limit - self.data.len())
            .map_err(|_| "optimizer allocation failed".to_string())?;
        // Steady state: the lease must cover the real new capacity.
        let actual = mul(self.data.capacity(), element)?;
        self.lease.resize(meter, actual)?;
        drop(peak);
        self.limit = new_limit;
        Ok(())
    }

    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    /// Append, raising the paid limit first (old+new peak paid by `grow`)
    /// when `values` would not fit. Used only by producers whose final size
    /// is not known up front (the paid panic adapter).
    pub fn extend_growing(&mut self, meter: &Meter<'_>, values: &[T]) -> Result<(), String> {
        let end = add(self.data.len(), values.len())?;
        if end > self.limit {
            let target = end.max(mul(self.limit, 2)?);
            self.grow(meter, target - self.limit)?;
        }
        self.extend(values)
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    pub fn into_parts(self) -> (Vec<T>, Lease) {
        (self.data, self.lease)
    }
}
