// Found by fuzz.py: seed L12-20, case c25, edition 2024, backend cranelift.
// left: intrinsic `powif32`
// Reduced in 61 builds. Again: fuzz.py check left-intrinsic-powif32-L12-20-c25.rs
// fz::c25::run: unsupported: callee `catch_unwind`: callee `catch_unwind`: callee `do_call`: callee `call_once`: callee `{closure}`: callee `powi`: callee `powi`: unsupported: intrinsic `powif32`
mod c25 {
    use super::*;
    #[derive(Clone, Debug, PartialEq, PartialOrd, Copy)]
    pub struct S2 { pub f0: f64 }
    pub fn run() {
        let mut v239: [i8; 4] = id::<[i8; 4]>(({ let mut items = [(-19i8); 4]; items.sort_by(|a, b| b.cmp(a)); items }));
        let r303 = catch_unwind(AssertUnwindSafe(|| -> i8 {
            match (&{ (if (false) { Box::new(S2 { f0: 65535.0f64 }) } else { Box::new(S2 { f0: 7.0f64 }) }) }) {
                p313 if (!true) => {
                        let (a, b) = (black_box((-2147483649.0) as f32), black_box((f64::MIN) as f32));
                        let both = [a, b, a + b, a * 2.0]; both.iter().copied().fold(0.0, |acc, it| if it.is_nan() { acc } else { acc + it }).mix(); both.iter().filter(|it| it.is_finite()).count().mix();
                        (a + b).mix(); (a - b).mix(); (a * b).mix(); (a / b).mix(); (a % b).mix(); (-a).mix();
                        if !a.is_nan() && !b.is_nan() && a != b { a.max(b).mix(); a.min(b).mix(); } a.clamp(-1.0, 1.0).mix();
                        a.sqrt().mix(); a.mul_add(b, 1.5).mix(); a.rem_euclid(3.0).mix(); a.div_euclid(3.0).mix(); a.powi(3).mix(); a.recip().mix();
                        format!("{a} {a:?} {a:e} {b:.3} {b:10.2} {a:+}").mix();
                        feed(a.total_cmp(&b) as i8 as u64); feed(a.partial_cmp(&b).map_or(9, |it| it as i8 as u8) as u64); feed((a < b) as u64 + 2 * (a <= b) as u64 + 4 * (a == b) as u64 + 8 * (a != b) as u64);
                }
                p315 => {
                }
            }
            (v239[2].checked_mul(v239[0]).unwrap_or(v239[0]) * (v239[2] * 11i8))
        }));
    }
}
