// Found by fuzz.py: seed L2-463, case c8, edition 2024, backend cranelift.
// differs: checksum
// Reduced in 47 builds. Again: fuzz.py check differs-checksum-L2-463-c8.rs
// rustc:
// c8 12db8949a6af33cf ok
// ra-jit:
// c8 43a2a2649d437d7b ok
mod c8 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S1 { pub f0: i16, pub f1: isize, pub f2: i16 }
    impl Mix for S1 {
        fn mix(&self) {
            self.f0.mix();
        }
    }
    impl Drop for S1 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    pub fn run() {
        if let [_, p125, S1 { f0: p126 @ -9i16..7i16, f1: _, f2: p127 }, S1 { f0: p128, f1: p129, .. }] = (({ let mut items = [S1 { f0: (-3i16), f1: 7isize, f2: (-8882i16) }, S1 { f0: 6i16, f1: 9223372036854775807isize, f2: (-32767i16) }, S1 { f0: (-17589i16), f1: 1isize, f2: 0i16 }, S1 { f0: 512i16, f1: 4294967297isize, f2: 12i16 }]; items.rotate_left(1); items })) {
        }
    }
}
