//! rand's `StandardUniform` scales by `1.0 / ((1 as $u_scalar << precision) as $f_scalar)`,
//! the types and the bit count handed to a macro.

macro_rules! scale {
    ($f_scalar:ident, $u_scalar:ty, $fraction_bits:expr) => {{
        let precision = $fraction_bits + 1;
        let shifted = 1 as $u_scalar << precision;
        let scale = 1.0 / ((1 as $u_scalar << precision) as $f_scalar);
        (precision as u64, shifted as u64, scale as f64)
    }};
}

pub fn run() -> [(u64, u64, f64); 2] {
    [scale!(f32, u32, 23), scale!(f64, u64, 52)]
}
