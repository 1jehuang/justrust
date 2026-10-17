//! Opt-in, process-lifetime requested-allocation accounting for the CLI.
//!
//! Include this module only in the `jr-rust` binary with `allocation-profile`
//! enabled, and install `ProfilingAllocator::new(std::alloc::System)` there.
//! Nothing in this module registers a global allocator for library consumers.
//!
//! Bytes are the allocator's requested layout sizes, NOT RSS, usable allocation
//! sizes, allocator metadata, or compiler-category attribution. Only calls that
//! actually reach this wrapper are observed. Optimized-away allocations and
//! allocations bypassing Rust's global allocator are not counted. Totals start
//! before `main` and are not reset between compiler phases.
//!
//! A successful realloc contributes the full old size to freed bytes and the
//! full new size to allocated bytes, even when the address stays unchanged. It
//! increments only `reallocation_calls`, not the allocation/deallocation call
//! counters. A failed allocation or realloc changes nothing. Realloc changes
//! live bytes by its net size difference, without inventing a transient peak.
//!
//! Hooks use only atomic operations and the wrapped allocator, with no logging,
//! allocation, locks, thread-local initialization, or panicking arithmetic.
//! Cumulative counters saturate at `u64::MAX`. Settled live bytes are exact within
//! the address space supported by this CLI (Linux x86-64). Live and peak bytes
//! follow hook-accounting order, not instantaneous occupancy inside concurrent
//! allocator calls. Relaxed atomic snapshots are individually race-free but not
//! a transaction across fields. Take a snapshot after worker joins for settled
//! worker totals, and before formatting the snapshot so reporting allocations
//! are not included in that snapshot.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicU64, Ordering};

/// Primitive fields for conversion to the CLI's chosen serialization format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocationSnapshot {
    /// Successful `alloc` and `alloc_zeroed` calls, excluding reallocations.
    pub allocation_calls: u64,
    /// Completed `dealloc` calls, excluding reallocations.
    pub deallocation_calls: u64,
    /// Successful `realloc` calls, including same-size/in-place reallocations.
    pub reallocation_calls: u64,
    pub requested_allocated_bytes: u64,
    pub requested_freed_bytes: u64,
    pub requested_live_bytes: u64,
    pub peak_requested_live_bytes: u64,
}

impl AllocationSnapshot {
    pub fn to_json(self) -> String {
        format!(
            "{{\"scope\":\"process lifetime through compiler return\",\"bytes_kind\":\"Rust global allocator requested layout bytes, not RSS\",\"allocation_calls\":{},\"deallocation_calls\":{},\"reallocation_calls\":{},\"requested_allocated_bytes\":{},\"requested_freed_bytes\":{},\"requested_live_bytes\":{},\"peak_requested_live_bytes\":{}}}",
            self.allocation_calls,
            self.deallocation_calls,
            self.reallocation_calls,
            self.requested_allocated_bytes,
            self.requested_freed_bytes,
            self.requested_live_bytes,
            self.peak_requested_live_bytes
        )
    }
}

#[derive(Default)]
struct Counters {
    allocation_calls: AtomicU64,
    deallocation_calls: AtomicU64,
    reallocation_calls: AtomicU64,
    allocated_bytes: AtomicU64,
    freed_bytes: AtomicU64,
    live_bytes: AtomicU64,
    peak_live_bytes: AtomicU64,
}

impl Counters {
    const fn new() -> Self {
        Self {
            allocation_calls: AtomicU64::new(0),
            deallocation_calls: AtomicU64::new(0),
            reallocation_calls: AtomicU64::new(0),
            allocated_bytes: AtomicU64::new(0),
            freed_bytes: AtomicU64::new(0),
            live_bytes: AtomicU64::new(0),
            peak_live_bytes: AtomicU64::new(0),
        }
    }

    fn add_total(counter: &AtomicU64, amount: u64) {
        let mut value = counter.load(Ordering::Relaxed);
        loop {
            match counter.compare_exchange_weak(
                value,
                value.saturating_add(amount),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => value = actual,
            }
        }
    }

