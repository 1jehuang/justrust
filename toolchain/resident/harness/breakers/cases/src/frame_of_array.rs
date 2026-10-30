//! stacker grows the stack when less than 100k of it is left, and its test recurses through a
//! function that keeps an array of 50k: the frame of that function has to be less than twice
//! the array. cpp_demangle's parser recurses to a depth that fits rustc's frames in 2MB.

use std::hint::black_box;

#[inline(never)]
fn recurse(depth: usize, above: usize) -> usize {
    let array = [42u8; 50000];
    let here = black_box(array.as_ptr() as usize);
    let frame = if depth == 0 { above - here } else { recurse(depth - 1, here) };
    drop(array);
    frame
}

#[inline(never)]
fn through_closure(depth: usize, above: usize) -> usize {
    let array = [42u8; 50000];
    let here = black_box(array.as_ptr() as usize);
    let frame = if depth == 0 {
        above - here
    } else {
        let next = || through_closure(depth - 1, here);
        next()
    };
    drop(array);
    frame
}

#[inline(never)]
fn make(byte: u8) -> [u8; 50000] {
    [byte; 50000]
}

#[inline(never)]
fn of_a_call(depth: usize, above: usize) -> usize {
    let array = make(42);
    let here = black_box(array.as_ptr() as usize);
    let frame = if depth == 0 { above - here } else { of_a_call(depth - 1, here) };
    black_box(&array);
    frame
}

/// Whether as few arrays of 50k fit in one frame of `recurse`, in one of `through_closure` with
/// the frame of its closure, and in one of `of_a_call` as in rustc's, 2, 2 and 1: a smaller
/// frame fits too. curve25519-dalek's tests keep five tables that calls return, of 950k
/// together, on a stack of 2MB.
pub fn run() -> (bool, bool, bool) {
    let arrays = |frame: usize| frame / 50000;
    (arrays(recurse(1, 0)) <= 2, arrays(through_closure(1, 0)) <= 2, arrays(of_a_call(1, 0)) <= 1)
}
