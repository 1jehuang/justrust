//! bun keeps slices in arenas: `Box<[T], &MimallocArena>`. A `Box` beside an allocator that has
//! a size is not a pointer (and a length) alone, and its pointer is a field wherever the layout
//! put it: ra-jit read it at the start, and left a deref of one to rustc. `allocator_api` is a
//! nightly feature, so breakers.py runs this crate's tests with RUSTC_BOOTSTRAP=1.
#![feature(allocator_api)]

use std::alloc::{AllocError, Allocator, Global, Layout};
use std::cell::Cell;
use std::ptr::NonNull;

pub struct Counting {
    pub live: Cell<usize>,
}

unsafe impl Allocator for &Counting {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        self.live.set(self.live.get() + 1);
        Global.allocate(layout)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        self.live.set(self.live.get() - 1);
        unsafe { Global.deallocate(ptr, layout) }
    }
}

/// The sum through a boxed slice and a boxed value, and how many allocations were live before
/// and after the slice was dropped.
pub fn sum(arena: &Counting) -> (u32, usize, usize) {
    let mut boxed: Box<[u32], &Counting> = Box::new_in([1, 2, 3, 4], arena);
    boxed[1] = 20;
    let total: u32 = boxed.iter().sum();
    let one: Box<u64, &Counting> = Box::new_in(7, arena);
    let live = arena.live.get();
    drop(boxed);
    (total + *one as u32, live, arena.live.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxed_slices_in_an_allocator() {
        let arena = Counting { live: Cell::new(0) };
        assert_eq!(sum(&arena), (35, 2, 1));
        assert_eq!(arena.live.get(), 0);
    }
}