    fn grow_live(&self, amount: u64) {
        let previous = self.live_bytes.fetch_add(amount, Ordering::Relaxed);
        // Valid live requested sizes fit in the supported process address space.
        // Wrapping arithmetic additionally prevents unwind in allocator hooks.
        self.peak_live_bytes
            .fetch_max(previous.wrapping_add(amount), Ordering::Relaxed);
    }

    fn allocation_result(&self, size: usize, successful: bool) {
        if successful {
            Self::add_total(&self.allocation_calls, 1);
            Self::add_total(&self.allocated_bytes, size as u64);
            self.grow_live(size as u64);
        }
    }

    fn deallocation(&self, size: usize) {
        Self::add_total(&self.deallocation_calls, 1);
        Self::add_total(&self.freed_bytes, size as u64);
        self.live_bytes.fetch_sub(size as u64, Ordering::Relaxed);
    }

    fn reallocation_result(&self, old_size: usize, new_size: usize, successful: bool) {
        if !successful {
            // The original allocation remains owned by the caller on failure.
            return;
        }
        Self::add_total(&self.reallocation_calls, 1);
        Self::add_total(&self.allocated_bytes, new_size as u64);
        Self::add_total(&self.freed_bytes, old_size as u64);
        if new_size > old_size {
            self.grow_live((new_size - old_size) as u64);
        } else {
            self.live_bytes
                .fetch_sub((old_size - new_size) as u64, Ordering::Relaxed);
        }
    }

    fn snapshot(&self) -> AllocationSnapshot {
        AllocationSnapshot {
            allocation_calls: self.allocation_calls.load(Ordering::Relaxed),
            deallocation_calls: self.deallocation_calls.load(Ordering::Relaxed),
            reallocation_calls: self.reallocation_calls.load(Ordering::Relaxed),
            requested_allocated_bytes: self.allocated_bytes.load(Ordering::Relaxed),
            requested_freed_bytes: self.freed_bytes.load(Ordering::Relaxed),
            requested_live_bytes: self.live_bytes.load(Ordering::Relaxed),
            peak_requested_live_bytes: self.peak_live_bytes.load(Ordering::Relaxed),
        }
    }
}

/// A wrapper installed by the binary only, never by the engine library.
pub struct ProfilingAllocator<A> {
    inner: A,
    counters: Counters,
}

impl<A> ProfilingAllocator<A> {
    pub const fn new(inner: A) -> Self {
        Self {
            inner,
            counters: Counters::new(),
        }
    }

    pub fn snapshot(&self) -> AllocationSnapshot {
        self.counters.snapshot()
    }
}

