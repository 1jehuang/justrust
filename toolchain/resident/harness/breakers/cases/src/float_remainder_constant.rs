//! tests/ui/numbers-arithmetic/apfloat-modulo-wrong.rs: `%` of two floats in a constant.
//! ra-jit's evaluator had no float remainder and left the function that reads the constant
//! to rustc.

const SINGLE: f32 = 7.5 % 2.0;
const DOUBLE: f64 = -7.5 % 2.0;
const SMALL: f64 = 1e-320 % 3e-321;

#[inline(never)]
fn remainders(x: f64, y: f64) -> f64 {
    x % y
}

pub fn run() -> (f32, f64, f64, bool) {
    (SINGLE, DOUBLE, SMALL, remainders(1e-320, 3e-321) == SMALL)
}
