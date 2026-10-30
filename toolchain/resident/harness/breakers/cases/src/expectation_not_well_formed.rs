//! library/coretests pin_macro.rs, `unsize_coercion`: `pin!([x; 2])` where a `Pin<&mut [T]>`
//! is wanted. The helper `pin!` calls returns `Pin<&mut T>` for a `&mut Helper<T>`, and the
//! wanted type would make that `Helper<[T]>`, which is no type: rustc drops what it expected
//! of the argument and unsizes the result instead. ra-jit took nothing to be needed for a type
//! to be well formed, called the helper with `[T]`, and left the caller to rustc for having
//! no layout.

use std::marker::PhantomPinned;
use std::pin::{pin, Pin};

struct Held<T> {
    value: T,
}

#[inline(never)]
fn held<T>(of: &mut Held<T>) -> &mut T {
    &mut of.value
}

#[inline(never)]
fn length(of: Pin<&mut [PhantomPinned]>) -> usize {
    of.len()
}

fn pinned() -> usize {
    let slice: Pin<&mut [PhantomPinned]> = pin!([PhantomPinned; 2]);
    length(slice)
}

fn through_a_sized_parameter() -> usize {
    let mut three = Held { value: [1u8; 3] };
    let slice: &mut [u8] = held(&mut three);
    slice.len()
}

pub fn run() -> (usize, usize) {
    (pinned(), through_a_sized_parameter())
}
