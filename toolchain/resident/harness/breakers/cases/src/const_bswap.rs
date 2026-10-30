//! bitvec computes masks in constants with `swap_bytes` and `reverse_bits`. The function that
//! uses such a constant is compiled only if the constant evaluates, and the evaluator had
//! neither `bswap` nor `bitreverse`.

const SWAPPED: u32 = 0x1234_5678u32.swap_bytes();
const WIDE: u128 = 0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10u128.swap_bytes();
const REVERSED: u16 = 0b1011_0000_0000_0001u16.reverse_bits();
const BIG_ENDIAN: [u8; 4] = 0xdead_beefu32.to_be_bytes();

pub fn run() -> (u32, u128, u16, [u8; 4]) {
    (SWAPPED, WIDE, REVERSED, BIG_ENDIAN)
}
