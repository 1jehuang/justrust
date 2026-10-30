// Found by fuzz.py: seed land2-rpa-33, case c12, edition 2024, backend llvm.
// differs: died
// Reduced in 100 builds. Again: fuzz.py check differs-died-land2-rpa-33-c12.rs
// ra-jit's program ended with status -11 in this case
// 
mod c12 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy)]
    #[repr(C)]
    pub struct S1 { pub f0: f32, pub f1: bool }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(u32)]
    pub enum E2 {
        V0 = 19,
        V1 = 39,
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy)]
    #[repr(C)]
    pub struct S3 { pub f0: S1, pub f1: isize, pub f2: isize, pub f3: f32, pub f4: u8, pub f5: S1 }
    const K5: S3 = S3 { f0: S1 { f0: f32::MAX, f1: false }, f1: 9223372036854775806isize, f2: 18isize, f3: f32::NEG_INFINITY, f4: 2u8, f5: S1 { f0: 283.4851f32, f1: true } };
    static K6: char = '中';
    #[inline(never)]
    pub fn f94(a95: &'static str, a96: char, a97: u128, a98: f32, a99: &&'static str, a100: i128, a101: Option<Option<String>>, a102: E2) -> char {
        char::from_u32(u32::from((K5.f4 | K5.f4))).unwrap_or(K6)
    }
    pub fn run() {
        let S1 { f0: p131, .. }: S1 = K5.f5;
                f94("line\n", K6, 36u128, p131, &"hello", 20i128, Some(Some(String::from("hello"))), E2::V1).to_ascii_lowercase().mix();
            let mut v181: Vec<Option<Option<String>>> = vec![Some(true).and_then(|x183: bool| -> Option<Option<String>> { Some(Some(String::from("42"))) }); ((20usize.count_ones() as usize) % 6)];
    }
}
