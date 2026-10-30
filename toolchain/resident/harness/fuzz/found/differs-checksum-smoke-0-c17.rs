// Found by fuzz.py: seed smoke-0, case c17, edition 2024, backend cranelift.
// differs: checksum
// Reduced in 78 builds. Again: fuzz.py check differs-checksum-smoke-0-c17.rs
// rustc:
// c17 be9335fd72ba3538 ok
// rustd:
// c17 b9d7455c33868e00 ok
mod c17 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(C)]
    pub struct S1 { pub f0: u32, pub f1: char, pub f2: bool }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S2(pub u32, pub S1);
    impl Drop for S2 {
        fn drop(&mut self) {
            feed(0x576b5bea);
        }
    }
    const K4: S1 = S1 { f0: 0u32.saturating_sub(0u32), f1: '9', f2: true };
    pub fn run() {
        let mut v9: S1 = S1 { f2: false, ..K4 };
        let mut v36: S2 = S2(v9.f0, S1 { f0: 4294967295u32, f1: 'a', f2: false });
            let mut f37 = || -> char { v36 = (if (false) { S2(51110277u32, S1 { f0: 3783521898u32, f1: 'Z', f2: false }) } else { Some(S2(587432115u32, S1 { f0: 36u32, f1: '\0', f2: true })).map_or(v36.clone(), |x38: S2| -> S2 { x38.clone() }) }); '\u{7f}' };
            f37().mix();
    }
}
