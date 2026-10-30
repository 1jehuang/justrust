// Found by fuzz.py: seed L7-87, case c6, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 53 builds. Again: fuzz.py check differs-checksum-L7-87-c6.rs
// rustc:
// c6 d209022dd5dbdc20 ok
// ra-jit:
// c6 b1a302313bdbdc3f ok
mod c6 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S2 { pub f0: u16 }
    impl Mix for S2 {
        fn mix(&self) {
            self.f0.mix();
        }
    }
    impl Drop for S2 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    pub fn run() {
        if let Ok((_, ..)) = (Ok::<(f32, u128, f64, char, &'static str, u128), f64>((306.2591f32, 23u128, 0.2f64, 'Z', "hello", 1u128))) {
            let mut v83: S2 = S2 { f0: 50112u16.clamp(0u16, 23u16) };
            if let S2 { f0: 65535u16 } = ((*Box::new(S2 { f0: 63u16 }))) {
            }
        }
    }
}
