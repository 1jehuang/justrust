// Found by fuzz.py: seed told-rpa-92, case c0, edition 2021, backend llvm.
// differs: died
// Reduced in 61 builds. Again: fuzz.py check differs-died-told-rpa-92-c0.rs
// ra-jit's program ended with status -11 in this case
// 
mod c0 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy)]
    #[repr(C)]
    pub struct S1 { pub f0: f64, pub f1: f64, pub f2: f64, pub f3: f64, pub f4: f64, pub f5: f64 }
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy)]
    pub enum E2 {
        V0,
        V1(f64, S1, f64),
    }
    static mut K3: i128 = (14i128.leading_zeros() as i128);
    static mut K4: f64 = 65535.0f64;
    static K5: E2 = E2::V0;
    pub fn f18(a19: &i8, a20: f64, a21: E2, a22: &mut [char], a23: &[f64], a24: E2, a25: i128, a26: S1, a27: f64, a28: f64) -> char {
            let v31: f64 = sel::<f64>(false, a26.f2, a26.f4);
        (match (&{ (unsafe { K3 }) }) { ..3i128 => 'a', _ => 'Z' })
    }
    pub fn run() {
        let mut v90: f64 = ({ let t91: E2 = K5; (unsafe { K4 }) }).mul_add((f32::NAN as f64), ((unsafe { K4 }) - 255.5f64));
        let (_, S1 { f0: p92, f1: p93, f2: p94, f4: _, f5: p95, .. }, p96, ()): (char, S1, char, ()) = ('\0', S1 { f0: 1.0f64, f1: (-2.5e-3f64), f2: (-128.9f64), f3: (-390.2265f64), f4: (-9223372036854775808.0f64), f5: 0.0f64 }, ' ', ());
        match (vec![(-1i128), 1i128, 15i128, 10i128, (-11982103479036087258856139444494326914i128)].as_slice()) {
            [p98 @ -119045969441908332073115657724443162291i128..100i128, ..] => {
                let mut v99: [char; 2] = <[char; 2]>::try_from((|x104: bool| -> Vec<char> { Vec::<char>::new() })(false)).unwrap_or(<[char; 2]>::try_from(vec!['中', 'é', '\n']).unwrap_or(['_', 'ß']));
                { let value = ({ let mut m107: Vec<char> = [S1 { f0: 1e10f64, f1: 0.5f64, f2: 0.5f64, f3: 0.5f64, f4: f64::MAX, f5: 688.9011f64 }, S1 { f0: (-1.0f64), f1: 3.25f64, f2: 29.5539f64, f3: f64::NAN, f4: f64::MIN, f5: f64::MIN_POSITIVE }].into_iter().take(3).map(|x110: S1| -> char { '\u{10FFFF}' }).collect::<Vec<char>>(); let out = f18(&(unsafe { *(&126i8 as *const i8) }), (-9223372036854775808.0f64), E2::V0, &mut m107, &sel::<Vec<f64>>(false, vec![65535.0f64, 30.3279f64], vec![f64::NAN, f64::NEG_INFINITY]), E2::V1(0.2f64, S1 { f0: f64::EPSILON, f1: f64::MAX, f2: (-9223372036854775808.0f64), f3: 3.25f64, f4: f64::MIN, f5: (-9223372036854775808.0f64) }, p92), (unsafe { K3 }), S1 { f0: 1e10f64, f1: (unsafe { K4 }), f2: p92, f3: p95, f4: f64::MAX, f5: p92 }, (v90 % 1e-40f64), p93.fract()); m107.mix(); out }); let slot = &mut v99[1] as *mut char; unsafe { *slot = value; (*slot).mix(); } }
            }
            _ => apply(|x153: char| -> f64 { p92 }, p96).mix(),
        }
    }
}
