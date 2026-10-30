//! Found by the fuzzer (`left: backend panic: StructArgument size is not properly aligned`):
//! an `extern "C"` function whose struct argument no longer fits in registers, so that it is
//! passed on the stack, where the struct's size is not a multiple of eight. The stack slot is
//! a multiple of eight; ra-jit asked Cranelift for one of the struct's size, Cranelift refused
//! it, and the function was left to rustc.

use std::hint::black_box;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Odd {
    a: u32,
    b: u8,
    c: u16,
    d: u32,
}

#[inline(never)]
pub extern "C" fn spill(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, first: Odd, g: u8, second: Odd) -> u64 {
    a + b + c + d + e + f + first.a as u64 + first.b as u64 + first.c as u64 + first.d as u64
        + (g as u64) * 1000
        + (second.a as u64 + second.b as u64 + second.c as u64 + second.d as u64) * 1_000_000
}

pub fn run() -> u64 {
    let first = black_box(Odd { a: 1, b: 2, c: 3, d: 4 });
    let second = black_box(Odd { a: 5, b: 6, c: 7, d: 8 });
    spill(1, 2, 3, 4, 5, 6, first, 7, second)
}
