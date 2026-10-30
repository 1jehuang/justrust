//! unicode-ident tests a bit of a `u128` with `1 << ch as u128`: a shift whose amount is as
//! wide as the value. The backend had no answer for an amount of 128 bits.

use std::hint::black_box;

fn bit(mask: u128, at: char) -> bool {
    mask & (1 << at as u128) != 0
}

fn shifts(value: u128, by: u128) -> (u128, u128, i128) {
    (value << by, value >> by, (value as i128) >> by)
}

fn narrow_by_wide(value: u8, by: u128) -> (u8, u8) {
    (value << by, value >> by)
}

pub fn run() -> (bool, bool, (u128, u128, i128), (u8, u8)) {
    let mask = black_box(0x07ff_fffe_07ff_fffe_0000_0000_0000_0000u128);
    (
        bit(mask, black_box('a')),
        bit(mask, black_box('1')),
        shifts(black_box(u128::MAX - 12345), black_box(77)),
        narrow_by_wide(black_box(0xb5), black_box(3)),
    )
}
