//! tests/ui/threads-sendsync/tls-in-global-alloc.rs: the methods of the `#[global_allocator]`
//! are called through symbols rustc writes, not by anything the program names. They were left
//! to rustc without being counted, and wrote to rustc's copy of a static the program read
//! ra-jit's copy of.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, at: *mut u8, layout: Layout) {
        unsafe { System.dealloc(at, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

pub fn run() -> bool {
    let before = ALLOCATED.load(Ordering::Relaxed);
    let boxed = std::hint::black_box(Box::new(7u8));
    drop(boxed);
    ALLOCATED.load(Ordering::Relaxed) > before
}
