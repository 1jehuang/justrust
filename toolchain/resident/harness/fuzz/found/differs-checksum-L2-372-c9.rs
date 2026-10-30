// Found by fuzz.py: seed L2-372, case c9, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 84 builds. Again: fuzz.py check differs-checksum-L2-372-c9.rs
// rustc:
// c9 606d53f1c3cd6678 ok
// ra-jit:
// c9 ca7633d1ee1586d1 ok
mod c9 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S1 { pub f0: i128, pub f1: bool }
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
            'l132: for x131 in ((37u16..40u16).enumerate()) {
                    let [S1 { .. }, p137 @ .., S1 { f0: 1073741824i128, f1: p138 }] = (id::<[S1; 3]>([S1 { f0: 2i128, f1: true }, S1 { f0: 9i128, f1: true }, S1 { f0: (-165389353748757961091619552348374937455i128), f1: true }])) else { break 'l132; };
            }
    }
}
