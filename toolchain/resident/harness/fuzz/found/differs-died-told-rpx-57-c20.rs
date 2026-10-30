// Found by fuzz.py: seed told-rpx-57, case c20, edition 2021, backend llvm.
// differs: died
// Reduced in 72 builds. Again: fuzz.py check differs-died-told-rpx-57-c20.rs
// ra-jit's program ended with status -11 in this case
// 
mod c20 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord, Hash)]
    pub struct S1(pub Vec<u128>);
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub struct S2(pub S1, pub u128, pub f64);
    #[derive(Clone, Debug, PartialEq, PartialOrd)]
    pub enum E3 {
        V0 { f0: S2 },
        V1(f32),
        V2 { f0: Vec<u128> },
        V3,
        V4(bool),
    }
    #[inline(never)]
    pub fn f63(a64: u128, a65: f64, a66: bool, a67: Vec<u128>, a68: u128, a69: S1) -> f64 {
        apply_dyn(&|x70: S2| -> f64 { (-a65) }, ('b71: { if (a66) { break 'b71 S2(S1(vec![340282366920938463463374607431768211455u128, 18u128]), 4294967296u128, (-147.0123f64)); } S2(S1(vec![93784781297939414414033719865179607031u128, 4u128]), 9u128, 0.5f64) }))
    }
    pub fn run() {
        let mut v112: [i128; 1] = [(16i128 as i128)];
        let mut v126: u128 = (340282366920938463463374607431768211455u128.rotate_right(21u32).count_ones() as u128);
            let mut n134 = 0u32;
            'l135: while n134 < 5 && ((false || true)) {
                n134 += 1;
                if let [p140, p139 @ ..] = (Vec::<f64>::new().into_iter().map(|x137: f64| -> Option<f32> { Some(1.0f32) }).map(|x138: Option<f32>| -> Vec<String> { vec![String::from("hello"), String::from("a")] }).collect::<Vec<Vec<String>>>().as_slice()) {
                    for x141 in v112.iter_mut() {
                        *x141 = (*x141).wrapping_add((*x141).wrapping_mul(10i128));
                    }
                    v126 &= ((p139.len() as u128) ^ 170141183460469231731687303715884105727u128.wrapping_sub(v126));
                }
            }
            let mut n142 = 0u32;
            let v144: [i128; 1] = 'l143: loop {
                n142 += 1;
                if n142 > 2 { break (unsafe { *(&v112 as *const [i128; 1]) }); }
                let r151 = catch_unwind(AssertUnwindSafe(|| -> bool {
                    let g152: fn(u128, f64, bool, Vec<u128>, u128, S1) -> f64 = (|a153: u128, a154: f64, a155: bool, a156: Vec<u128>, a157: u128, a158: S1| -> f64 { (-S2(S1(Vec::<u128>::new()), 18u128, 1.0f64).2) });
                    let mut v159: [f64; 8] = [290.8967f64, g152(v126, (-9223372036854775808.0f64), false, vec![32u128, 1u128, 1u128], 25u128, S1(vec![322011603056950191716904445185676322575u128])), (-(-125.3070f64)), 1e30f64, Ok::<f64, bool>(53.0196f64).unwrap_or_else(|x163: bool| -> f64 { f64::EPSILON }), S2(S1(vec![1048575u128, 35u128, 34u128]), 170141183460469231731687303715884105727u128, 1e10f64).2, S2(S1(vec![281474976710657u128]), 35u128, 4294967296.0f64).2, (229.2446f64 - (-679.6147f64))];
                    apply_dyn(&|x170: S1| -> String { String::from("a") }, id::<S1>(S1(vec![11u128, 28u128]))).ends_with("x y")
                }));
            };
    }
}
