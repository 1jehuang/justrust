// Found by fuzz.py: seed L1-6, case c22, edition 2024, backend cranelift.
// left: backend panic: StructArgument size is not properly aligned
// Reduced in 104 builds. Again: fuzz.py check left-backend-panic-structargument-size-is-not-properly-align-L1-6-c22.rs
// fz::c22::run: backend: backend panic: StructArgument size is not properly aligned
mod c22 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub struct S1(pub u16, pub u8, pub u16, pub i8, pub u16, pub u8);
    pub extern "C" fn f90(a91: u16, a92: f64, a93: i64, a94: i32, a95: f64, a96: S1, a97: usize, a98: S1, a99: isize) -> u32 {
        25u32
    }
    pub fn run() {
            match (({ let mut items = vec![2u8, 7u8]; items.insert(0, 38u8); items }).as_slice()) {
                [] => {
                        let mut v186: BTreeMap<u32, String> = BTreeMap::new();
                        v186.insert(f90(5u16, 0.1f64, 281474976710657i64, 8388607i32, 1e-40f64, S1(34u16, 207u8, 7u16, (-82i8), 100u16, 254u8), 18446744073709551614usize, S1(100u16, 35u8, 2u16, (-84i8), 10u16, 10u8), 7isize).checked_rem(4u32).unwrap_or(8u32.checked_shr(1u32).unwrap_or(15u32)), (if ((!false)) { String::from("héllo") } else { 26280453u32.to_string() })).mix();
                }
                _ => {}
            }
    }
}
