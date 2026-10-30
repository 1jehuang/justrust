// Found by fuzz.py: seed L12-18, case c0, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 50 builds. Again: fuzz.py check differs-checksum-L12-18-c0.rs
// rustc:
// c0 719b59e3f2024a55 ok
// ra-jit:
// c0 bfa7e647f0f8a0d0 ok
mod c0 {
    use super::*;
    pub fn run() {
            enum Loud { I(i32, Noisy), C(char, Noisy) }
            #[inline(never)] fn items(k: u32, last: char) -> [Loud; 3] { black_box([Loud::I(k as i32 % 4, Noisy(k)), Loud::C('q', Noisy(k + 1)), if k % 3 == 0 { Loud::I(1, Noisy(k + 2)) } else { Loud::C(last, Noisy(k + 2)) }]) }
            #[inline(never)] fn pick0(k: u32, last: char, flag: bool) -> u8 { match items(k, last) { [Loud::C('A'..='Z', _), ..] => 1, [Loud::C('b', _) | Loud::I(..3, _), ..] => 2, [Loud::C('A'..='Z', _), m @ .., _] => 3, [Loud::I(..), ..] if black_box(false) => 4, _ => 0 } }
            for k in 0..4u32 { for last in ['a', 'Z', 'q'] { for flag in [false, true] { feed(pick0(k, last, flag) as u64); feed(0xf00d); } } }
    }
}
