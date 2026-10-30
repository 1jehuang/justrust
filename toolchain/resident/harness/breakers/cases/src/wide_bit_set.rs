//! A set of 128 members in a `u128`, tested with `1 << n`: unicode-ident's ASCII tables.

const LETTERS: u128 = 0x7fffffe07fffffe0000000000000000;
const SIGNED: i128 = -0x7fffffe07fffffe0000000000000001;

#[inline(never)]
fn member(ch: char) -> bool {
    ch.is_ascii() && LETTERS & (1 << ch as u128) != 0
}

#[inline(never)]
fn bit(at: u32) -> u128 {
    1 << at
}

#[inline(never)]
fn signed(at: u8) -> (i128, i128, bool) {
    (-1 << at, SIGNED >> at, SIGNED & (1 << at as i128) != 0)
}

pub fn run() -> Vec<String> {
    let mut out: Vec<String> = "aZ0_~\u{7f}\u{0}@[".chars().map(|ch| member(ch).to_string()).collect();
    out.extend([0, 1, 63, 64, 65, 127].map(|at| format!("{:x}", bit(at))));
    out.extend([0, 7, 64, 100, 127].map(|at| format!("{:?}", signed(at))));
    out
}
