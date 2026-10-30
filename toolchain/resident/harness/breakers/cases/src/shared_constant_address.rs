//! library/alloctests str.rs, `const_str_ptr`: a pointer a constant holds to another
//! constant's value and a pointer the function makes to that value are equal under rustc,
//! which keeps one copy of equal read-only bytes. ra-jit did not know the alignment the
//! constant's memory was made with, gave it the most a value could need, and so did not take
//! it for the same bytes at the alignment of their type.

const PAIR: [u8; 2] = [b'h', b'i'];
const HELD: &[u8; 2] = &PAIR;
const FIRST: *const u8 = HELD as *const u8;

#[inline(never)]
fn same() -> (bool, u8) {
    let here = &PAIR as *const u8;
    (here == FIRST, unsafe { *FIRST })
}

pub fn run() -> (bool, u8) {
    same()
}
