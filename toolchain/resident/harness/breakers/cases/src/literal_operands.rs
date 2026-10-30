//! A literal operand of an operator is an immediate rather than a constant that is given an
//! address and read back. Its bits are the type's, whatever its width or sign, an overflow
//! against it still panics where rustc's does, and `line!()` and `column!()` are literals
//! whose value is not the one rust-analyzer expanded them to.

use std::hint::black_box;
use std::panic::{AssertUnwindSafe, catch_unwind};

const WIDE: u128 = 0xffff_ffff_ffff_ffff_0000_0000_0000_0001;
const LOW: i8 = i8::MIN;

fn caught<T: std::fmt::Debug>(run: impl FnOnce() -> T) -> String {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(value) => format!("{value:?}"),
        Err(payload) => match payload.downcast_ref::<&str>() {
            Some(message) => format!("panicked: {message}"),
            None => "panicked".to_owned(),
        },
    }
}

#[inline(never)]
fn narrow(value: u8, signed: i8) -> (u8, i8, u8, i8, bool) {
    (value + 200, signed - 100, value * 3, signed / -1, value > 250)
}

#[inline(never)]
fn wide(value: u128, signed: i128) -> (u128, i128, u128, bool) {
    (
        value + 0xffff_ffff_ffff_ffff_ffff_ffff_ffff_fff0,
        signed - 170141183460469231731687303715884105727,
        value & WIDE,
        signed < -170141183460469231731687303715884105727,
    )
}

#[inline(never)]
fn mixed(value: u64, signed: i64, by: u32) -> (u64, i64, u64, u64, i64, i64) {
    (
        value << 63,
        signed >> 1u8,
        1u64 << by,
        18446744073709551615 - value,
        -9223372036854775807 - signed,
        signed % -1,
    )
}

#[inline(never)]
fn others(flag: bool, letter: char, value: i32) -> (bool, bool, bool, bool, i32, i32) {
    (flag & true, flag | false, letter == '\u{10ffff}', letter < 'é', value ^ -1, -2147483647 - value)
}

#[inline(never)]
fn named(value: i8, wide: u128) -> (i8, u128) {
    (value + LOW, wide - WIDE)
}

#[inline(never)]
fn where_am_i(add: u32) -> (u32, u32, bool) {
    (line!() + add, column!() + add, line!() > 60)
}

pub fn run() -> Vec<String> {
    vec![
        caught(|| narrow(black_box(55), black_box(-28))),
        caught(|| narrow(black_box(56), black_box(-28))),
        caught(|| narrow(black_box(55), black_box(-29))),
        caught(|| narrow(black_box(1), black_box(i8::MIN))),
        caught(|| narrow(black_box(86), black_box(0))),
        caught(|| wide(black_box(15), black_box(-1))),
        caught(|| wide(black_box(16), black_box(-1))),
        caught(|| wide(black_box(1), black_box(-2))),
        caught(|| wide(black_box(WIDE), black_box(i128::MIN + 1 + i128::MAX))),
        caught(|| mixed(black_box(3), black_box(-5), black_box(63))),
        caught(|| mixed(black_box(3), black_box(-5), black_box(64))),
        caught(|| mixed(black_box(u64::MAX), black_box(1), black_box(0))),
        caught(|| mixed(black_box(0), black_box(2), black_box(0))),
        caught(|| mixed(black_box(0), black_box(i64::MIN), black_box(0))),
        caught(|| others(black_box(true), black_box('\u{10ffff}'), black_box(1))),
        caught(|| others(black_box(false), black_box('a'), black_box(2))),
        caught(|| named(black_box(127), black_box(WIDE))),
        caught(|| named(black_box(-1), black_box(WIDE))),
        caught(|| named(black_box(0), black_box(0))),
        caught(|| where_am_i(black_box(1))),
        caught(|| where_am_i(black_box(u32::MAX))),
    ]
}
