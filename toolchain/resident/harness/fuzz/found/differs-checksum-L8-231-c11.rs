// Found by fuzz.py: seed L8-231, case c11, edition 2021, backend cranelift.
// differs: checksum
// Reduced in 103 builds. Again: fuzz.py check differs-checksum-L8-231-c11.rs
// rustc:
// panic: panicked at src/main.rs:364:695:
// attempt to divide by zero
// c11 d88a79ed2d66c13e ok
// ra-jit:
// panic: panicked at src/main.rs:364:695:
// attempt to divide by zero
// c11 8a400fca52e41ce1 ok
mod c11 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub enum E1 {
        V0(i128, u128),
        V1(&'static str, i128),
        V2 { f0: i128 },
    }
    impl Mix for E1 {
        fn mix(&self) {
            match self {
                E1::V0(a0, a1) => { feed(10); a0.mix(); a1.mix(); }
                E1::V1(a0, a1) => { feed(11); a0.mix(); a1.mix(); }
                E1::V2 { f0: a0 } => { feed(12); a0.mix(); }
            }
        }
    }
    impl Drop for E1 {
        fn drop(&mut self) {
            self.mix();
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S2<A>(pub i128, pub i128, pub E1, pub Box<A>, pub E1, pub i128);
    pub fn run() {
            match (&{ Ok::<(), E1>(()) }) {
                Ok(()) => {
                    let r60 = catch_unwind(AssertUnwindSafe(|| -> S2<E1> {
                        S2((154564069092477551457654274860279453034i128 ^ (1i128 >> 10u32)), (-4i128).to_be().saturating_sub((-111569104845928584268462650633725978427i128)), pick::<E1>(17179869183i128.is_negative(), &Some(E1::V2 { f0: 105716125101924610038706223115468787968i128 }).unwrap(), &E1::V2 { f0: 2596148429267413814265248164610048i128 }).clone(), Box::new(E1::V1("  pad  ", 5192296858534827628530496329220096i128)), pick::<E1>((false || false), &larger::<E1>(E1::V2 { f0: 10i128 }, E1::V2 { f0: (-5i128) }), &E1::V2 { f0: 3762956535947026073837279534277290269i128 }).clone(), (13604195070865570274568468084988530956i128.saturating_add((-63074316055547349760354514066546187595i128)) & (1i128 / 0i128)))
                    }));
                }
                _ => {}
            }
    }
}
