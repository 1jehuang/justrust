//! library/alloctests vec.rs, `test_into_iter_as_slice`, and vec_deque.rs,
//! `test_partial_eq_array`: an empty array compared with a slice or a `VecDeque` has the
//! element type that the one `PartialEq` impl for the other side's elements gives it. ra-jit
//! counted the impls of `proc_macro`, which this crate does not load, and with `impl
//! PartialEq<Punct> for char` the element type was left unknown: the functions had no layout
//! and were left to rustc.

use std::collections::VecDeque;

#[inline(never)]
fn slice_is_empty(of: &[char]) -> bool {
    of == &[]
}

#[inline(never)]
fn deque_is_empty(of: &VecDeque<char>) -> bool {
    *of == []
}

pub fn run() -> (bool, bool) {
    (slice_is_empty(&['a']), deque_is_empty(&VecDeque::new()))
}
