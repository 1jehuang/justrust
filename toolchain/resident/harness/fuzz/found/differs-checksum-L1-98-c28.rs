// Found by fuzz.py: seed L1-98, case c28, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 55 builds. Again: fuzz.py check differs-checksum-L1-98-c28.rs
// rustc:
// c28 247796217305c030 ok
// ra-jit:
// c28 b4c79ddafdc38b21 ok
mod c28 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S1 { pub f0: bool, pub f1: bool }
    impl Drop for S1 {
        fn drop(&mut self) {
            feed(0xa084dddc);
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S2(pub S1, pub i128);
    pub fn run() {
        if let [p409 @ .., S1 { f1: true, .. }] = (({ let mut items = vec![S1 { f0: true, f1: true }, S1 { f0: true, f1: false }, S1 { f0: false, f1: true }]; items.sort(); items }).as_slice()) {
            match (vec![S2(S1 { f0: false, f1: true }, 100i128)].as_slice()) {
                _ => (64i32 as f64).mix(),
            }
        }
    }
}
