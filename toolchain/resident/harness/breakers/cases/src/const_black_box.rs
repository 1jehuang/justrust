//! crypto-bigint hides its constant-time choices from the optimizer with `black_box`, in
//! `const fn`s that p256 and p384 evaluate for their field constants.

use std::hint::black_box;

const fn select(a: u64, b: u64, choice: u64) -> u64 {
    let mask = black_box(choice).wrapping_neg();
    a ^ (mask & (a ^ b))
}

const MODULUS: [u64; 2] = [select(3, 5, 0), select(3, 5, 1)];

pub fn run() -> (u64, u64, u64) {
    const HIDDEN: u64 = black_box(7) + 1;
    (MODULUS[0], MODULUS[1], HIDDEN)
}
