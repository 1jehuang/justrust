// Found by fuzz.py: seed L11-133, case c4, edition 2024, backend cranelift.
// differs: checksum
// Reduced in 53 builds. Again: fuzz.py check differs-checksum-L11-133-c4.rs
// rustc:
// panic: panicked at src/main.rs:528:1112:
// index out of bounds: the len is 3 but the index is 33
// panic: panicked at src/main.rs:528:1112:
// index out of bounds: the len is 3 but the index is 33
// c4 b561a37a42c21f7b ok
// ra-jit:
// panic: panicked at src/main.rs:528:1112:
// index out of bounds: the len is 3 but the index is 33
// panic: panicked at src/main.rs:528:1112:
// index out of bounds: the len is 3 but the index is 33
// c4 3c83f002014e1c3c ok
mod c4 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S1 { pub f0: i16 }
    impl Mix for S1 {
        fn mix(&self) {
            feed(1);
            self.f0.mix();
        }
    }
    impl Drop for S1 {
        fn drop(&mut self) {
            feed(0x5c10901);
            self.mix();
        }
    }
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub struct S2 { pub f0: f32, pub f1: i16 }
    impl Mix for S2 {
        fn mix(&self) {
        }
    }
    impl Drop for S2 {
        fn drop(&mut self) {
            feed(0x11d580ca);
        }
    }
    #[inline(never)]
    pub fn f47<G45: Mix + Clone + Debug + PartialOrd, G46: Mix + Clone + Debug + PartialOrd>(a48: G45, a49: G46, a50: G46, a51: Box<G45>, a52: &G46, a53: S2) -> S2 {
        if (true) {
            if (Vec::<S2>::new().contains(&S2 { f0: f32::MAX, f1: a53.f1 })) {
                ((-21167278036084975882522383071477095785i128).wrapping_mul(107128146156324376925489535292563575936i128) | 1i128.wrapping_add(7i128)).mix();
            } else {
                let mut v55: Box<S1> = Box::new(S1 { f0: 127i16 });
            }
            'l59: for x58 in (({ let mut items = vec![127i8]; items.extend(vec![63i8, (-103i8), (-10i8), 20i8, 17i8]); items }).into_iter().take_while(|x57: &i8| -> bool { true })) {
                a50.clone().mix();
                ((-128.9f64) * 3.25f64).mix();
            }
        }
        S2 { f0: (a53.f0 % a53.f0), f1: 2i16 }
    }
    #[inline(never)]
    pub fn f143(a144: S1) -> impl Fn() -> Box<S1> {
        move || -> Box<S1> { Box::new(id::<S1>(a144.clone())) }
    }
    pub fn run() {
        let r551 = catch_unwind(AssertUnwindSafe(|| -> [i128; 2] {
            let mut n552 = 0u32;
            'l553: while n552 < 2 && ((!false)) {
                n552 += 1;
                let r556 = catch_unwind(AssertUnwindSafe(|| -> i32 {
                    let [.., S2 { f0: _, .. }]: [S2; 2] = [f47::<S2, Box<S1>>(Some(S2 { f0: f32::NEG_INFINITY, f1: 100i16 }).expect("nothing there"), (match (&{ S1 { f0: 100i16 } }) { S1 { f0: p560 } => Box::new(S1 { f0: 1i16 }), _ => Box::new(S1 { f0: 512i16 }) }), Box::new(S1 { f0: 14i16 }), (if (true) { Box::new(S2 { f0: f32::NEG_INFINITY, f1: 7i16 }) } else { Box::new(S2 { f0: (-0.0f32), f1: 8192i16 }) }), &f143(S1 { f0: 14i16 })(), S2 { f0: (-128.9f32), f1: 32767i16 }), f47::<S1, Result<S2, S2>>(id::<S1>(S1 { f0: 1i16 }), (if (false) { Ok::<S2, S2>(S2 { f0: (-949.6335f32), f1: (-12181i16) }) } else { Ok::<S2, S2>(S2 { f0: (-745.5345f32), f1: 20057i16 }) }), Ok::<S2, S2>(S2 { f0: 1e-40f32, f1: 6005i16 }), (match (&{ Ok::<Result<f64, usize>, S1>(Err::<f64, usize>(1usize)) }) { Ok(Ok(p563)) => Box::new(S1 { f0: 3i16 }), Err(S1 { f0: p564 }) => Box::new(S1 { f0: 3i16 }), _ => Box::new(S1 { f0: 11i16 }) }), &Some(S2 { f0: f32::MIN, f1: 11i16 }).ok_or(S2 { f0: 0.2f32, f1: 11925i16 }), vec![S2 { f0: f32::NEG_INFINITY, f1: 31499i16 }, S2 { f0: 1.5f32, f1: 1i16 }, S2 { f0: f32::MIN, f1: 32767i16 }][33usize].clone())];
                    (((-128.9f32) * (-2.5e-3f32)) as i32).wrapping_mul(100i32.wrapping_add(1056330224i32).checked_mul(20i32.wrapping_sub(796687572i32)).expect("out of range"))
                }));
            }
            [(-170141183460469231731687303715884105727i128); 2]
        }));
    }
}
