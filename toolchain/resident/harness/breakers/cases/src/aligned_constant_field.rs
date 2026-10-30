//! A reference a constant holds to a field of an over-aligned value is as aligned as the
//! value, since the field is at its start. ra-jit did not keep the alignment of the memory a
//! constant points to and aligned all of it to 16, which is less than was asked for here.

#[repr(align(64))]
struct Wide([u8; 4]);

#[repr(align(8))]
struct Word([u8; 4]);

const WIDE: &[u8; 4] = &Wide([1, 2, 3, 4]).0;
const WORD: &[u8; 4] = &Word([5, 6, 7, 8]).0;
static KEPT: &[u8] = &Wide([9, 9, 9, 9]).0;
const MORE: [&[u8; 4]; 3] = [&Wide([1, 1, 1, 1]).0, &Wide([2, 2, 2, 2]).0, &Wide([3, 3, 3, 3]).0];

#[inline(never)]
fn offsets() -> (usize, usize, usize, u8, [usize; 3]) {
    let wide: *const u8 = WIDE.as_ptr();
    (wide.addr() % 64, WORD.as_ptr().addr() % 8, KEPT.as_ptr().addr() % 64, WIDE[3] + WORD[0], MORE.map(|it| it.as_ptr().addr() % 64))
}

pub fn run() -> (usize, usize, usize, u8, [usize; 3]) {
    offsets()
}
