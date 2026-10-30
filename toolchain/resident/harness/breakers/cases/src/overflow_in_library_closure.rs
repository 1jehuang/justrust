//! tests/ui/iterators/iter-sum-overflow-debug.rs: `Iterator::sum` adds in a closure of `core`
//! marked `#[rustc_inherit_overflow_checks]`, which checks for overflow when the crate that
//! calls `sum` does. ra-jit read the attribute on functions only, and the sum wrapped.

use std::panic::catch_unwind;

#[inline(never)]
fn sum(values: &[u8]) -> Option<u8> {
    catch_unwind(|| values.iter().copied().sum::<u8>()).ok()
}

#[inline(never)]
fn product(values: &[i8]) -> Option<i8> {
    catch_unwind(|| values.iter().product::<i8>()).ok()
}

pub fn run() -> (Option<u8>, Option<u8>, Option<i8>, Option<i8>) {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| ()));
    let sums = (sum(&[100, 100]), sum(&[200, 100]), product(&[-8, 16]), product(&[-8, 17]));
    std::panic::set_hook(hook);
    sums
}
