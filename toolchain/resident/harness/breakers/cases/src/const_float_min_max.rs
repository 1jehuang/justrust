//! `f32::max`, `f32::min` and their `f64` twins in a constant: since Rust 1.99 they call
//! `maximum_number_nsz_f32` and its kin, intrinsics with a body in core that the evaluator
//! runs. nsh's colors are such constants (`const C: Hsla = rgb(0x2563eb).to_hsla();`), and
//! every function reading one was refused, so every edit near them was (#224).

#[derive(Debug, Clone, Copy)]
pub struct Hsl {
    h: f32,
    s: f32,
    l: f32,
}

const fn to_hsl(r: f32, g: f32, b: f32) -> Hsl {
    let max = r.max(g.max(b));
    let min = r.min(g.min(b));
    let delta = max - min;
    let l = (max + min) / 2.0;
    let s = if delta == 0.0 { 0.0 } else { delta / (1.0 - (2.0 * l - 1.0).abs()) };
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        let x = (g - b) / delta;
        (if x < 0.0 { x + 6.0 } else { x }) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    Hsl { h, s, l }
}

const ACCENT: Hsl = to_hsl(0.49, 0.67, 0.08);
const GREY: Hsl = to_hsl(0.5, 0.5, 0.5);
const WIDE: (f64, f64) = (2.5f64.max(-1.0), 2.5f64.min(f64::NAN));
const NAN_IGNORED: (f32, f32) = (f32::NAN.max(1.0), 3.0f32.min(f32::NAN));

#[inline(never)]
fn tint(dark: bool) -> Hsl {
    if dark { ACCENT } else { GREY }
}

pub fn run() -> (Hsl, Hsl, (f64, f64), (f32, f32)) {
    (tint(true), tint(false), WIDE, NAN_IGNORED)
}
