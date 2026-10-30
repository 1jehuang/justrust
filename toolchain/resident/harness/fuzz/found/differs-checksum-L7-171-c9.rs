// Found by fuzz.py: seed L7-171, case c9, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 86 builds. Again: fuzz.py check differs-checksum-L7-171-c9.rs
// rustc:
// c9 22ef2b5ee367935f ok
// ra-jit:
// c9 e7594b007c04805d ok
mod c9 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub struct S2 { pub f0: f64, pub f1: &'static str, pub f2: NonZeroU8, pub f3: &'static str, pub f4: f32, pub f5: &'static str, pub f6: bool }
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
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy)]
    pub enum E3<A> {
        V0(f64),
        V1 { f0: () },
        V2(i128),
        V3,
        V4(A, i128, f64),
    }
    const K5: bool = true;
    const K6: f32 = 16777217.0f32;
    pub fn run() {
        let mut v51: bool = K5;
            let mut f52 = |a49: E3<char>, a50: char| -> &'static str { v51 = v51; Err::<S2, f64>(65535.0f64).unwrap_or(S2 { f0: (-799.2402f64), f1: "3.5", f2: NonZeroU8::new(8u8).unwrap(), f3: "😀", f4: 0.1f32, f5: "42", f6: false }).f3 };
            let S2 { f0: p59, f2: p60, f3: p61, f4: p62, .. }: S2 = (if let Some(x57) = (vec![S2 { f0: 938.4704f64, f1: "42", f2: NonZeroU8::new(7u8).unwrap(), f3: "ff", f4: 4294967296.0f32, f5: "x y", f6: false }, S2 { f0: 0.2f64, f1: "line\n", f2: NonZeroU8::new(255u8).unwrap(), f3: "", f4: (-493.5720f32), f5: "", f6: false }].last().cloned()) { x57 } else { S2 { f0: (1e-40f64 - 65535.0f64), f1: f52(E3::<char>::V3, '\u{7f}'), f2: NonZeroU8::new(16u8).unwrap_or(NonZeroU8::new(128u8).unwrap()), f3: "3.5", f4: K6, f5: f52(E3::<char>::V2(2305843009213693953i128), '\u{10FFFF}'), f6: (-128.9f64).is_infinite() } });
    }
}
