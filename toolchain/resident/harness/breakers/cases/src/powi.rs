//! `f32::powi` and `f64::powi`: an integer power, which compiler-builtins multiplies out.

#[inline(never)]
fn single(base: f32, power: i32) -> f32 {
    base.powi(power)
}

#[inline(never)]
fn double(base: f64, power: i32) -> f64 {
    base.powi(power)
}

pub fn run() -> Vec<String> {
    let mut out = Vec::new();
    for power in [-7, -1, 0, 1, 2, 3, 10, 31] {
        out.push(format!("{:e} {:e}", single(1.7, power), double(-0.93, power)));
    }
    out.push(format!("{} {}", single(0.0, -1), double(f64::NAN, 0)));
    out
}
