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
use crate::compiler::ir::{PlanBudget, PlanReservation, RetainedStorage};

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

    #[track_caller]
    pub fn work(&self, amount: usize) -> Result<(), String> {
        self.budget.charge_work(amount)
    }

    #[track_caller]
    pub fn work2(&self, a: usize, b: usize) -> Result<(), String> {
        self.work(mul(a, b)?)
    }

    #[track_caller]
    pub fn lease(&self, bytes: usize) -> Result<Lease, String> {
        Ok(Lease {
            reservation: self.budget.reserve_bytes(bytes)?,
            bytes,
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
        Ok(List { data, lease })
    }

    /// An empty vector with exactly-admitted capacity, plus its lease. The
    /// caller decides whether the lease is scratch (dropped) or sealed.
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

    /// Fallible clone of a slice into an exactly-admitted vector.
    #[track_caller]
    pub fn clone_vec<T: Clone>(&self, from: &[T], lease: &mut Lease) -> Result<Vec<T>, String> {
        self.work(from.len())?;
        let (mut data, part) = self.vec(from.len())?;
        lease.absorb(self, part)?;
        data.extend_from_slice(from);
        Ok(data)
    }

    /// Fallible clone of a string into exactly-admitted storage.
    #[track_caller]
    pub fn clone_string(&self, from: &str, lease: &mut Lease) -> Result<String, String> {
        self.work(from.len())?;
        let mut text = String::new();
        lease.admit(self, from.len())?;
        text.try_reserve_exact(from.len())
            .map_err(|_| "optimizer allocation failed".to_string())?;
        if text.capacity() > from.len() {
            lease.admit(self, text.capacity() - from.len())?;
        }
        text.push_str(from);
        Ok(text)
    }
}

/// Paid bytes, refunded on drop unless sealed into a receipt. Growing or
/// merging reserves the new combined total BEFORE releasing the old charge,
/// so old and new storage overlap in the ledger exactly as they do in memory.
#[derive(Debug)]
pub struct Lease {
    reservation: PlanReservation,
    bytes: usize,
}

impl Lease {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    fn regrow(&mut self, meter: &Meter<'_>, total: usize) -> Result<(), String> {
        let combined = meter.budget.reserve_bytes(total)?;
        // Old charge released only after the combined one is admitted.
        drop(std::mem::replace(&mut self.reservation, combined));
        self.bytes = total;
        Ok(())
    }

    /// Reconcile to exactly `total` bytes. Growing reserves the new total
    /// before releasing the old charge; shrinking also re-reserves first, so
    /// the ledger never under-reports while both charges are live.
    #[track_caller]
    pub fn resize(&mut self, meter: &Meter<'_>, total: usize) -> Result<(), String> {
        if total == self.bytes {
            return Ok(());
        }
        self.regrow(meter, total)
    }

    /// Admit `bytes` more into this lease.
    #[track_caller]
    pub fn admit(&mut self, meter: &Meter<'_>, bytes: usize) -> Result<(), String> {
        if bytes == 0 {
            return Ok(());
        }
        self.regrow(meter, add(self.bytes, bytes)?)
    }

    /// Absorb another lease. Both stay charged until the merged one exists.
    #[track_caller]
    pub fn absorb(&mut self, meter: &Meter<'_>, other: Lease) -> Result<(), String> {
        if other.bytes == 0 {
            return Ok(());
        }
        let total = add(self.bytes, other.bytes)?;
        self.regrow(meter, total)?;
        drop(other);
        Ok(())
    }

    /// Seal into an immutable receipt for a published output.
    pub fn seal(self) -> Result<Receipt, String> {
        Ok(Receipt {
            storage: self.reservation.retain()?,
        })
    }
}

/// Immutable evidence that a published optimizer output's storage was paid.
/// Holds no ledger or meter back-reference.
#[derive(Clone, Debug)]
pub struct Receipt {
    storage: RetainedStorage,
}

impl Receipt {
    pub fn bytes(&self) -> usize {
        self.storage.bytes()
    }

    pub fn same_authority(&self, budget: &PlanBudget) -> bool {
        self.storage.authority().same_authority(budget.authority())
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
    lease: Lease,
}

impl<T> List<T> {
    pub fn push(&mut self, value: T) -> Result<(), String> {
        if self.data.len() == self.data.capacity() {
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
