// Found by fuzz.py: seed s2-3, case c10, edition 2021, backend cranelift.
// differs: panic location
// Reduced in 78 builds. Again: fuzz.py check differs-panic-location-s2-3-c10.rs
// rustc:
// panic: panicked at src/main.rs:342:39:
// index out of bounds: the len is 3 but the index is 16
// c10 0000000000000000 ok
// ra-jit:
// panic: panicked at <ra-jit>:0:0:
// index out of bounds: the len is 3 but the index is 16
// c10 0000000000000000 ok
mod c10 {
    use super::*;
    pub fn run() {
        let mut v272: i16 = (match (Some([[1048577i32; 2], [16i32, 8i32]]).map(|x275: [[i32; 2]; 2]| -> i16 { 2i16 })) { None => 9i16.wrapping_div(32767i16 | 1), Some(x274) => x274 }).max(((!(-18i16)) << (2147483649u32.wrapping_sub(1u32) % 16)));
        let r309 = catch_unwind(AssertUnwindSafe(|| -> i16 {
            let v311: bool = ((-1602784331i32).is_negative() && matches!([0.0f64, 0.5f64, 1.5f64], ([.., _])));
            assert!(!v311, "held {}", [1048577u32, 3624379010u32, 2u32][16usize]);
            v272
        }));
    }
}
