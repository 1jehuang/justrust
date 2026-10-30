// Found by fuzz.py: seed L1-12, case c3, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 102 builds. Again: fuzz.py check differs-checksum-L1-12-c3.rs
// rustc:
// c3 c5cae62c646c06b9 ok
// ra-jit:
// c3 b3b7bed8beb10d4c ok
mod c3 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub enum E1 {
        V0,
        V1 { f0: u8, f1: u8, f2: f32 },
    }
    impl Mix for E1 {
        fn mix(&self) {
        }
    }
    impl Drop for E1 {
        fn drop(&mut self) {
            feed(0xa0e30f7c);
        }
    }
    pub fn run() {
                (if ((String::from("3.5").ends_with("  pad  ") && (false || false))) { (E1::V1 { f0: 34u8, f1: 8u8, f2: f32::MIN_POSITIVE }, Some((-797.0417f32)), None::<i64>) } else { (if ((Box::new(E1::V0) != Box::new(E1::V0))) { (E1::V0, Some(1e-40f32), Some(0i64)) } else { pick::<(E1, Option<f32>, Option<i64>)>(false, &(E1::V1 { f0: 17u8, f1: 3u8, f2: (-91.5693f32) }, Some(f32::MIN_POSITIVE), None::<i64>), &(E1::V1 { f0: 9u8, f1: 255u8, f2: 1.5f32 }, Some(1.0f32), Some(2097152i64))).clone() }) }).mix();
    }
}