// SAFETY: All allocation operations and their contracts are delegated unchanged
// to the wrapped GlobalAlloc. Accounting never dereferences returned pointers,
// changes layouts, unwinds, or calls the global allocator recursively.
unsafe impl<A: GlobalAlloc> GlobalAlloc for ProfilingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies the wrapped allocator's required layout.
        let result = unsafe { self.inner.alloc(layout) };
        self.counters
            .allocation_result(layout.size(), !result.is_null());
        result
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies the wrapped allocator's required layout.
        let result = unsafe { self.inner.alloc_zeroed(layout) };
        self.counters
            .allocation_result(layout.size(), !result.is_null());
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies a live pointer and its original layout.
        unsafe { self.inner.dealloc(ptr, layout) };
        self.counters.deallocation(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: The caller supplies a live pointer, its layout, and a valid
        // nonzero new size. A null result leaves the old allocation untouched.
        let result = unsafe { self.inner.realloc(ptr, layout, new_size) };
        self.counters
            .reallocation_result(layout.size(), new_size, !result.is_null());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_deallocation_and_peak_are_exact() {
        let counters = Counters::new();
        assert_eq!(counters.snapshot(), AllocationSnapshot::default());
        counters.allocation_result(40, true);
        counters.allocation_result(24, true);
        counters.deallocation(40);
        assert_eq!(
            counters.snapshot(),
            AllocationSnapshot {
                allocation_calls: 2,
                deallocation_calls: 1,
                reallocation_calls: 0,
                requested_allocated_bytes: 64,
                requested_freed_bytes: 40,
                requested_live_bytes: 24,
                peak_requested_live_bytes: 64,
            }
        );
        counters.deallocation(24);
        assert_eq!(counters.snapshot().requested_live_bytes, 0);
        assert_eq!(counters.snapshot().peak_requested_live_bytes, 64);
    }

    #[test]
    fn failed_allocation_and_reallocation_preserve_every_counter() {
        let counters = Counters::new();
        counters.allocation_result(32, true);
        let before = counters.snapshot();
        counters.allocation_result(999, false);
        counters.reallocation_result(32, 4096, false);
        counters.reallocation_result(32, 8, false);
        assert_eq!(counters.snapshot(), before);
        // The original 32-byte allocation is still valid and accounted for.
        counters.deallocation(32);
        assert_eq!(counters.snapshot().requested_live_bytes, 0);
    }

    #[test]
    fn realloc_grow_shrink_and_same_size_count_full_requested_sizes() {
        let counters = Counters::new();
        counters.allocation_result(10, true);
        counters.reallocation_result(10, 30, true);
        counters.reallocation_result(30, 5, true);
        counters.reallocation_result(5, 5, true);
        assert_eq!(
            counters.snapshot(),
            AllocationSnapshot {
                allocation_calls: 1,
                deallocation_calls: 0,
                reallocation_calls: 3,
                requested_allocated_bytes: 50,
                requested_freed_bytes: 45,
                requested_live_bytes: 5,
                peak_requested_live_bytes: 30,
            }
        );
        counters.deallocation(5);
        assert_eq!(counters.snapshot().requested_live_bytes, 0);
        assert_eq!(counters.snapshot().requested_freed_bytes, 50);
    }

    #[test]
    fn cumulative_totals_saturate_without_affecting_live_accounting() {
        let counters = Counters::new();
        counters.allocation_calls.store(u64::MAX, Ordering::Relaxed);
        counters
            .allocated_bytes
            .store(u64::MAX - 1, Ordering::Relaxed);
        counters.allocation_result(4, true);
        assert_eq!(counters.snapshot().allocation_calls, u64::MAX);
        assert_eq!(counters.snapshot().requested_allocated_bytes, u64::MAX);
        assert_eq!(counters.snapshot().requested_live_bytes, 4);
        counters.deallocation(4);
        assert_eq!(counters.snapshot().requested_live_bytes, 0);
    }

    #[test]
    fn concurrent_accounting_has_exact_settled_totals_and_peak() {
        const THREADS: usize = 4;
        const ROUNDS: usize = 100;
        const INITIAL: usize = 8;
        const GROWN: usize = 16;
        let counters = Counters::new();
        let barrier = std::sync::Barrier::new(THREADS);
        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    for _ in 0..ROUNDS {
                        counters.allocation_result(INITIAL, true);
                        counters.reallocation_result(INITIAL, GROWN, true);
                        // Every thread has a grown allocation before any free.
                        barrier.wait();
                        counters.deallocation(GROWN);
                        // No next-round allocation overlaps this round's frees.
                        barrier.wait();
                    }
                });
            }
        });
        let operations = (THREADS * ROUNDS) as u64;
        assert_eq!(
            counters.snapshot(),
            AllocationSnapshot {
                allocation_calls: operations,
                deallocation_calls: operations,
                reallocation_calls: operations,
                requested_allocated_bytes: operations * (INITIAL + GROWN) as u64,
                requested_freed_bytes: operations * (INITIAL + GROWN) as u64,
                requested_live_bytes: 0,
                peak_requested_live_bytes: (THREADS * GROWN) as u64,
            }
        );
    }
}
