// Found by fuzz.py: seed L7-81, case c9, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 124 builds. Again: fuzz.py check differs-checksum-L7-81-c9.rs
// rustc:
// c9 44855dc8d6faa550 ok
// ra-jit:
// c9 a7aee10d707b47d4 ok
mod c9 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub enum E1 {
        V0,
        V1,
        V2,
        V3(i128),
        V4,
    }
    impl Mix for E1 {
        fn mix(&self) {
            match self {
                E1::V0 => { feed(10); }
                E1::V1 => { feed(11); }
                E1::V2 => { feed(12); }
                E1::V3(a0) => { feed(13); a0.mix(); }
                E1::V4 => { feed(14); }
            }
        }
    }
    impl Drop for E1 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S2<A, B> { pub f0: Box<A>, pub f1: u128, pub f2: u128, pub f3: (B, u8), pub f4: u64, pub f5: u128, pub f6: E1 }
    impl<A: Mix, B: Mix> Mix for S2<A, B> {
        fn mix(&self) {
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy, Eq, Ord, Hash)]
    #[repr(u8)]
    pub enum E3 {
        V0,
    }
    static K4: u16 = (std::mem::size_of::<i128>() as u16);
    #[inline(always)]
    pub extern "C" fn f6(a7: i32) -> u16 {
        (if let Some(x8) = (pick::<Option<S2<u64, u128>>>(Ok::<bool, S2<u64, u128>>(true).unwrap_or_else(|x10: S2<u64, u128>| -> bool { false }), &Some(S2 { f0: Box::new(0u64), f1: 40u128, f2: 30u128, f3: (237517611374771880156353934598602797001u128, 16u8), f4: 2388312918144402777u64, f5: 340282366920938463463374607431768211455u128, f6: E1::V4 }), &(if (false) { None::<S2<u64, u128>> } else { Some(S2 { f0: Box::new(4354384325376832629u64), f1: 2199023255552u128, f2: 9u128, f3: (1u128, 166u8), f4: 16777216u64, f5: 131072u128, f6: E1::V0 }) })).clone()) { x8 } else { S2 { f5: (340282366920938463463374607431768211455u128.checked_ilog2().unwrap_or(0) as u128), ..S2 { f0: Box::new(9518194399964379203u64), f1: 1048576u128, f2: 122240661882119398087397125817639843482u128, f3: (209951347681927080723861685295269106870u128, 0u8), f4: 4u64, f5: 8u128, f6: E1::V1 } } }).mix();
        let v13: i32 = 17i32;
        K4.to_le().reverse_bits()
    }
    pub fn run() {
            match (E3::V0) {
                E3::V0 => {
                        f6(6i32).mix();
                }
            }
    }
}
