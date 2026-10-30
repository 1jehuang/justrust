//! What each thread interned of the GC-managed values lately, looked up before the shared map.
//!
//! The values a thread interns are mostly ones it interned a moment ago (`usize`, `&str`, the
//! arguments of the function it is compiling), and the shard of the map that holds one is
//! the same for every thread: with many threads, taking even the shard's read lock has them
//! wait on each other for the lock's cache line. A thread's own table has no lock. A GC-managed
//! value is freed only by a collection ([`crate::GarbageCollector::collect`]), which starts a
//! new epoch: a table of an older one is emptied before it is read.

use std::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU64, Ordering},
};

/// The collections there have been.
pub(crate) static EPOCH: AtomicU64 = AtomicU64::new(0);

const SLOTS: usize = 8192;

#[derive(Clone, Copy)]
struct Slot {
    hash: u64,
    /// The map the value is of, which tells what type it is.
    storage: usize,
    value: *const (),
}

const EMPTY: Slot = Slot { hash: 0, storage: 0, value: std::ptr::null() };

struct Table {
    epoch: u64,
    slots: Box<[Slot]>,
}

thread_local! {
    static TABLE: UnsafeCell<Option<Table>> = const { UnsafeCell::new(None) };
}

#[inline]
fn slot(storage: usize, hash: u64) -> usize {
    (hash.rotate_left(26) ^ storage as u64) as usize & (SLOTS - 1)
}

/// The value of the map at `storage` with `hash` this thread interned in this epoch, when
/// `matches` says it is the one asked for.
#[inline]
pub(crate) fn find(
    storage: usize,
    hash: u64,
    matches: impl FnOnce(*const ()) -> bool,
) -> Option<*const ()> {
    let epoch = EPOCH.load(Ordering::Acquire);
    // SAFETY: The table is this thread's, and no reference to it outlives the closure.
    let found = TABLE.with(|table| unsafe {
        let table = (*table.get()).as_mut()?;
        if table.epoch != epoch {
            table.slots.fill(EMPTY);
            table.epoch = epoch;
            return None;
        }
        let it = table.slots[slot(storage, hash)];
        (it.storage == storage && it.hash == hash).then_some(it.value)
    })?;
    matches(found).then_some(found)
}

/// Has this thread find `value`, of the map at `storage` with `hash`, next time.
#[inline]
pub(crate) fn remember(storage: usize, hash: u64, value: *const ()) {
    let epoch = EPOCH.load(Ordering::Acquire);
    // SAFETY: The table is this thread's, and no reference to it outlives the closure.
    TABLE.with(|table| unsafe {
        let table = (*table.get())
            .get_or_insert_with(|| Table { epoch, slots: vec![EMPTY; SLOTS].into_boxed_slice() });
        if table.epoch != epoch {
            table.slots.fill(EMPTY);
            table.epoch = epoch;
        }
        table.slots[slot(storage, hash)] = Slot { hash, storage, value };
    });
}
