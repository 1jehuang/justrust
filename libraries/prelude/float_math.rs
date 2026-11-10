// Floating-point math methods of `f32` and `f64` as ordinary Rust source.
//
// Stock Rust forwards the transcendental methods to the platform libm, so
// the observable results on Linux x86-64 are glibc's. The routines here
// reproduce those results bit for bit:
//
// - `exp`, `ln`, `powf` (f64) and `exp`, `exp2`, `ln`, `log2`, `powf`, `sin`,
//   `cos` (f32) follow the Arm Optimized Routines algorithms and tables that
//   glibc ships, evaluated with the fused multiply-add contractions of
//   glibc's FMA build (the variant its ifunc selects on current x86-64).
// - `log2` and `exp2` (f64) follow the same routines without contraction
//   (glibc has no FMA variant of those two).
// - `log10` and `hypot` follow the classic fdlibm formulations.
//
// Adapted from third-party sources, which keep their own licenses (see
// NOTICE and THIRD-PARTY-LICENSES.md):
//
// Arm Optimized Routines v20.02 (math/), Copyright (c) 2017-2018, Arm Limited.
// SPDX-License-Identifier: MIT
//
// fdlibm (e_hypot.c, e_log10.c):
// ====================================================
// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
// Developed at SunSoft, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
// ====================================================
//
// f64 `sin`/`cos`/`tan`/`atan`/`atan2` are deliberately absent: glibc's
// versions are neither correctly rounded nor permissively licensed, so they
// stay an explicit E0599 instead of an approximately right answer.
//
// Internal helpers are free functions with names no primitive method uses.
use super::primitive::FloatBits;

// ---- bit helpers ----

fn fb(bits: u64) -> f64 {
    f64::from_bits(bits)
}

fn fbs(bits: u32) -> f32 {
    f32::from_bits(bits)
}

fn high_word(x: f64) -> u32 {
    (x.to_bits() >> 32) as u32
}

// `x + y` when at least one operand is NaN: the first NaN, quieted.
fn nan_sum(x: f64, y: f64) -> f64 {
    let v = if x.is_nan() { x } else { y };
    fb(v.to_bits() | 0x0008_0000_0000_0000u64)
}

fn nan_sumf(x: f32, y: f32) -> f32 {
    let v = if x.is_nan() { x } else { y };
    fbs(v.to_bits() | 0x0040_0000u32)
}

fn with_high_word(x: f64, high: u32) -> f64 {
    fb((x.to_bits() & 0xffff_ffffu64) | ((high as u64) << 32))
}

// ---- exact fused multiply-add ----

// Rounds the exact value `r * 2^e` (r nonzero) to nearest-even in the f64
// format, or in the f32 format when `single`. Returns the bits without sign.
fn fma_round(r0: u128, e0: i32, single: bool) -> u64 {
    let lz = r0.leading_zeros();
    let r = r0 << lz;
    let e = e0 - (lz as i32);
    let mant_bits: u32 = if single { 24 } else { 53 };
    let bias: i32 = if single { 127 } else { 1023 };
    let max_field: i32 = if single { 0xff } else { 0x7ff };
    let frac_bits = mant_bits - 1;
    let mut field = e + 127 + bias;
    if field >= max_field {
        return (max_field as u64) << frac_bits;
    }
    let mut shift = 128 - mant_bits;
    if field <= 0 {
        let extra = (1 - field) as u32;
        if extra > 200 {
            return 0;
        }
        shift = shift + extra;
        field = 0;
    }
    if shift > 128 {
        return 0;
    }
    let one: u128 = 1;
    let mut m: u64 = 0;
    let rem: u128;
    if shift == 128 {
        rem = r;
    } else {
        m = (r >> shift) as u64;
        rem = r & ((one << shift) - 1);
    }
    let half = one << (shift - 1);
    if rem > half || (rem == half && (m & 1) == 1) {
        m = m + 1;
    }
    if field == 0 {
        m
    } else {
        ((field as u64) << frac_bits) + (m - (1u64 << frac_bits))
    }
}

// Correctly rounded `x * y + z`. With `single`, the operands are widened f32
// values and the result is the f32 bit pattern in the low 32 bits.
fn fma_bits(x: f64, y: f64, z: f64, single: bool) -> u64 {
    let ux = x.to_bits();
    let uy = y.to_bits();
    let uz = z.to_bits();
    let frac: u64 = 0x000f_ffff_ffff_ffff;
    let mut ex = ((ux >> 52) & 0x7ff) as i32;
    let mut ey = ((uy >> 52) & 0x7ff) as i32;
    let mut ez = ((uz >> 52) & 0x7ff) as i32;
    let mut mx = ux & frac;
    let mut my = uy & frac;
    let mut mz = uz & frac;
    let special = ex == 0x7ff
        || ey == 0x7ff
        || ez == 0x7ff
        || (ex == 0 && mx == 0)
        || (ey == 0 && my == 0);
    if special {
        // NaN, infinity or an exact zero product. A NaN operand is returned
        // quieted (y, then x, then z, the hardware operand order), and a
        // finite product must not overflow against an infinite addend.
        // Otherwise the plain expression is exact.
        let quiet: u64 = 0x0008_0000_0000_0000;
        let v = if ey == 0x7ff && my != 0 {
            fb(uy | quiet)
        } else if ex == 0x7ff && mx != 0 {
            fb(ux | quiet)
        } else if ez == 0x7ff && mz != 0 {
            fb(uz | quiet)
        } else if ex != 0x7ff && ey != 0x7ff && ez == 0x7ff {
            z
        } else {
            x * y + z
        };
        if single {
            return (v as f32).to_bits() as u64;
        }
        return v.to_bits();
    }
    let sign_shift: u32 = if single { 31 } else { 63 };
    let sp = (ux ^ uy) >> 63;
    let sz = uz >> 63;
    if ex == 0 {
        let s = mx.leading_zeros() - 11;
        mx = mx << s;
        ex = 1 - (s as i32);
    } else {
        mx = mx | (1u64 << 52);
    }
    if ey == 0 {
        let s = my.leading_zeros() - 11;
        my = my << s;
        ey = 1 - (s as i32);
    } else {
        my = my | (1u64 << 52);
    }
    // p * 2^ep is the exact product, leading bit moved to bit 126.
    let mut p: u128 = (mx as u128) * (my as u128);
    let mut ep = ex + ey - 2150;
    let lp = p.leading_zeros() - 1;
    p = p << lp;
    ep = ep - (lp as i32);
    let r: u128;
    let e: i32;
    let sign: u64;
    if ez == 0 && mz == 0 {
        r = p;
        e = ep;
        sign = sp;
    } else {
        if ez == 0 {
            let s = mz.leading_zeros() - 11;
            mz = mz << s;
            ez = 1 - (s as i32);
        } else {
            mz = mz | (1u64 << 52);
        }
        let zz: u128 = (mz as u128) << 74;
        let ezz = ez - 1075 - 74;
        let d = ep - ezz;
        let big: u128;
        let small: u128;
        let sign_big: u64;
        let sign_small: u64;
        let shift: u32;
        if d >= 0 {
            big = p;
            small = zz;
            sign_big = sp;
            sign_small = sz;
            shift = d as u32;
            e = ep;
        } else {
            big = zz;
            small = p;
            sign_big = sz;
            sign_small = sp;
            shift = (0 - d) as u32;
            e = ezz;
        }
        let mut lost = false;
        let st: u128;
        if shift >= 128 {
            st = 0;
            lost = true;
        } else if shift == 0 {
            st = small;
        } else {
            st = small >> shift;
            lost = (small << (128 - shift)) != 0;
        }
        if sign_big == sign_small {
            let sum = big + st;
            r = if lost { sum | 1 } else { sum };
            sign = sign_big;
        } else if shift == 0 {
            if big >= st {
                r = big - st;
                sign = sign_big;
            } else {
                r = st - big;
                sign = sign_small;
            }
        } else {
            let diff = big - st;
            r = if lost { (diff - 1) | 1 } else { diff };
            sign = sign_big;
        }
        if r == 0 {
            return 0;
        }
    }
    fma_round(r, e, single) | (sign << sign_shift)
}

fn fma(x: f64, y: f64, z: f64) -> f64 {
    fb(fma_bits(x, y, z, false))
}

fn fmaf(x: f32, y: f32, z: f32) -> f32 {
    fbs(fma_bits(x as f64, y as f64, z as f64, true) as u32)
}

// ---- rounding to integers ----

fn trunc64(x: f64) -> f64 {
    let bits = x.to_bits();
    let e = (((bits >> 52) & 0x7ff) as i32) - 1023;
    if e >= 52 {
        return x;
    }
    if e < 0 {
        return fb(bits & 0x8000_0000_0000_0000u64);
    }
    let mask = (1u64 << ((52 - e) as u32)) - 1;
    fb(bits & !mask)
}

fn floor64(x: f64) -> f64 {
    let t = trunc64(x);
    if t > x { t - 1.0 } else { t }
}

fn ceil64(x: f64) -> f64 {
    let t = trunc64(x);
    if t < x { t + 1.0 } else { t }
}

fn copysign64(x: f64, sign: f64) -> f64 {
    let top: u64 = 0x8000_0000_0000_0000;
    fb((x.to_bits() & !top) | (sign.to_bits() & top))
}

fn round64(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    let t = trunc64(x);
    let d = (x - t).abs();
    if d >= 0.5 { t + copysign64(1.0, x) } else { t }
}

fn round_even64(x: f64) -> f64 {
    let t = trunc64(x);
    let d = (x - t).abs();
    if d > 0.5 || (d == 0.5 && ((t as i64) & 1) != 0) {
        t + copysign64(1.0, x)
    } else {
        t
    }
}

fn trunc32(x: f32) -> f32 {
    let bits = x.to_bits();
    let e = (((bits >> 23) & 0xff) as i32) - 127;
    if e >= 23 {
        return x;
    }
    if e < 0 {
        return fbs(bits & 0x8000_0000u32);
    }
    let mask = (1u32 << ((23 - e) as u32)) - 1;
    fbs(bits & !mask)
}

fn floor32(x: f32) -> f32 {
    let t = trunc32(x);
    if t > x { t - 1.0 } else { t }
}

fn ceil32(x: f32) -> f32 {
    let t = trunc32(x);
    if t < x { t + 1.0 } else { t }
}

fn copysign32(x: f32, sign: f32) -> f32 {
    let top: u32 = 0x8000_0000;
    fbs((x.to_bits() & !top) | (sign.to_bits() & top))
}

fn round32(x: f32) -> f32 {
    if x.is_nan() {
        return x + x;
    }
    let t = trunc32(x);
    let d = (x - t).abs();
    if d >= 0.5 { t + copysign32(1.0, x) } else { t }
}

fn round_even32(x: f32) -> f32 {
    let t = trunc32(x);
    let d = (x - t).abs();
    if d > 0.5 || (d == 0.5 && ((t as i32) & 1) != 0) {
        t + copysign32(1.0, x)
    } else {
        t
    }
}

// ---- integer powers (the compiler-rt `__powidf2`/`__powisf2` loop) ----

fn powi64(x: f64, n: i32) -> f64 {
    let mut a = x;
    let mut pow: u32 = if n < 0 { 0u32.wrapping_sub(n as u32) } else { n as u32 };
    let mut mul: f64 = 1.0;
    loop {
        if (pow & 1) != 0 {
            mul = mul * a;
        }
        pow = pow >> 1;
        if pow == 0 {
            break;
        }
        a = a * a;
    }
    if n < 0 { 1.0 / mul } else { mul }
}

fn powi32(x: f32, n: i32) -> f32 {
    let mut a = x;
    let mut pow: u32 = if n < 0 { 0u32.wrapping_sub(n as u32) } else { n as u32 };
    let mut mul: f32 = 1.0;
    loop {
        if (pow & 1) != 0 {
            mul = mul * a;
        }
        pow = pow >> 1;
        if pow == 0 {
            break;
        }
        a = a * a;
    }
    if n < 0 { 1.0 / mul } else { mul }
}

// ---- f64 exp / exp2 ----

fn top12(x: f64) -> u32 {
    (x.to_bits() >> 52) as u32
}

// Results that overflow or underflow the scale `2^(k/N)` on its own.
fn exp_special(tmp: f64, sbits0: u64, ki: u64, fused: bool) -> f64 {
    if (ki & 0x8000_0000u64) == 0 {
        // k > 0: the exponent of scale might have overflowed by <= 460.
        let sbits = sbits0.wrapping_sub(1009u64 << 52);
        let scale = fb(sbits);
        let inner = if fused { fma(tmp, scale, scale) } else { scale + scale * tmp };
        return fb(0x7f00_0000_0000_0000u64) * inner;
    }
    // k < 0: take care in the subnormal range.
    let sbits = sbits0.wrapping_add(1022u64 << 52);
    let scale = fb(sbits);
    let st = scale * tmp;
    let mut y = scale + st;
    if y.abs() < 1.0 {
        // Round y to the right precision before scaling it into the
        // subnormal range, avoiding a double rounding.
        let one: f64 = if y < 0.0 { -1.0 } else { 1.0 };
        let lo0 = (scale - y) + st;
        let hi = one + y;
        let lo = ((one - hi) + y) + lo0;
        y = (hi + lo) - one;
        if y == 0.0 {
            y = fb(sbits & 0x8000_0000_0000_0000u64);
        }
    }
    fb(0x0010_0000_0000_0000u64) * y
}

fn exp64(x: f64) -> f64 {
    let bits = x.to_bits();
    let mut abstop = top12(x) & 0x7ff;
    if abstop.wrapping_sub(0x3c9) >= 0x3f {
        if abstop.wrapping_sub(0x3c9) >= 0x8000_0000u32 {
            // Tiny x.
            return 1.0 + x;
        }
        if abstop >= 0x409 {
            if bits == 0xfff0_0000_0000_0000u64 {
                return 0.0;
            }
            if abstop >= 0x7ff {
                return 1.0 + x;
            }
            if (bits >> 63) != 0 {
                return 0.0;
            }
            return fb(0x7ff0_0000_0000_0000u64);
        }
        // Large x is special cased below.
        abstop = 0;
    }
    let shift = fb(EXP_K[1]);
    let z = fma(fb(EXP_K[0]), x, shift);
    let ki = z.to_bits();
    let kd = z - shift;
    let r0 = fma(fb(EXP_K[2]), kd, x);
    let r = fma(fb(EXP_K[3]), kd, r0);
    let idx = (2 * (ki & 127)) as usize;
    let top = ki << 45;
    let tail = fb(EXP_TAB[idx]);
    let sbits = EXP_TAB[idx + 1].wrapping_add(top);
    let r2 = r * r;
    let a = fma(fma(fb(EXP_K[5]), r, fb(EXP_K[4])), r2, r + tail);
    let tmp = fma(r2 * r2, fma(fb(EXP_K[7]), r, fb(EXP_K[6])), a);
    if abstop == 0 {
        return exp_special(tmp, sbits, ki, true);
    }
    let scale = fb(sbits);
    fma(tmp, scale, scale)
}

fn exp2_special(tmp: f64, sbits0: u64, ki: u64) -> f64 {
    if (ki & 0x8000_0000u64) == 0 {
        let sbits = sbits0.wrapping_sub(1u64 << 52);
        let scale = fb(sbits);
        return 2.0 * (scale + scale * tmp);
    }
    let sbits = sbits0.wrapping_add(1022u64 << 52);
    let scale = fb(sbits);
    let st = scale * tmp;
    let mut y = scale + st;
    if y < 1.0 {
        let lo0 = (scale - y) + st;
        let hi = 1.0 + y;
        let lo = ((1.0 - hi) + y) + lo0;
        y = (hi + lo) - 1.0;
        if y == 0.0 {
            y = 0.0;
        }
    }
    fb(0x0010_0000_0000_0000u64) * y
}

fn exp2_64(x: f64) -> f64 {
    let bits = x.to_bits();
    let mut abstop = top12(x) & 0x7ff;
    if abstop.wrapping_sub(0x3c9) >= 0x3f {
        if abstop.wrapping_sub(0x3c9) >= 0x8000_0000u32 {
            return 1.0 + x;
        }
        if abstop >= 0x409 {
            if bits == 0xfff0_0000_0000_0000u64 {
                return 0.0;
            }
            if abstop >= 0x7ff {
                return 1.0 + x;
            }
            if (bits >> 63) == 0 {
                return fb(0x7ff0_0000_0000_0000u64);
            }
            // x <= -1075.
            if bits >= 0xc090_cc00_0000_0000u64 {
                return 0.0;
            }
        }
        // |x| > 928.
        if (bits << 1) > 0x811a_0000_0000_0000u64 {
            abstop = 0;
        }
    }
    let shift = fb(EXP_K[8]);
    let kd0 = x + shift;
    let ki = kd0.to_bits();
    let kd = kd0 - shift;
    let r = x - kd;
    let idx = (2 * (ki & 127)) as usize;
    let top = ki << 45;
    let tail = fb(EXP_TAB[idx]);
    let sbits = EXP_TAB[idx + 1].wrapping_add(top);
    let r2 = r * r;
    let tmp = ((tail + r * fb(EXP_K[9])) + r2 * (fb(EXP_K[10]) + r * fb(EXP_K[11])))
        + (r2 * r2) * (fb(EXP_K[12]) + r * fb(EXP_K[13]));
    if abstop == 0 {
        return exp2_special(tmp, sbits, ki);
    }
    let scale = fb(sbits);
    scale + scale * tmp
}

// ---- f64 ln / log2 / log10 ----

fn ln64(x: f64) -> f64 {
    let mut ix = x.to_bits();
    let top = (ix >> 48) as u32;
    if ix.wrapping_sub(0x3fee_0000_0000_0000u64) < 0x0003_0900_0000_0000u64 {
        // Inputs close to 1.0.
        if ix == 0x3ff0_0000_0000_0000u64 {
            return 0.0;
        }
        let r = x - 1.0;
        let r2 = r * r;
        let r3 = r * r2;
        let a = fma(fb(LOG_K[10]), r2, fma(fb(LOG_K[9]), r, fb(LOG_K[8])));
        let b = fma(fb(LOG_K[13]), r2, fma(fb(LOG_K[12]), r, fb(LOG_K[11])));
        let c0 = fma(fb(LOG_K[16]), r2, fma(fb(LOG_K[15]), r, fb(LOG_K[14])));
        let c = fma(fb(LOG_K[17]), r3, c0);
        let q = fma(fma(c, r3, b), r3, a);
        // Split r so that rhi * rhi is exact.
        let w = fma(r, 134217728.0, r);
        let rhi = fma(-r, 134217728.0, w);
        let rlo = r - rhi;
        let sq = rhi * rhi;
        let b0 = fb(LOG_K[7]);
        let hi = fma(sq, b0, r);
        let lo0 = fma(sq, b0, r - hi);
        let lo = fma(b0 * rlo, r + rhi, lo0);
        return hi + fma(q, r3, lo);
    }
    if top.wrapping_sub(0x0010) >= 0x7fe0 {
        // x < 0x1p-1022 or inf or nan.
        if (ix << 1) == 0 {
            return fb(0xfff0_0000_0000_0000u64);
        }
        if ix == 0x7ff0_0000_0000_0000u64 {
            return x;
        }
        if (top & 0x8000) != 0 || (top & 0x7ff0) == 0x7ff0 {
            return (x - x) / (x - x);
        }
        // Subnormal: normalize.
        ix = (x * 4503599627370496.0).to_bits();
        ix = ix.wrapping_sub(52u64 << 52);
    }
    let tmp = ix.wrapping_sub(0x3fe6_0000_0000_0000u64);
    let i = ((tmp >> 45) & 127) as usize;
    let k = (tmp as i64) >> 52;
    let iz = ix.wrapping_sub(tmp & 0xfff0_0000_0000_0000u64);
    let invc = fb(LOG_TAB[2 * i]);
    let logc = fb(LOG_TAB[2 * i + 1]);
    let r = fma(fb(iz), invc, -1.0);
    let kd = k as f64;
    let w = fma(fb(LOG_K[0]), kd, logc);
    let hi = r + w;
    let lo = fma(fb(LOG_K[1]), kd, (w - hi) + r);
    let r2 = r * r;
    let a = fma(fb(LOG_K[2]), r2, lo);
    let b = fma(fb(LOG_K[4]), r, fb(LOG_K[3]));
    let c = fma(fb(LOG_K[6]), r, fb(LOG_K[5]));
    fma(r * r2, fma(c, r2, b), a) + hi
}

fn log2_64(x: f64) -> f64 {
    let mut ix = x.to_bits();
    let top = (ix >> 48) as u32;
    let mask32: u64 = 0xffff_ffff_0000_0000;
    let invln2hi = fb(LOG2_K[0]);
    let invln2lo = fb(LOG2_K[1]);
    if ix.wrapping_sub(0x3fee_a4af_0000_0000u64) < 0x0002_10aa_0000_0000u64 {
        if ix == 0x3ff0_0000_0000_0000u64 {
            return 0.0;
        }
        let r = x - 1.0;
        let rhi = fb(r.to_bits() & mask32);
        let rlo = r - rhi;
        let hi = rhi * invln2hi;
        let mut lo = rlo * invln2hi + r * invln2lo;
        let r2 = r * r;
        let r4 = r2 * r2;
        let p = r2 * (fb(LOG2_K[8]) + r * fb(LOG2_K[9]));
        let mut y = hi + p;
        lo = lo + ((hi - y) + p);
        let q0 = (fb(LOG2_K[10]) + r * fb(LOG2_K[11])) + r2 * (fb(LOG2_K[12]) + r * fb(LOG2_K[13]));
        let q1 = (fb(LOG2_K[14]) + r * fb(LOG2_K[15])) + r2 * (fb(LOG2_K[16]) + r * fb(LOG2_K[17]));
        lo = lo + r4 * (q0 + r4 * q1);
        y = y + lo;
        return y;
    }
    if top.wrapping_sub(0x0010) >= 0x7fe0 {
        if (ix << 1) == 0 {
            return fb(0xfff0_0000_0000_0000u64);
        }
        if ix == 0x7ff0_0000_0000_0000u64 {
            return x;
        }
        if (top & 0x8000) != 0 || (top & 0x7ff0) == 0x7ff0 {
            return (x - x) / (x - x);
        }
        ix = (x * 4503599627370496.0).to_bits();
        ix = ix.wrapping_sub(52u64 << 52);
    }
    let tmp = ix.wrapping_sub(0x3fe6_0000_0000_0000u64);
    let i = ((tmp >> 46) & 63) as usize;
    let k = (tmp as i64) >> 52;
    let iz = ix.wrapping_sub(tmp & 0xfff0_0000_0000_0000u64);
    let invc = fb(LOG2_TAB[4 * i]);
    let logc = fb(LOG2_TAB[4 * i + 1]);
    let z = fb(iz);
    let kd = k as f64;
    let r = ((z - fb(LOG2_TAB[4 * i + 2])) - fb(LOG2_TAB[4 * i + 3])) * invc;
    let rhi = fb(r.to_bits() & mask32);
    let rlo = r - rhi;
    let t1 = rhi * invln2hi;
    let t2 = rlo * invln2hi + r * invln2lo;
    let t3 = kd + logc;
    let hi = t3 + t1;
    let lo = ((t3 - hi) + t1) + t2;
    let r2 = r * r;
    let r4 = r2 * r2;
    let p = ((fb(LOG2_K[2]) + r * fb(LOG2_K[3])) + r2 * (fb(LOG2_K[4]) + r * fb(LOG2_K[5])))
        + r4 * (fb(LOG2_K[6]) + r * fb(LOG2_K[7]));
    (lo + r2 * p) + hi
}

fn log10_64(x0: f64) -> f64 {
    let mut x = x0;
    if x.is_nan() {
        return x + x;
    }
    let mut hx = high_word(x) as i32;
    let lx = x.to_bits() as u32;
    let mut k: i32 = 0;
    if hx < 0x0010_0000 {
        if (((hx & 0x7fff_ffff) as u32) | lx) == 0 {
            return -18014398509481984.0 / x.abs();
        }
        if hx < 0 {
            return fb(0x7ff8_0000_0000_0000u64);
        }
        k = k - 54;
        x = x * 18014398509481984.0;
        hx = high_word(x) as i32;
    }
    if hx >= 0x7ff0_0000 {
        return x + x;
    }
    k = k + ((hx >> 20) - 1023);
    let i = ((k as u32) & 0x8000_0000u32) >> 31;
    hx = (hx & 0x000f_ffff) | ((0x3ff - (i as i32)) << 20);
    let y = (k + (i as i32)) as f64;
    x = with_high_word(x, hx as u32);
    let z = y * 3.69423907715893078616e-13 + 4.34294481903251816668e-01 * ln64(x);
    z + y * 3.01029995663611771306e-01
}

// ---- f64 powf ----

// 0 if not an integer, 1 if odd, 2 if even. `iy` is nonzero and finite.
fn pow_checkint(iy: u64) -> i32 {
    let e = ((iy >> 52) & 0x7ff) as i32;
    if e < 0x3ff {
        return 0;
    }
    if e > 0x3ff + 52 {
        return 2;
    }
    let bit = 1u64 << ((0x3ff + 52 - e) as u32);
    if (iy & (bit - 1)) != 0 {
        return 0;
    }
    if (iy & bit) != 0 {
        return 1;
    }
    2
}

fn pow_zeroinfnan(i: u64) -> bool {
    (i << 1).wrapping_sub(1) >= 0xffdf_ffff_ffff_ffffu64
}

fn pow_signaling(i: u64) -> bool {
    ((i ^ 0x0008_0000_0000_0000u64) << 1) > 0xfff0_0000_0000_0000u64
}

// sign * exp(x + xtail), `sign_bias` selecting a negative result.
fn pow_exp(x: f64, xtail: f64, sign_bias: u64) -> f64 {
    let mut abstop = top12(x) & 0x7ff;
    if abstop.wrapping_sub(0x3c9) >= 0x3f {
        if abstop.wrapping_sub(0x3c9) >= 0x8000_0000u32 {
            let one = 1.0 + x;
            return if sign_bias != 0 { -one } else { one };
        }
        if abstop >= 0x409 {
            let negative = sign_bias != 0;
            if (x.to_bits() >> 63) != 0 {
                return if negative { -0.0 } else { 0.0 };
            }
            let inf = fb(0x7ff0_0000_0000_0000u64);
            return if negative { -inf } else { inf };
        }
        abstop = 0;
    }
    let shift = fb(EXP_K[1]);
    let z = fma(x, fb(EXP_K[0]), shift);
    let ki = z.to_bits();
    let kd = z - shift;
    let r0 = fma(kd, fb(EXP_K[2]), x);
    let r1 = fma(kd, fb(EXP_K[3]), r0);
    let r = xtail + r1;
    let idx = (2 * (ki & 127)) as usize;
    let top = ki.wrapping_add(sign_bias) << 45;
    let tail = fb(EXP_TAB[idx]);
    let sbits = top.wrapping_add(EXP_TAB[idx + 1]);
    let r2 = r * r;
    let a = fma(r2, fma(r, fb(EXP_K[5]), fb(EXP_K[4])), r + tail);
    let tmp = fma(r2 * r2, fma(r, fb(EXP_K[7]), fb(EXP_K[6])), a);
    if abstop == 0 {
        return exp_special(tmp, sbits, ki, true);
    }
    let scale = fb(sbits);
    fma(tmp, scale, scale)
}

fn pow64(x: f64, y: f64) -> f64 {
    let mut sign_bias: u64 = 0;
    let mut ix = x.to_bits();
    let iy = y.to_bits();
    let mut topx = top12(x);
    let topy = top12(y);
    let one_bits: u64 = 0x3ff0_0000_0000_0000;
    if topx.wrapping_sub(0x001) >= 0x7ff - 0x001 || (topy & 0x7ff).wrapping_sub(0x3be) >= 0x43e - 0x3be {
        // (x < 0x1p-1022 or inf or nan) or (|y| < 0x1p-65 or |y| >= 0x1p63 or nan).
        if pow_zeroinfnan(iy) {
            if (iy << 1) == 0 {
                return if pow_signaling(ix) { nan_sum(x, y) } else { 1.0 };
            }
            if ix == one_bits {
                return if pow_signaling(iy) { nan_sum(x, y) } else { 1.0 };
            }
            if (ix << 1) > 0xffe0_0000_0000_0000u64 || (iy << 1) > 0xffe0_0000_0000_0000u64 {
                return nan_sum(x, y);
            }
            if (ix << 1) == (one_bits << 1) {
                return 1.0;
            }
            if ((ix << 1) < (one_bits << 1)) == ((iy >> 63) == 0) {
                // |x| < 1 && y == inf, or |x| > 1 && y == -inf.
                return 0.0;
            }
            return y * y;
        }
        if pow_zeroinfnan(ix) {
            let mut x2 = x * x;
            if (ix >> 63) != 0 && pow_checkint(iy) == 1 {
                x2 = -x2;
            }
            return if (iy >> 63) != 0 { 1.0 / x2 } else { x2 };
        }
        // Here x and y are nonzero finite.
        if (ix >> 63) != 0 {
            let yint = pow_checkint(iy);
            if yint == 0 {
                return (x - x) / (x - x);
            }
            if yint == 1 {
                sign_bias = 0x800u64 << 7;
            }
            ix = ix & 0x7fff_ffff_ffff_ffffu64;
            topx = topx & 0x7ff;
        }
        if (topy & 0x7ff).wrapping_sub(0x3be) >= 0x43e - 0x3be {
            // sign_bias == 0 here because y is not odd.
            if ix == one_bits {
                return 1.0;
            }
            if (topy & 0x7ff) < 0x3be {
                // |y| < 2^-65, x^y ~= 1 + y*log(x).
                return if ix > one_bits { 1.0 + y } else { 1.0 - y };
            }
            return if (ix > one_bits) == (topy < 0x800) {
                fb(0x7ff0_0000_0000_0000u64)
            } else {
                0.0
            };
        }
        if topx == 0 {
            // Normalize subnormal x so the exponent becomes negative.
            ix = (x * 4503599627370496.0).to_bits();
            ix = ix & 0x7fff_ffff_ffff_ffffu64;
            ix = ix.wrapping_sub(52u64 << 52);
        }
    }
    // hi + lo = log(x), lo carrying about 15 extra bits.
    let tmp = ix.wrapping_sub(0x3fe6_9555_0000_0000u64);
    let i = ((tmp >> 45) & 127) as usize;
    let k = (tmp as i64) >> 52;
    let iz = ix.wrapping_sub(tmp & 0xfff0_0000_0000_0000u64);
    let kd = k as f64;
    let invc = fb(POW_LOG_TAB[3 * i]);
    let logc = fb(POW_LOG_TAB[3 * i + 1]);
    let logctail = fb(POW_LOG_TAB[3 * i + 2]);
    let r = fma(fb(iz), invc, -1.0);
    let t1 = fma(kd, fb(POW_LOG_K[0]), logc);
    let t2 = t1 + r;
    let lo1 = fma(kd, fb(POW_LOG_K[1]), logctail);
    let lo2 = (t1 - t2) + r;
    let ar = fb(POW_LOG_K[2]) * r;
    let ar2 = r * ar;
    let ar3 = r * ar2;
    let hi = t2 + ar2;
    let lo3 = fma(ar, r, -ar2);
    let lo4 = (t2 - hi) + ar2;
    let p0 = fma(r, fb(POW_LOG_K[4]), fb(POW_LOG_K[3]));
    let p1 = fma(r, fb(POW_LOG_K[6]), fb(POW_LOG_K[5]));
    let p2 = fma(r, fb(POW_LOG_K[8]), fb(POW_LOG_K[7]));
    let p = fma(ar2, fma(ar2, p2, p1), p0);
    let lo = fma(ar3, p, lo4 + (lo3 + (lo1 + lo2)));
    let lhi = hi + lo;
    let llo = (hi - lhi) + lo;
    let ehi = y * lhi;
    let elo = fma(y, llo, fma(y, lhi, -ehi));
    pow_exp(ehi, elo, sign_bias)
}

// ---- f64 hypot ----

fn hypot64(x: f64, y: f64) -> f64 {
    let mut ha = (high_word(x) & 0x7fff_ffff) as i32;
    let mut hb = (high_word(y) & 0x7fff_ffff) as i32;
    let mut a = x;
    let mut b = y;
    if hb > ha {
        a = y;
        b = x;
        let j = ha;
        ha = hb;
        hb = j;
    }
    a = a.abs();
    b = b.abs();
    if ha >= 0x7ff0_0000 {
        // Infinity or NaN. An infinity wins unless a signaling NaN is present.
        let inf = fb(0x7ff0_0000_0000_0000u64);
        let signaling = pow_signaling(a.to_bits()) || pow_signaling(b.to_bits());
        if !signaling && (a == inf || b == inf) {
            return inf;
        }
        return nan_sum(a, b);
    }
    if ha - hb > 0x03c0_0000 {
        // x / y > 2^60.
        return a + b;
    }
    let mut k: i32 = 0;
    if ha > 0x5f30_0000 {
        // a > 2^500: scale by 2^-600.
        ha = ha - 0x2580_0000;
        hb = hb - 0x2580_0000;
        k = k + 600;
        a = with_high_word(a, ha as u32);
        b = with_high_word(b, hb as u32);
    }
    if hb < 0x20b0_0000 {
        // b < 2^-500.
        if hb <= 0x000f_ffff {
            if b == 0.0 {
                return a;
            }
            let t1 = fb(0x7fd0_0000_0000_0000u64);
            b = b * t1;
            a = a * t1;
            k = k - 1022;
            ha = high_word(a) as i32;
            hb = high_word(b) as i32;
            if hb > ha {
                let t = a;
                a = b;
                b = t;
                let j = ha;
                ha = hb;
                hb = j;
            }
        } else {
            ha = ha + 0x2580_0000;
            hb = hb + 0x2580_0000;
            k = k - 600;
            a = with_high_word(a, ha as u32);
            b = with_high_word(b, hb as u32);
        }
    }
    let mut w = a - b;
    if w > b {
        let t1 = with_high_word(0.0, ha as u32);
        let t2 = a - t1;
        w = (t1 * t1 - (b * (-b) - t2 * (a + t1))).sqrt();
    } else {
        a = a + a;
        let y1 = with_high_word(0.0, hb as u32);
        let y2 = b - y1;
        let t1 = with_high_word(0.0, (ha + 0x0010_0000) as u32);
        let t2 = a - t1;
        w = (t1 * y1 - (w * (-w) - (t1 * y2 + t2 * b))).sqrt();
    }
    if k != 0 {
        let t1 = with_high_word(0.0, ((1023 + k) as u32) << 20);
        return t1 * w;
    }
    w
}

// ---- f32 exp / exp2 / ln / log2 / log10 / powf ----

fn expf32(x: f32) -> f32 {
    let xd = x as f64;
    let bits = x.to_bits();
    let abstop = (bits >> 20) & 0x7ff;
    if abstop >= 0x42b {
        // |x| >= 88 or x is nan.
        if bits == 0xff80_0000u32 {
            return 0.0;
        }
        if abstop >= 0x7f8 {
            return x + x;
        }
        if x > 88.72283172607421875f32 {
            return fbs(0x7f80_0000u32);
        }
        if x < -103.972076416015625f32 {
            return 0.0;
        }
    }
    let inv = fb(EXP2F_K[5]);
    let shift = fb(EXP2F_K[4]);
    let z0 = fma(inv, xd, shift);
    let ki = z0.to_bits();
    let kd = z0 - shift;
    let r = fma(inv, xd, -kd);
    let t = EXP2F_TAB[(ki & 31) as usize].wrapping_add(ki << 47);
    let s = fb(t);
    let z = fma(fb(EXP2F_K[6]), r, fb(EXP2F_K[7]));
    let r2 = r * r;
    let y0 = fma(fb(EXP2F_K[8]), r, 1.0);
    let y = fma(z, r2, y0);
    (s * y) as f32
}

// 2^xd as f32, `sign_bias` selecting a negative result.
fn exp2f_core(xd: f64, sign_bias: u64) -> f32 {
    let shift = fb(EXP2F_K[0]);
    let kd0 = xd + shift;
    let ki = kd0.to_bits();
    let kd = kd0 - shift;
    let r = xd - kd;
    let t = EXP2F_TAB[(ki & 31) as usize].wrapping_add(ki.wrapping_add(sign_bias) << 47);
    let s = fb(t);
    let z = fma(r, fb(EXP2F_K[1]), fb(EXP2F_K[2]));
    let r2 = r * r;
    let y0 = fma(r, fb(EXP2F_K[3]), 1.0);
    let y = fma(z, r2, y0);
    (s * y) as f32
}

fn exp2f32(x: f32) -> f32 {
    let bits = x.to_bits();
    let abstop = (bits >> 20) & 0x7ff;
    if abstop >= 0x430 {
        // |x| >= 128 or x is nan.
        if bits == 0xff80_0000u32 {
            return 0.0;
        }
        if abstop >= 0x7f8 {
            return x + x;
        }
        if x > 0.0 {
            return fbs(0x7f80_0000u32);
        }
        if x <= -150.0 {
            return 0.0;
        }
    }
    exp2f_core(x as f64, 0)
}

fn lnf32(x: f32) -> f32 {
    let mut ix = x.to_bits();
    if ix == 0x3f80_0000u32 {
        return 0.0;
    }
    if ix.wrapping_sub(0x0080_0000) >= 0x7f80_0000u32 - 0x0080_0000u32 {
        // x < 0x1p-126 or inf or nan.
        if (ix << 1) == 0 {
            return fbs(0xff80_0000u32);
        }
        if ix == 0x7f80_0000u32 {
            return x;
        }
        if (ix & 0x8000_0000u32) != 0 || (ix << 1) >= 0xff00_0000u32 {
            return (x - x) / (x - x);
        }
        ix = (x * 8388608.0f32).to_bits();
        ix = ix.wrapping_sub(23u32 << 23);
    }
    let tmp = ix.wrapping_sub(0x3f33_0000);
    let i = ((tmp >> 19) & 15) as usize;
    let k = (tmp as i32) >> 23;
    let iz = ix.wrapping_sub(tmp & 0xff80_0000u32);
    let invc = fb(LOGF_TAB[2 * i]);
    let logc = fb(LOGF_TAB[2 * i + 1]);
    let z = fbs(iz) as f64;
    let r = fma(invc, z, -1.0);
    let y0 = fma(k as f64, fb(LOGF_K[0]), logc);
    let r2 = r * r;
    let y1 = fma(fb(LOGF_K[2]), r, fb(LOGF_K[3]));
    let y2 = fma(fb(LOGF_K[1]), r2, y1);
    fma(r2, y2, r + y0) as f32
}

fn log2f32(x: f32) -> f32 {
    let mut ix = x.to_bits();
    if ix == 0x3f80_0000u32 {
        return 0.0;
    }
    if ix.wrapping_sub(0x0080_0000) >= 0x7f80_0000u32 - 0x0080_0000u32 {
        if (ix << 1) == 0 {
            return fbs(0xff80_0000u32);
        }
        if ix == 0x7f80_0000u32 {
            return x;
        }
        if (ix & 0x8000_0000u32) != 0 || (ix << 1) >= 0xff00_0000u32 {
            return (x - x) / (x - x);
        }
        ix = (x * 8388608.0f32).to_bits();
        ix = ix.wrapping_sub(23u32 << 23);
    }
    let tmp = ix.wrapping_sub(0x3f33_0000);
    let i = ((tmp >> 19) & 15) as usize;
    let top = tmp & 0xff80_0000u32;
    let iz = ix.wrapping_sub(top);
    let k = (tmp as i32) >> 23;
    let invc = fb(LOG2F_TAB[2 * i]);
    let logc = fb(LOG2F_TAB[2 * i + 1]);
    let z = fbs(iz) as f64;
    let r = fma(invc, z, -1.0);
    let y0 = (k as f64) + logc;
    let r2 = r * r;
    let y1 = fma(fb(LOG2F_K[1]), r, fb(LOG2F_K[2]));
    let y2 = fma(fb(LOG2F_K[0]), r2, y1);
    let p = fma(fb(LOG2F_K[3]), r, y0);
    fma(r2, y2, p) as f32
}

fn log10f32(x0: f32) -> f32 {
    let mut x = x0;
    if x.is_nan() {
        return x + x;
    }
    let mut hx = x.to_bits() as i32;
    let mut k: i32 = 0;
    if hx < 0x0080_0000 {
        if (hx & 0x7fff_ffff) == 0 {
            return -33554432.0f32 / x.abs();
        }
        if hx < 0 {
            return fbs(0x7fc0_0000u32);
        }
        k = k - 25;
        x = x * 33554432.0f32;
        hx = x.to_bits() as i32;
    }
    if hx >= 0x7f80_0000 {
        return x + x;
    }
    k = k + ((hx >> 23) - 127);
    let i = ((k as u32) & 0x8000_0000u32) >> 31;
    hx = (hx & 0x007f_ffff) | ((0x7f - (i as i32)) << 23);
    let y = (k + (i as i32)) as f32;
    x = fbs(hx as u32);
    let z = y * 7.9034151668e-07f32 + 4.3429449201e-01f32 * lnf32(x);
    z + y * 3.0102920532e-01f32
}

fn powf_checkint(iy: u32) -> i32 {
    let e = ((iy >> 23) & 0xff) as i32;
    if e < 0x7f {
        return 0;
    }
    if e > 0x7f + 23 {
        return 2;
    }
    let bit = 1u32 << ((0x7f + 23 - e) as u32);
    if (iy & (bit - 1)) != 0 {
        return 0;
    }
    if (iy & bit) != 0 {
        return 1;
    }
    2
}

fn powf_zeroinfnan(i: u32) -> bool {
    (i << 1).wrapping_sub(1) >= 0xfeff_ffffu32
}

fn powf_signaling(i: u32) -> bool {
    ((i ^ 0x0040_0000u32) << 1) > 0xff80_0000u32
}

fn powf32(x: f32, y: f32) -> f32 {
    let mut sign_bias: u64 = 0;
    let mut ix = x.to_bits();
    let iy = y.to_bits();
    let one_bits: u32 = 0x3f80_0000;
    if ix.wrapping_sub(0x0080_0000) >= 0x7f80_0000u32 - 0x0080_0000u32 || powf_zeroinfnan(iy) {
        // (x < 0x1p-126 or inf or nan) or (y is 0 or inf or nan).
        if powf_zeroinfnan(iy) {
            if (iy << 1) == 0 {
                return if powf_signaling(ix) { nan_sumf(x, y) } else { 1.0 };
            }
            if ix == one_bits {
                return if powf_signaling(iy) { nan_sumf(x, y) } else { 1.0 };
            }
            if (ix << 1) > 0xff00_0000u32 || (iy << 1) > 0xff00_0000u32 {
                return nan_sumf(x, y);
            }
            if (ix << 1) == (one_bits << 1) {
                return 1.0;
            }
            if ((ix << 1) < (one_bits << 1)) == ((iy & 0x8000_0000u32) == 0) {
                return 0.0;
            }
            return y * y;
        }
        if powf_zeroinfnan(ix) {
            let mut x2 = x * x;
            if (ix & 0x8000_0000u32) != 0 && powf_checkint(iy) == 1 {
                x2 = -x2;
            }
            return if (iy & 0x8000_0000u32) != 0 { 1.0 / x2 } else { x2 };
        }
        // x and y are nonzero finite.
        if (ix & 0x8000_0000u32) != 0 {
            let yint = powf_checkint(iy);
            if yint == 0 {
                return (x - x) / (x - x);
            }
            if yint == 1 {
                sign_bias = 1u64 << 16;
            }
            ix = ix & 0x7fff_ffffu32;
        }
        if ix < 0x0080_0000 {
            // Normalize subnormal x so the exponent becomes negative.
            ix = (x * 8388608.0f32).to_bits();
            ix = ix & 0x7fff_ffffu32;
            ix = ix.wrapping_sub(23u32 << 23);
        }
    }
    // log2(x) in double precision.
    let tmp = ix.wrapping_sub(0x3f33_0000);
    let i = ((tmp >> 19) & 15) as usize;
    let top = tmp & 0xff80_0000u32;
    let iz = ix.wrapping_sub(top);
    let k = (top as i32) >> 23;
    let invc = fb(POWF_LOG2_TAB[2 * i]);
    let logc = fb(POWF_LOG2_TAB[2 * i + 1]);
    let z = fbs(iz) as f64;
    let r = fma(invc, z, -1.0);
    let y0 = logc + (k as f64);
    let r2 = r * r;
    let a = fma(r, fb(POWF_LOG2_K[0]), fb(POWF_LOG2_K[1]));
    let p = fma(r, fb(POWF_LOG2_K[2]), fb(POWF_LOG2_K[3]));
    let r4 = r2 * r2;
    let q0 = fma(r, fb(POWF_LOG2_K[4]), y0);
    let q = fma(r2, p, q0);
    let logx = fma(a, r4, q);
    let ylogx = (y as f64) * logx;
    if ((ylogx.to_bits() >> 47) & 0xffff) >= 0x80bf {
        // |y * log2(x)| >= 126.
        let negative = sign_bias != 0;
        if ylogx > fb(0x405f_ffff_ffd1_d571u64) {
            let inf = fbs(0x7f80_0000u32);
            return if negative { -inf } else { inf };
        }
        if ylogx <= -150.0 {
            return if negative { -0.0 } else { 0.0 };
        }
    }
    exp2f_core(ylogx, sign_bias)
}

fn hypotf32(x: f32, y: f32) -> f32 {
    let ax = x.abs();
    let ay = y.abs();
    let ha = ax.to_bits();
    let hb = ay.to_bits();
    if ha == 0x7f80_0000u32 && !powf_signaling(hb) {
        return ax;
    }
    if hb == 0x7f80_0000u32 && !powf_signaling(ha) {
        return ay;
    }
    if ha > 0x7f80_0000u32 || hb > 0x7f80_0000u32 {
        return nan_sumf(ax, ay);
    }
    let dx = ax as f64;
    let dy = ay as f64;
    (dx * dx + dy * dy).sqrt() as f32
}

// ---- f32 sin / cos ----

// Sine (even `n`) or cosine (odd `n`) polynomial of table entry `p`.
fn sinf_poly(x: f64, x2: f64, p: usize, n: i32) -> f32 {
    if (n & 1) == 0 {
        let x3 = x * x2;
        let s1 = fma(x2, fb(SINCOSF[p + 13]), fb(SINCOSF[p + 12]));
        let x7 = x3 * x2;
        let s = fma(x3, fb(SINCOSF[p + 11]), x);
        fma(s1, x7, s) as f32
    } else {
        let x4 = x2 * x2;
        let c2 = fma(x2, fb(SINCOSF[p + 10]), fb(SINCOSF[p + 9]));
        let c1 = fma(x2, fb(SINCOSF[p + 7]), fb(SINCOSF[p + 6]));
        let x6 = x4 * x2;
        let c = fma(x4, fb(SINCOSF[p + 8]), c1);
        fma(c2, x6, c) as f32
    }
}

// Shared sine/cosine driver: `flip` is 0 for sine and 1 for cosine.
fn sincosf32(y: f32, flip: i32) -> f32 {
    let x = y as f64;
    let xi = y.to_bits();
    let top = (xi >> 20) & 0x7ff;
    if top < 0x3f4 {
        // |y| < pi/4.
        let x2 = x * x;
        if top < 0x398 {
            // |y| < 2^-12.
            return if flip == 0 { y } else { 1.0 };
        }
        return sinf_poly(x, x2, 0, flip);
    }
    if top < 0x42f {
        // |y| < 120: single multiply-subtract reduction.
        let r = x * fb(SINCOSF[4]);
        let n = ((r as i32) + 0x0080_0000) >> 24;
        let xr = fma(-(n as f64), fb(SINCOSF[5]), x);
        let s = fb(SINCOSF[(n & 3) as usize]);
        let p: usize = if (n & 2) != 0 { 14 } else { 0 };
        return sinf_poly(xr * s, xr * xr, p, n ^ flip);
    }
    if top < 0x7f8 {
        // Large finite: reduce against 4/pi to 192 bits.
        let sign = (xi >> 31) as i32;
        let at = ((xi >> 26) & 15) as usize;
        let shift = (xi >> 23) & 7;
        let m = ((xi & 0x00ff_ffff) | 0x0080_0000) << shift;
        let m64 = m as u64;
        let low = (m.wrapping_mul(INV_PIO4[at] as u32)) as u64;
        let res1 = m64 * INV_PIO4[at + 4];
        let res2 = m64 * INV_PIO4[at + 8];
        let mut res0 = (res2 >> 32) | (low << 32);
        res0 = res0.wrapping_add(res1);
        let q = res0.wrapping_add(1u64 << 61) >> 62;
        res0 = res0.wrapping_sub(q << 62);
        let n = q as i32;
        let xr = ((res0 as i64) as f64) * fb(0x3c19_21fb_5444_2d18u64);
        let s = fb(SINCOSF[((n + sign) & 3) as usize]);
        let p: usize = if ((n + sign) & 2) != 0 { 14 } else { 0 };
        return sinf_poly(xr * s, xr * xr, p, n ^ flip);
    }
    (y - y) / (y - y)
}

// ---- the method surface ----

/// Inherent math methods shared by `f32` and `f64`.
pub trait FloatMath: Sized {
    fn floor(self) -> Self;
    fn ceil(self) -> Self;
    fn round(self) -> Self;
    fn round_ties_even(self) -> Self;
    fn trunc(self) -> Self;
    fn fract(self) -> Self;
    fn signum(self) -> Self;
    fn copysign(self, sign: Self) -> Self;
    fn clamp(self, min: Self, max: Self) -> Self;
    fn recip(self) -> Self;
    fn mul_add(self, a: Self, b: Self) -> Self;
    fn powi(self, n: i32) -> Self;
    fn powf(self, n: Self) -> Self;
    fn exp(self) -> Self;
    fn exp2(self) -> Self;
    fn ln(self) -> Self;
    fn log(self, base: Self) -> Self;
    fn log2(self) -> Self;
    fn log10(self) -> Self;
    fn hypot(self, other: Self) -> Self;
    fn to_degrees(self) -> Self;
    fn to_radians(self) -> Self;
    fn div_euclid(self, rhs: Self) -> Self;
    fn rem_euclid(self, rhs: Self) -> Self;
    // Path forms (`f64::sqrt(x)`, `f64::max` as a function value) of the
    // compiler's primitive kernels.
    fn sqrt(self) -> Self;
    fn abs(self) -> Self;
    fn min(self, other: Self) -> Self;
    fn max(self, other: Self) -> Self;
}

impl FloatMath for f64 {
    fn floor(self) -> f64 {
        floor64(self)
    }
    fn ceil(self) -> f64 {
        ceil64(self)
    }
    fn round(self) -> f64 {
        round64(self)
    }
    fn round_ties_even(self) -> f64 {
        round_even64(self)
    }
    fn trunc(self) -> f64 {
        trunc64(self)
    }
    fn fract(self) -> f64 {
        self - trunc64(self)
    }
    fn signum(self) -> f64 {
        if self.is_nan() { self } else { copysign64(1.0, self) }
    }
    fn copysign(self, sign: f64) -> f64 {
        copysign64(self, sign)
    }
    fn clamp(self, min: f64, max: f64) -> f64 {
        if !(min <= max) {
            panic!("min > max, or either was NaN");
        }
        let mut x = self;
        if x < min {
            x = min;
        }
        if x > max {
            x = max;
        }
        x
    }
    fn recip(self) -> f64 {
        1.0 / self
    }
    fn mul_add(self, a: f64, b: f64) -> f64 {
        fma(self, a, b)
    }
    fn powi(self, n: i32) -> f64 {
        powi64(self, n)
    }
    fn powf(self, n: f64) -> f64 {
        pow64(self, n)
    }
    fn exp(self) -> f64 {
        exp64(self)
    }
    fn exp2(self) -> f64 {
        exp2_64(self)
    }
    fn ln(self) -> f64 {
        ln64(self)
    }
    fn log(self, base: f64) -> f64 {
        ln64(self) / ln64(base)
    }
    fn log2(self) -> f64 {
        log2_64(self)
    }
    fn log10(self) -> f64 {
        log10_64(self)
    }
    fn hypot(self, other: f64) -> f64 {
        hypot64(self, other)
    }
    fn to_degrees(self) -> f64 {
        self * (180.0f64 / 3.14159265358979323846264338327950288f64)
    }
    fn to_radians(self) -> f64 {
        self * (3.14159265358979323846264338327950288f64 / 180.0f64)
    }
    fn div_euclid(self, rhs: f64) -> f64 {
        let q = trunc64(self / rhs);
        if self % rhs < 0.0 {
            return if rhs > 0.0 { q - 1.0 } else { q + 1.0 };
        }
        q
    }
    fn rem_euclid(self, rhs: f64) -> f64 {
        let r = self % rhs;
        if r < 0.0 { r + rhs.abs() } else { r }
    }
    fn sqrt(self) -> f64 {
        self.sqrt()
    }
    fn abs(self) -> f64 {
        self.abs()
    }
    fn min(self, other: f64) -> f64 {
        self.min(other)
    }
    fn max(self, other: f64) -> f64 {
        self.max(other)
    }
}

impl FloatMath for f32 {
    fn floor(self) -> f32 {
        floor32(self)
    }
    fn ceil(self) -> f32 {
        ceil32(self)
    }
    fn round(self) -> f32 {
        round32(self)
    }
    fn round_ties_even(self) -> f32 {
        round_even32(self)
    }
    fn trunc(self) -> f32 {
        trunc32(self)
    }
    fn fract(self) -> f32 {
        self - trunc32(self)
    }
    fn signum(self) -> f32 {
        if self.is_nan() { self } else { copysign32(1.0, self) }
    }
    fn copysign(self, sign: f32) -> f32 {
        copysign32(self, sign)
    }
    fn clamp(self, min: f32, max: f32) -> f32 {
        if !(min <= max) {
            panic!("min > max, or either was NaN");
        }
        let mut x = self;
        if x < min {
            x = min;
        }
        if x > max {
            x = max;
        }
        x
    }
    fn recip(self) -> f32 {
        1.0 / self
    }
    fn mul_add(self, a: f32, b: f32) -> f32 {
        fmaf(self, a, b)
    }
    fn powi(self, n: i32) -> f32 {
        powi32(self, n)
    }
    fn powf(self, n: f32) -> f32 {
        powf32(self, n)
    }
    fn exp(self) -> f32 {
        expf32(self)
    }
    fn exp2(self) -> f32 {
        exp2f32(self)
    }
    fn ln(self) -> f32 {
        lnf32(self)
    }
    fn log(self, base: f32) -> f32 {
        lnf32(self) / lnf32(base)
    }
    fn log2(self) -> f32 {
        log2f32(self)
    }
    fn log10(self) -> f32 {
        log10f32(self)
    }
    fn hypot(self, other: f32) -> f32 {
        hypotf32(self, other)
    }
    fn to_degrees(self) -> f32 {
        self * 57.2957795130823208767981548141051703f32
    }
    fn to_radians(self) -> f32 {
        self * (3.14159265358979323846264338327950288f32 / 180.0f32)
    }
    fn div_euclid(self, rhs: f32) -> f32 {
        let q = trunc32(self / rhs);
        if self % rhs < 0.0 {
            return if rhs > 0.0 { q - 1.0 } else { q + 1.0 };
        }
        q
    }
    fn rem_euclid(self, rhs: f32) -> f32 {
        let r = self % rhs;
        if r < 0.0 { r + rhs.abs() } else { r }
    }
    fn sqrt(self) -> f32 {
        self.sqrt()
    }
    fn abs(self) -> f32 {
        self.abs()
    }
    fn min(self, other: f32) -> f32 {
        self.min(other)
    }
    fn max(self, other: f32) -> f32 {
        self.max(other)
    }
}

/// Trigonometry available for `f32` only (see the file header).
pub trait FloatTrig: Sized {
    fn sin(self) -> Self;
    fn cos(self) -> Self;
    fn sin_cos(self) -> (Self, Self);
}

impl FloatTrig for f32 {
    fn sin(self) -> f32 {
        sincosf32(self, 0)
    }
    fn cos(self) -> f32 {
        sincosf32(self, 1)
    }
    fn sin_cos(self) -> (f32, f32) {
        (sincosf32(self, 0), sincosf32(self, 1))
    }
}

// ---- constants (`std::f64::consts::PI`, `std::f64::MAX`, ...) ----

pub mod f64_items {
    pub const RADIX: u32 = 2;
    pub const MANTISSA_DIGITS: u32 = 53;
    pub const DIGITS: u32 = 15;
    pub const EPSILON: f64 = 2.2204460492503131e-16;
    pub const MIN: f64 = -1.7976931348623157e308;
    pub const MIN_POSITIVE: f64 = 2.2250738585072014e-308;
    pub const MAX: f64 = 1.7976931348623157e308;
    pub const MIN_EXP: i32 = -1021;
    pub const MAX_EXP: i32 = 1024;
    pub const MIN_10_EXP: i32 = -307;
    pub const MAX_10_EXP: i32 = 308;
}

pub mod f64_consts {
    pub const PI: f64 = 3.14159265358979323846264338327950288;
    pub const TAU: f64 = 6.28318530717958647692528676655900577;
    pub const FRAC_PI_2: f64 = 1.57079632679489661923132169163975144;
    pub const FRAC_PI_3: f64 = 1.04719755119659774615421446109316763;
    pub const FRAC_PI_4: f64 = 0.785398163397448309615660845819875721;
    pub const FRAC_PI_6: f64 = 0.52359877559829887307710723054658381;
    pub const FRAC_PI_8: f64 = 0.39269908169872415480783042290993786;
    pub const FRAC_1_PI: f64 = 0.318309886183790671537767526745028724;
    pub const FRAC_2_PI: f64 = 0.636619772367581343075535053490057448;
    pub const FRAC_2_SQRT_PI: f64 = 1.12837916709551257389615890312154517;
    pub const SQRT_2: f64 = 1.41421356237309504880168872420969808;
    pub const FRAC_1_SQRT_2: f64 = 0.707106781186547524400844362104849039;
    pub const E: f64 = 2.71828182845904523536028747135266250;
    pub const LOG2_E: f64 = 1.44269504088896340735992468100189214;
    pub const LOG2_10: f64 = 3.32192809488736234787031942948939018;
    pub const LOG10_E: f64 = 0.434294481903251827651128918916605082;
    pub const LOG10_2: f64 = 0.301029995663981195213738894724493027;
    pub const LN_2: f64 = 0.693147180559945309417232121458176568;
    pub const LN_10: f64 = 2.30258509299404568401799145468436421;
}

pub mod f32_items {
    pub const RADIX: u32 = 2;
    pub const MANTISSA_DIGITS: u32 = 24;
    pub const DIGITS: u32 = 6;
    pub const EPSILON: f32 = 1.19209290e-07;
    pub const MIN: f32 = -3.40282347e+38;
    pub const MIN_POSITIVE: f32 = 1.17549435e-38;
    pub const MAX: f32 = 3.40282347e+38;
    pub const MIN_EXP: i32 = -125;
    pub const MAX_EXP: i32 = 128;
    pub const MIN_10_EXP: i32 = -37;
    pub const MAX_10_EXP: i32 = 38;
}

pub mod f32_consts {
    pub const PI: f32 = 3.14159265358979323846264338327950288;
    pub const TAU: f32 = 6.28318530717958647692528676655900577;
    pub const FRAC_PI_2: f32 = 1.57079632679489661923132169163975144;
    pub const FRAC_PI_3: f32 = 1.04719755119659774615421446109316763;
    pub const FRAC_PI_4: f32 = 0.785398163397448309615660845819875721;
    pub const FRAC_PI_6: f32 = 0.52359877559829887307710723054658381;
    pub const FRAC_PI_8: f32 = 0.39269908169872415480783042290993786;
    pub const FRAC_1_PI: f32 = 0.318309886183790671537767526745028724;
    pub const FRAC_2_PI: f32 = 0.636619772367581343075535053490057448;
    pub const FRAC_2_SQRT_PI: f32 = 1.12837916709551257389615890312154517;
    pub const SQRT_2: f32 = 1.41421356237309504880168872420969808;
    pub const FRAC_1_SQRT_2: f32 = 0.707106781186547524400844362104849039;
    pub const E: f32 = 2.71828182845904523536028747135266250;
    pub const LOG2_E: f32 = 1.44269504088896340735992468100189214;
    pub const LOG2_10: f32 = 3.32192809488736234787031942948939018;
    pub const LOG10_E: f32 = 0.434294481903251827651128918916605082;
    pub const LOG10_2: f32 = 0.301029995663981195213738894724493027;
    pub const LN_2: f32 = 0.693147180559945309417232121458176568;
    pub const LN_10: f32 = 2.30258509299404568401799145468436421;
}

// ---- tables (bit patterns, generated from the Arm Optimized Routines data) ----
//
// EXP_K: invln2N, shift, negln2hiN, negln2loN, exp poly C2..C5, exp2 shift,
//   exp2 poly C1..C5. EXP_TAB: (tail, scale bits) per 2^(k/128).
// LOG_K: ln2hi, ln2lo, poly A0..A4, near-one poly B0..B10. LOG_TAB: (invc, logc).
// LOG2_K: invln2hi, invln2lo, poly A0..A5, near-one poly B0..B9.
//   LOG2_TAB: (invc, logc, chi, clo).
// POW_LOG_K: ln2hi, ln2lo, poly A0..A6. POW_LOG_TAB: (invc, logc, logctail).
// EXP2F_K: shift_scaled, poly C0..C2, shift, invln2_scaled, scaled poly C0..C2.
// LOGF_K: ln2, poly A0..A2. LOG2F_K: poly A0..A3. POWF_LOG2_K: poly A0..A4.
//   Their tables: (invc, logc).
// SINCOSF: two entries of sign[4], hpi_inv, hpi, c0..c4, s1..s3.
// INV_PIO4: 4/pi in 32-bit limbs.
static EXP_K: [u64; 14] = [
    0x40671547652b82fe, 0x4338000000000000, 0xbf762e42fefa0000, 0xbd0cf79abc9e3b3a,
    0x3fdffffffffffdbd, 0x3fc555555555543c, 0x3fa55555cf172b91, 0x3f81111167a4d017,
    0x42c8000000000000, 0x3fe62e42fefa39ef, 0x3fcebfbdff82c424, 0x3fac6b08d70cf4b5,
    0x3f83b2abd24650cc, 0x3f55d7e09b4e3a84,
];
static EXP_TAB: [u64; 256] = [
    0x0000000000000000, 0x3ff0000000000000, 0x3c9b3b4f1a88bf6e, 0x3feff63da9fb3335,
    0xbc7160139cd8dc5d, 0x3fefec9a3e778061, 0xbc905e7a108766d1, 0x3fefe315e86e7f85,
    0x3c8cd2523567f613, 0x3fefd9b0d3158574, 0xbc8bce8023f98efa, 0x3fefd06b29ddf6de,
    0x3c60f74e61e6c861, 0x3fefc74518759bc8, 0x3c90a3e45b33d399, 0x3fefbe3ecac6f383,
    0x3c979aa65d837b6d, 0x3fefb5586cf9890f, 0x3c8eb51a92fdeffc, 0x3fefac922b7247f7,
    0x3c3ebe3d702f9cd1, 0x3fefa3ec32d3d1a2, 0xbc6a033489906e0b, 0x3fef9b66affed31b,
    0xbc9556522a2fbd0e, 0x3fef9301d0125b51, 0xbc5080ef8c4eea55, 0x3fef8abdc06c31cc,
    0xbc91c923b9d5f416, 0x3fef829aaea92de0, 0x3c80d3e3e95c55af, 0x3fef7a98c8a58e51,
    0xbc801b15eaa59348, 0x3fef72b83c7d517b, 0xbc8f1ff055de323d, 0x3fef6af9388c8dea,
    0x3c8b898c3f1353bf, 0x3fef635beb6fcb75, 0xbc96d99c7611eb26, 0x3fef5be084045cd4,
    0x3c9aecf73e3a2f60, 0x3fef54873168b9aa, 0xbc8fe782cb86389d, 0x3fef4d5022fcd91d,
    0x3c8a6f4144a6c38d, 0x3fef463b88628cd6, 0x3c807a05b0e4047d, 0x3fef3f49917ddc96,
    0x3c968efde3a8a894, 0x3fef387a6e756238, 0x3c875e18f274487d, 0x3fef31ce4fb2a63f,
    0x3c80472b981fe7f2, 0x3fef2b4565e27cdd, 0xbc96b87b3f71085e, 0x3fef24dfe1f56381,
    0x3c82f7e16d09ab31, 0x3fef1e9df51fdee1, 0xbc3d219b1a6fbffa, 0x3fef187fd0dad990,
    0x3c8b3782720c0ab4, 0x3fef1285a6e4030b, 0x3c6e149289cecb8f, 0x3fef0cafa93e2f56,
    0x3c834d754db0abb6, 0x3fef06fe0a31b715, 0x3c864201e2ac744c, 0x3fef0170fc4cd831,
    0x3c8fdd395dd3f84a, 0x3feefc08b26416ff, 0xbc86a3803b8e5b04, 0x3feef6c55f929ff1,
    0xbc924aedcc4b5068, 0x3feef1a7373aa9cb, 0xbc9907f81b512d8e, 0x3feeecae6d05d866,
    0xbc71d1e83e9436d2, 0x3feee7db34e59ff7, 0xbc991919b3ce1b15, 0x3feee32dc313a8e5,
    0x3c859f48a72a4c6d, 0x3feedea64c123422, 0xbc9312607a28698a, 0x3feeda4504ac801c,
    0xbc58a78f4817895b, 0x3feed60a21f72e2a, 0xbc7c2c9b67499a1b, 0x3feed1f5d950a897,
    0x3c4363ed60c2ac11, 0x3feece086061892d, 0x3c9666093b0664ef, 0x3feeca41ed1d0057,
    0x3c6ecce1daa10379, 0x3feec6a2b5c13cd0, 0x3c93ff8e3f0f1230, 0x3feec32af0d7d3de,
    0x3c7690cebb7aafb0, 0x3feebfdad5362a27, 0x3c931dbdeb54e077, 0x3feebcb299fddd0d,
    0xbc8f94340071a38e, 0x3feeb9b2769d2ca7, 0xbc87deccdc93a349, 0x3feeb6daa2cf6642,
    0xbc78dec6bd0f385f, 0x3feeb42b569d4f82, 0xbc861246ec7b5cf6, 0x3feeb1a4ca5d920f,
    0x3c93350518fdd78e, 0x3feeaf4736b527da, 0x3c7b98b72f8a9b05, 0x3feead12d497c7fd,
    0x3c9063e1e21c5409, 0x3feeab07dd485429, 0x3c34c7855019c6ea, 0x3feea9268a5946b7,
    0x3c9432e62b64c035, 0x3feea76f15ad2148, 0xbc8ce44a6199769f, 0x3feea5e1b976dc09,
    0xbc8c33c53bef4da8, 0x3feea47eb03a5585, 0xbc845378892be9ae, 0x3feea34634ccc320,
    0xbc93cedd78565858, 0x3feea23882552225, 0x3c5710aa807e1964, 0x3feea155d44ca973,
    0xbc93b3efbf5e2228, 0x3feea09e667f3bcd, 0xbc6a12ad8734b982, 0x3feea012750bdabf,
    0xbc6367efb86da9ee, 0x3fee9fb23c651a2f, 0xbc80dc3d54e08851, 0x3fee9f7df9519484,
    0xbc781f647e5a3ecf, 0x3fee9f75e8ec5f74, 0xbc86ee4ac08b7db0, 0x3fee9f9a48a58174,
    0xbc8619321e55e68a, 0x3fee9feb564267c9, 0x3c909ccb5e09d4d3, 0x3feea0694fde5d3f,
    0xbc7b32dcb94da51d, 0x3feea11473eb0187, 0x3c94ecfd5467c06b, 0x3feea1ed0130c132,
    0x3c65ebe1abd66c55, 0x3feea2f336cf4e62, 0xbc88a1c52fb3cf42, 0x3feea427543e1a12,
    0xbc9369b6f13b3734, 0x3feea589994cce13, 0xbc805e843a19ff1e, 0x3feea71a4623c7ad,
    0xbc94d450d872576e, 0x3feea8d99b4492ed, 0x3c90ad675b0e8a00, 0x3feeaac7d98a6699,
    0x3c8db72fc1f0eab4, 0x3feeace5422aa0db, 0xbc65b6609cc5e7ff, 0x3feeaf3216b5448c,
    0x3c7bf68359f35f44, 0x3feeb1ae99157736, 0xbc93091fa71e3d83, 0x3feeb45b0b91ffc6,
    0xbc5da9b88b6c1e29, 0x3feeb737b0cdc5e5, 0xbc6c23f97c90b959, 0x3feeba44cbc8520f,
    0xbc92434322f4f9aa, 0x3feebd829fde4e50, 0xbc85ca6cd7668e4b, 0x3feec0f170ca07ba,
    0x3c71affc2b91ce27, 0x3feec49182a3f090, 0x3c6dd235e10a73bb, 0x3feec86319e32323,
    0xbc87c50422622263, 0x3feecc667b5de565, 0x3c8b1c86e3e231d5, 0x3feed09bec4a2d33,
    0xbc91bbd1d3bcbb15, 0x3feed503b23e255d, 0x3c90cc319cee31d2, 0x3feed99e1330b358,
    0x3c8469846e735ab3, 0x3feede6b5579fdbf, 0xbc82dfcd978e9db4, 0x3feee36bbfd3f37a,
    0x3c8c1a7792cb3387, 0x3feee89f995ad3ad, 0xbc907b8f4ad1d9fa, 0x3feeee07298db666,
    0xbc55c3d956dcaeba, 0x3feef3a2b84f15fb, 0xbc90a40e3da6f640, 0x3feef9728de5593a,
    0xbc68d6f438ad9334, 0x3feeff76f2fb5e47, 0xbc91eee26b588a35, 0x3fef05b030a1064a,
    0x3c74ffd70a5fddcd, 0x3fef0c1e904bc1d2, 0xbc91bdfbfa9298ac, 0x3fef12c25bd71e09,
    0x3c736eae30af0cb3, 0x3fef199bdd85529c, 0x3c8ee3325c9ffd94, 0x3fef20ab5fffd07a,
    0x3c84e08fd10959ac, 0x3fef27f12e57d14b, 0x3c63cdaf384e1a67, 0x3fef2f6d9406e7b5,
    0x3c676b2c6c921968, 0x3fef3720dcef9069, 0xbc808a1883ccb5d2, 0x3fef3f0b555dc3fa,
    0xbc8fad5d3ffffa6f, 0x3fef472d4a07897c, 0xbc900dae3875a949, 0x3fef4f87080d89f2,
    0x3c74a385a63d07a7, 0x3fef5818dcfba487, 0xbc82919e2040220f, 0x3fef60e316c98398,
    0x3c8e5a50d5c192ac, 0x3fef69e603db3285, 0x3c843a59ac016b4b, 0x3fef7321f301b460,
    0xbc82d52107b43e1f, 0x3fef7c97337b9b5f, 0xbc892ab93b470dc9, 0x3fef864614f5a129,
    0x3c74b604603a88d3, 0x3fef902ee78b3ff6, 0x3c83c5ec519d7271, 0x3fef9a51fbc74c83,
    0xbc8ff7128fd391f0, 0x3fefa4afa2a490da, 0xbc8dae98e223747d, 0x3fefaf482d8e67f1,
    0x3c8ec3bc41aa2008, 0x3fefba1bee615a27, 0x3c842b94c3a9eb32, 0x3fefc52b376bba97,
    0x3c8a64a931d185ee, 0x3fefd0765b6e4540, 0xbc8e37bae43be3ed, 0x3fefdbfdad9cbe14,
    0x3c77893b4d91cd9d, 0x3fefe7c1819e90d8, 0x3c5305c14160cc89, 0x3feff3c22b8f71f1,
];
static LOG_K: [u64; 18] = [
    0x3fe62e42fefa3800, 0x3d2ef35793c76730, 0xbfe0000000000001, 0x3fd555555551305b,
    0xbfcfffffffeb4590, 0x3fc999b324f10111, 0xbfc55575e506c89f, 0xbfe0000000000000,
    0x3fd5555555555577, 0xbfcffffffffffdcb, 0x3fc999999995dd0c, 0xbfc55555556745a7,
    0x3fc24924a344de30, 0xbfbfffffa4423d65, 0x3fbc7184282ad6ca, 0xbfb999eb43b068ff,
    0x3fb78182f7afd085, 0xbfb5521375d145cd,
];
static LOG_TAB: [u64; 256] = [
    0x3ff734f0c3e0de9f, 0xbfd7cc7f79e69000, 0x3ff713786a2ce91f, 0xbfd76feec20d0000,
    0x3ff6f26008fab5a0, 0xbfd713e31351e000, 0x3ff6d1a61f138c7d, 0xbfd6b85b38287800,
    0x3ff6b1490bc5b4d1, 0xbfd65d5590807800, 0x3ff69147332f0cba, 0xbfd602d076180000,
    0x3ff6719f18224223, 0xbfd5a8ca86909000, 0x3ff6524f99a51ed9, 0xbfd54f4356035000,
    0x3ff63356aa8f24c4, 0xbfd4f637c36b4000, 0x3ff614b36b9ddc14, 0xbfd49da7fda85000,
    0x3ff5f66452c65c4c, 0xbfd445923989a800, 0x3ff5d867b5912c4f, 0xbfd3edf439b0b800,
    0x3ff5babccb5b90de, 0xbfd396ce448f7000, 0x3ff59d61f2d91a78, 0xbfd3401e17bda000,
    0x3ff5805612465687, 0xbfd2e9e2ef468000, 0x3ff56397cee76bd3, 0xbfd2941b3830e000,
    0x3ff54725e2a77f93, 0xbfd23ec58cda8800, 0x3ff52aff42064583, 0xbfd1e9e129279000,
    0x3ff50f22dbb2bddf, 0xbfd1956d2b48f800, 0x3ff4f38f4734ded7, 0xbfd141679ab9f800,
    0x3ff4d843cfde2840, 0xbfd0edd094ef9800, 0x3ff4bd3ec078a3c8, 0xbfd09aa518db1000,
    0x3ff4a27fc3e0258a, 0xbfd047e65263b800, 0x3ff4880524d48434, 0xbfcfeb224586f000,
    0x3ff46dce1b192d0b, 0xbfcf474a7517b000, 0x3ff453d9d3391854, 0xbfcea4443d103000,
    0x3ff43a2744b4845a, 0xbfce020d44e9b000, 0x3ff420b54115f8fb, 0xbfcd60a22977f000,
    0x3ff40782da3ef4b1, 0xbfccc00104959000, 0x3ff3ee8f5d57fe8f, 0xbfcc202956891000,
    0x3ff3d5d9a00b4ce9, 0xbfcb81178d811000, 0x3ff3bd60c010c12b, 0xbfcae2c9ccd3d000,
    0x3ff3a5242b75dab8, 0xbfca45402e129000, 0x3ff38d22cd9fd002, 0xbfc9a877681df000,
    0x3ff3755bc5847a1c, 0xbfc90c6d69483000, 0x3ff35dce49ad36e2, 0xbfc87120a645c000,
    0x3ff34679984dd440, 0xbfc7d68fb4143000, 0x3ff32f5cceffcb24, 0xbfc73cb83c627000,
    0x3ff3187775a10d49, 0xbfc6a39a9b376000, 0x3ff301c8373e3990, 0xbfc60b3154b7a000,
    0x3ff2eb4ebb95f841, 0xbfc5737d76243000, 0x3ff2d50a0219a9d1, 0xbfc4dc7b8fc23000,
    0x3ff2bef9a8b7fd2a, 0xbfc4462c51d20000, 0x3ff2a91c7a0c1bab, 0xbfc3b08abc830000,
    0x3ff293726014b530, 0xbfc31b996b490000, 0x3ff27dfa5757a1f5, 0xbfc2875490a44000,
    0x3ff268b39b1d3bbf, 0xbfc1f3b9f879a000, 0x3ff2539d838ff5bd, 0xbfc160c8252ca000,
    0x3ff23eb7aac9083b, 0xbfc0ce7f57f72000, 0x3ff22a012ba940b6, 0xbfc03cdc49fea000,
    0x3ff2157996cc4132, 0xbfbf57bdbc4b8000, 0x3ff201201dd2fc9b, 0xbfbe370896404000,
    0x3ff1ecf4494d480b, 0xbfbd17983ef94000, 0x3ff1d8f5528f6569, 0xbfbbf9674ed8a000,
    0x3ff1c52311577e7c, 0xbfbadc79202f6000, 0x3ff1b17c74cb26e9, 0xbfb9c0c3e7288000,
    0x3ff19e010c2c1ab6, 0xbfb8a646b372c000, 0x3ff18ab07bb670bd, 0xbfb78d01b3ac0000,
    0x3ff1778a25efbcb6, 0xbfb674f145380000, 0x3ff1648d354c31da, 0xbfb55e0e6d878000,
    0x3ff151b990275fdd, 0xbfb4485cdea1e000, 0x3ff13f0ea432d24c, 0xbfb333d94d6aa000,
    0x3ff12c8b7210f9da, 0xbfb22079f8c56000, 0x3ff11a3028ecb531, 0xbfb10e4698622000,
    0x3ff107fbda8434af, 0xbfaffa6c6ad20000, 0x3ff0f5ee0f4e6bb3, 0xbfadda8d4a774000,
    0x3ff0e4065d2a9fce, 0xbfabbcece4850000, 0x3ff0d244632ca521, 0xbfa9a1894012c000,
    0x3ff0c0a77ce2981a, 0xbfa788583302c000, 0x3ff0af2f83c636d1, 0xbfa5715e67d68000,
    0x3ff09ddb98a01339, 0xbfa35c8a49658000, 0x3ff08cabaf52e7df, 0xbfa149e364154000,
    0x3ff07b9f2f4e28fb, 0xbf9e72c082eb8000, 0x3ff06ab58c358f19, 0xbf9a55f152528000,
    0x3ff059eea5ecf92c, 0xbf963d62cf818000, 0x3ff04949cdd12c90, 0xbf9228fb8caa0000,
    0x3ff038c6c6f0ada9, 0xbf8c317b20f90000, 0x3ff02865137932a9, 0xbf8419355daa0000,
    0x3ff0182427ea7348, 0xbf781203c2ec0000, 0x3ff008040614b195, 0xbf60040979240000,
    0x3fefe01ff726fa1a, 0x3f6feff384900000, 0x3fefa11cc261ea74, 0x3f87dc41353d0000,
    0x3fef6310b081992e, 0x3f93cea3c4c28000, 0x3fef25f63ceeadcd, 0x3f9b9fc114890000,
    0x3feee9c8039113e7, 0x3fa1b0d8ce110000, 0x3feeae8078cbb1ab, 0x3fa58a5bd001c000,
    0x3fee741aa29d0c9b, 0x3fa95c8340d88000, 0x3fee3a91830a99b5, 0x3fad276aef578000,
    0x3fee01e009609a56, 0x3fb07598e598c000, 0x3fedca01e577bb98, 0x3fb253f5e30d2000,
    0x3fed92f20b7c9103, 0x3fb42edd8b380000, 0x3fed5cac66fb5cce, 0x3fb606598757c000,
    0x3fed272caa5ede9d, 0x3fb7da76356a0000, 0x3fecf26e3e6b2ccd, 0x3fb9ab434e1c6000,
    0x3fecbe6da2a77902, 0x3fbb78c7bb0d6000, 0x3fec8b266d37086d, 0x3fbd431332e72000,
    0x3fec5894bd5d5804, 0x3fbf0a3171de6000, 0x3fec26b533bb9f8c, 0x3fc067152b914000,
    0x3febf583eeece73f, 0x3fc147858292b000, 0x3febc4fd75db96c1, 0x3fc2266ecdca3000,
    0x3feb951e0c864a28, 0x3fc303d7a6c55000, 0x3feb65e2c5ef3e2c, 0x3fc3dfc33c331000,
    0x3feb374867c9888b, 0x3fc4ba366b7a8000, 0x3feb094b211d304a, 0x3fc5933928d1f000,
    0x3feadbe885f2ef7e, 0x3fc66acd2418f000, 0x3feaaf1d31603da2, 0x3fc740f8ec669000,
    0x3fea82e63fd358a7, 0x3fc815c0f51af000, 0x3fea5740ef09738b, 0x3fc8e92954f68000,
    0x3fea2c2a90ab4b27, 0x3fc9bb3602f84000, 0x3fea01a01393f2d1, 0x3fca8bed1c2c0000,
    0x3fe9d79f24db3c1b, 0x3fcb5b515c01d000, 0x3fe9ae2505c7b190, 0x3fcc2967ccbcc000,
    0x3fe9852ef297ce2f, 0x3fccf635d5486000, 0x3fe95cbaeea44b75, 0x3fcdc1bd3446c000,
    0x3fe934c69de74838, 0x3fce8c01b8cfe000, 0x3fe90d4f2f6752e6, 0x3fcf5509c0179000,
    0x3fe8e6528effd79d, 0x3fd00e6c121fb800, 0x3fe8bfce9fcc007c, 0x3fd071b80e93d000,
    0x3fe899c0dabec30e, 0x3fd0d46b9e867000, 0x3fe87427aa2317fb, 0x3fd13687334bd000,
    0x3fe84f00acb39a08, 0x3fd1980d67234800, 0x3fe82a49e8653e55, 0x3fd1f8ffe0cc8000,
    0x3fe8060195f40260, 0x3fd2595fd7636800, 0x3fe7e22563e0a329, 0x3fd2b9300914a800,
    0x3fe7beb377dcb5ad, 0x3fd3187210436000, 0x3fe79baa679725c2, 0x3fd377266dec1800,
    0x3fe77907f2170657, 0x3fd3d54ffbaf3000, 0x3fe756cadbd6130c, 0x3fd432eee32fe000,
];
static LOG2_K: [u64; 18] = [
    0x3ff7154765200000, 0x3de705fc2eefa200, 0xbfe71547652b8339, 0x3fdec709dc3a04be,
    0xbfd7154764702ffb, 0x3fd2776c50034c48, 0xbfcec7b328ea92bc, 0x3fca6225e117f92e,
    0xbfe71547652b82fe, 0x3fdec709dc3a03f7, 0xbfd71547652b7c3f, 0x3fd2776c50f05be4,
    0xbfcec709dd768fe5, 0x3fca61761ec4e736, 0xbfc7153fbc64a79b, 0x3fc484d154f01b4a,
    0xbfc289e4a72c383c, 0x3fc0b32f285aee66,
];
static LOG2_TAB: [u64; 256] = [
    0x3ff724286bb1acf8, 0xbfe1095feecdb000, 0x3fe6200012b90a8e, 0x3c8904ab0644b605,
    0x3ff6e1f766d2cca1, 0xbfe08494bd76d000, 0x3fe66000045734a6, 0x3c61ff9bea62f7a9,
    0x3ff6a13d0e30d48a, 0xbfe00143aee8f800, 0x3fe69fffc325f2c5, 0x3c827ecfcb3c90ba,
    0x3ff661ec32d06c85, 0xbfdefec5360b4000, 0x3fe6e00038b95a04, 0x3c88ff8856739326,
    0x3ff623fa951198f8, 0xbfddfdd91ab7e000, 0x3fe71fffe09994e3, 0x3c8afd40275f82b1,
    0x3ff5e75ba4cf026c, 0xbfdcffae0cc79000, 0x3fe7600015590e10, 0xbc72fd75b4238341,
    0x3ff5ac055a214fb8, 0xbfdc043811fda000, 0x3fe7a00012655bd5, 0x3c7808e67c242b76,
    0x3ff571ed0f166e1e, 0xbfdb0b67323ae000, 0x3fe7e0003259e9a6, 0xbc6208e426f622b7,
    0x3ff53909590bf835, 0xbfda152f5a2db000, 0x3fe81fffedb4b2d2, 0xbc8402461ea5c92f,
    0x3ff5014fed61addd, 0xbfd9217f5af86000, 0x3fe860002dfafcc3, 0x3c6df7f4a2f29a1f,
    0x3ff4cab88e487bd0, 0xbfd8304db0719000, 0x3fe89ffff78c6b50, 0xbc8e0453094995fd,
    0x3ff49539b4334fee, 0xbfd74189f9a9e000, 0x3fe8e00039671566, 0xbc8a04f3bec77b45,
    0x3ff460cbdfafd569, 0xbfd6552bb5199000, 0x3fe91fffe2bf1745, 0xbc77fa34400e203c,
    0x3ff42d664ee4b953, 0xbfd56b23a29b1000, 0x3fe95fffcc5c9fd1, 0xbc76ff8005a0695d,
    0x3ff3fb01111dd8a6, 0xbfd483650f5fa000, 0x3fe9a0003bba4767, 0x3c70f8c4c4ec7e03,
    0x3ff3c995b70c5836, 0xbfd39de937f6a000, 0x3fe9dfffe7b92da5, 0x3c8e7fd9478c4602,
    0x3ff3991c4ab6fd4a, 0xbfd2baa1538d6000, 0x3fea1fffd72efdaf, 0xbc6a0c554dcdae7e,
    0x3ff3698e0ce099b5, 0xbfd1d98340ca4000, 0x3fea5fffde04ff95, 0x3c867da98ce9b26b,
    0x3ff33ae48213e7b2, 0xbfd0fa853a40e000, 0x3fea9fffca5e8d2b, 0xbc8284c9b54c13de,
    0x3ff30d191985bdb1, 0xbfd01d9c32e73000, 0x3feadfffddad03ea, 0x3c5812c8ea602e3c,
    0x3ff2e025cab271d7, 0xbfce857da2fa6000, 0x3feb1ffff10d3d4d, 0xbc8efaddad27789c,
    0x3ff2b404cf13cd82, 0xbfccd3c8633d8000, 0x3feb5fffce21165a, 0x3c53cb1719c61237,
    0x3ff288b02c7ccb50, 0xbfcb26034c14a000, 0x3feb9fffd950e674, 0x3c73f7d94194ce00,
    0x3ff25e2263944de5, 0xbfc97c1c2f4fe000, 0x3febe000139ca8af, 0x3c750ac4215d9bc0,
    0x3ff234563d8615b1, 0xbfc7d6023f800000, 0x3fec20005b46df99, 0x3c6beea653e9c1c9,
    0x3ff20b46e33eaf38, 0xbfc633a71a05e000, 0x3fec600040b9f7ae, 0xbc7c079f274a70d6,
    0x3ff1e2eefdcda3dd, 0xbfc494f5e9570000, 0x3feca0006255fd8a, 0xbc7a0b4076e84c1f,
    0x3ff1bb4a580b3930, 0xbfc2f9e424e0a000, 0x3fecdfffd94c095d, 0x3c88f933f99ab5d7,
    0x3ff19453847f2200, 0xbfc162595afdc000, 0x3fed1ffff975d6cf, 0xbc582c08665fe1be,
    0x3ff16e06c0d5d73c, 0xbfbf9c9a75bd8000, 0x3fed5fffa2561c93, 0xbc7b04289bd295f3,
    0x3ff1485f47b7e4c2, 0xbfbc7b575bf9c000, 0x3fed9fff9d228b0c, 0x3c870251340fa236,
    0x3ff12358ad0085d1, 0xbfb960c60ff48000, 0x3fede00065bc7e16, 0xbc75011e16a4d80c,
    0x3ff0fef00f532227, 0xbfb64ce247b60000, 0x3fee200002f64791, 0x3c89802f09ef62e0,
    0x3ff0db2077d03a8f, 0xbfb33f78b2014000, 0x3fee600057d7a6d8, 0xbc7e0b75580cf7fa,
    0x3ff0b7e6d65980d9, 0xbfb0387d1a42c000, 0x3feea00027edc00c, 0xbc8c848309459811,
    0x3ff0953efe7b408d, 0xbfaa6f9208b50000, 0x3feee0006cf5cb7c, 0xbc8f8027951576f4,
    0x3ff07325cac53b83, 0xbfa47a954f770000, 0x3fef2000782b7dcc, 0xbc8f81d97274538f,
    0x3ff05197e40d1b5c, 0xbf9d23a8c50c0000, 0x3fef6000260c450a, 0xbc4071002727ffdc,
    0x3ff03091c1208ea2, 0xbf916a2629780000, 0x3fef9fffe88cd533, 0xbc581bdce1fda8b0,
    0x3ff0101025b37e21, 0xbf7720f8d8e80000, 0x3fefdfffd50f8689, 0x3c87f91acb918e6e,
    0x3fefc07ef9caa76b, 0x3f86fe53b1500000, 0x3ff0200004292367, 0x3c9b7ff365324681,
    0x3fef4465d3f6f184, 0x3fa11ccce10f8000, 0x3ff05fffe3e3d668, 0x3c86fa08ddae957b,
    0x3feecc079f84107f, 0x3fac4dfc8c8b8000, 0x3ff0a0000a85a757, 0xbc57e2de80d3fb91,
    0x3fee573a99975ae8, 0x3fb3aa321e574000, 0x3ff0e0001a5f3fcc, 0xbc91823305c5f014,
    0x3fede5d6f0bd3de6, 0x3fb918a0d08b8000, 0x3ff11ffff8afbaf5, 0xbc8bfabb6680bac2,
    0x3fed77b681ff38b3, 0x3fbe72e9da044000, 0x3ff15fffe54d91ad, 0xbc9d7f121737e7ef,
    0x3fed0cb5724de943, 0x3fc1dcd2507f6000, 0x3ff1a00011ac36e1, 0x3c9c000a0516f5ff,
    0x3feca4b2dc0e7563, 0x3fc476ab03dea000, 0x3ff1e00019c84248, 0xbc9082fbe4da5da0,
    0x3fec3f8ee8d6cb51, 0x3fc7074377e22000, 0x3ff220000ffe5e6e, 0xbc88fdd04c9cfb43,
    0x3febdd2b4f020c4c, 0x3fc98ede8ba94000, 0x3ff26000269fd891, 0x3c8cfe2a7994d182,
    0x3feb7d6c006015ca, 0x3fcc0db86ad2e000, 0x3ff2a00029a6e6da, 0xbc700273715e8bc5,
    0x3feb20366e2e338f, 0x3fce840aafcee000, 0x3ff2dfffe0293e39, 0x3c9b7c39dab2a6f9,
    0x3feac57026295039, 0x3fd0790ab4678000, 0x3ff31ffff7dcf082, 0x3c7df1336edc5254,
    0x3fea6d01bc2731dd, 0x3fd1ac056801c000, 0x3ff35ffff05a8b60, 0xbc9e03564ccd31eb,
    0x3fea16d3bc3ff18b, 0x3fd2db11d4fee000, 0x3ff3a0002e0eaecc, 0x3c75f0e74bd3a477,
    0x3fe9c2d14967fead, 0x3fd406464ec58000, 0x3ff3e000043bb236, 0x3c9c7dcb149d8833,
    0x3fe970e4f47c9902, 0x3fd52dbe093af000, 0x3ff4200002d187ff, 0x3c7e08afcf2d3d28,
    0x3fe920fb3982bcf2, 0x3fd651902050d000, 0x3ff460000d387cb1, 0x3c820837856599a6,
    0x3fe8d30187f759f1, 0x3fd771d2cdeaf000, 0x3ff4a00004569f89, 0xbc89fa5c904fbcd2,
    0x3fe886e5ebb9f66d, 0x3fd88e9c857d9000, 0x3ff4e000043543f3, 0xbc781125ed175329,
    0x3fe83c97b658b994, 0x3fd9a80155e16000, 0x3ff51fffcc027f0f, 0x3c9883d8847754dc,
    0x3fe7f405ffc61022, 0x3fdabe186ed3d000, 0x3ff55ffffd87b36f, 0xbc8709e731d02807,
    0x3fe7ad22181415ca, 0x3fdbd0f2aea0e000, 0x3ff59ffff21df7ba, 0x3c87f79f68727b02,
    0x3fe767dcf99eff8c, 0x3fdce0a43dbf4000, 0x3ff5dfffebfc3481, 0xbc9180902e30e93e,
];
static POW_LOG_K: [u64; 9] = [
    0x3fe62e42fefa3800, 0x3d2ef35793c76730, 0xbfe0000000000000, 0xbfe5555555555560,
    0x3fe0000000000006, 0x3fe999999959554e, 0xbfe555555529a47a, 0xbff2495b9b4845e9,
    0x3ff0002b8b263fc3,
];
static POW_LOG_TAB: [u64; 384] = [
    0x3ff6a00000000000, 0xbfd62c82f2b9c800, 0x3cfab42428375680, 0x3ff6800000000000,
    0xbfd5d1bdbf580800, 0xbd1ca508d8e0f720, 0x3ff6600000000000, 0xbfd5767717455800,
    0xbd2362a4d5b6506d, 0x3ff6400000000000, 0xbfd51aad872df800, 0xbce684e49eb067d5,
    0x3ff6200000000000, 0xbfd4be5f95777800, 0xbd041b6993293ee0, 0x3ff6000000000000,
    0xbfd4618bc21c6000, 0x3d13d82f484c84cc, 0x3ff5e00000000000, 0xbfd404308686a800,
    0x3cdc42f3ed820b3a, 0x3ff5c00000000000, 0xbfd3a64c55694800, 0x3d20b1c686519460,
    0x3ff5a00000000000, 0xbfd347dd9a988000, 0x3d25594dd4c58092, 0x3ff5800000000000,
    0xbfd2e8e2bae12000, 0x3d267b1e99b72bd8, 0x3ff5600000000000, 0xbfd2895a13de8800,
    0x3d15ca14b6cfb03f, 0x3ff5600000000000, 0xbfd2895a13de8800, 0x3d15ca14b6cfb03f,
    0x3ff5400000000000, 0xbfd22941fbcf7800, 0xbd165a242853da76, 0x3ff5200000000000,
    0xbfd1c898c1699800, 0xbd1fafbc68e75404, 0x3ff5000000000000, 0xbfd1675cababa800,
    0x3d1f1fc63382a8f0, 0x3ff4e00000000000, 0xbfd1058bf9ae4800, 0xbd26a8c4fd055a66,
    0x3ff4c00000000000, 0xbfd0a324e2739000, 0xbd0c6bee7ef4030e, 0x3ff4a00000000000,
    0xbfd0402594b4d000, 0xbcf036b89ef42d7f, 0x3ff4a00000000000, 0xbfd0402594b4d000,
    0xbcf036b89ef42d7f, 0x3ff4800000000000, 0xbfcfb9186d5e4000, 0x3d0d572aab993c87,
    0x3ff4600000000000, 0xbfcef0adcbdc6000, 0x3d2b26b79c86af24, 0x3ff4400000000000,
    0xbfce27076e2af000, 0xbd172f4f543fff10, 0x3ff4200000000000, 0xbfcd5c216b4fc000,
    0x3d21ba91bbca681b, 0x3ff4000000000000, 0xbfcc8ff7c79aa000, 0x3d27794f689f8434,
    0x3ff4000000000000, 0xbfcc8ff7c79aa000, 0x3d27794f689f8434, 0x3ff3e00000000000,
    0xbfcbc286742d9000, 0x3d194eb0318bb78f, 0x3ff3c00000000000, 0xbfcaf3c94e80c000,
    0x3cba4e633fcd9066, 0x3ff3a00000000000, 0xbfca23bc1fe2b000, 0xbd258c64dc46c1ea,
    0x3ff3a00000000000, 0xbfca23bc1fe2b000, 0xbd258c64dc46c1ea, 0x3ff3800000000000,
    0xbfc9525a9cf45000, 0xbd2ad1d904c1d4e3, 0x3ff3600000000000, 0xbfc87fa06520d000,
    0x3d2bbdbf7fdbfa09, 0x3ff3400000000000, 0xbfc7ab890210e000, 0x3d2bdb9072534a58,
    0x3ff3400000000000, 0xbfc7ab890210e000, 0x3d2bdb9072534a58, 0x3ff3200000000000,
    0xbfc6d60fe719d000, 0xbd10e46aa3b2e266, 0x3ff3000000000000, 0xbfc5ff3070a79000,
    0xbd1e9e439f105039, 0x3ff3000000000000, 0xbfc5ff3070a79000, 0xbd1e9e439f105039,
    0x3ff2e00000000000, 0xbfc526e5e3a1b000, 0xbd20de8b90075b8f, 0x3ff2c00000000000,
    0xbfc44d2b6ccb8000, 0x3d170cc16135783c, 0x3ff2c00000000000, 0xbfc44d2b6ccb8000,
    0x3d170cc16135783c, 0x3ff2a00000000000, 0xbfc371fc201e9000, 0x3cf178864d27543a,
    0x3ff2800000000000, 0xbfc29552f81ff000, 0xbd248d301771c408, 0x3ff2600000000000,
    0xbfc1b72ad52f6000, 0xbd2e80a41811a396, 0x3ff2600000000000, 0xbfc1b72ad52f6000,
    0xbd2e80a41811a396, 0x3ff2400000000000, 0xbfc0d77e7cd09000, 0x3d0a699688e85bf4,
    0x3ff2400000000000, 0xbfc0d77e7cd09000, 0x3d0a699688e85bf4, 0x3ff2200000000000,
    0xbfbfec9131dbe000, 0xbd2575545ca333f2, 0x3ff2000000000000, 0xbfbe27076e2b0000,
    0x3d2a342c2af0003c, 0x3ff2000000000000, 0xbfbe27076e2b0000, 0x3d2a342c2af0003c,
    0x3ff1e00000000000, 0xbfbc5e548f5bc000, 0xbd1d0c57585fbe06, 0x3ff1c00000000000,
    0xbfba926d3a4ae000, 0x3d253935e85baac8, 0x3ff1c00000000000, 0xbfba926d3a4ae000,
    0x3d253935e85baac8, 0x3ff1a00000000000, 0xbfb8c345d631a000, 0x3d137c294d2f5668,
    0x3ff1a00000000000, 0xbfb8c345d631a000, 0x3d137c294d2f5668, 0x3ff1800000000000,
    0xbfb6f0d28ae56000, 0xbd269737c93373da, 0x3ff1600000000000, 0xbfb51b073f062000,
    0x3d1f025b61c65e57, 0x3ff1600000000000, 0xbfb51b073f062000, 0x3d1f025b61c65e57,
    0x3ff1400000000000, 0xbfb341d7961be000, 0x3d2c5edaccf913df, 0x3ff1400000000000,
    0xbfb341d7961be000, 0x3d2c5edaccf913df, 0x3ff1200000000000, 0xbfb16536eea38000,
    0x3d147c5e768fa309, 0x3ff1000000000000, 0xbfaf0a30c0118000, 0x3d2d599e83368e91,
    0x3ff1000000000000, 0xbfaf0a30c0118000, 0x3d2d599e83368e91, 0x3ff0e00000000000,
    0xbfab42dd71198000, 0x3d1c827ae5d6704c, 0x3ff0e00000000000, 0xbfab42dd71198000,
    0x3d1c827ae5d6704c, 0x3ff0c00000000000, 0xbfa77458f632c000, 0xbd2cfc4634f2a1ee,
    0x3ff0c00000000000, 0xbfa77458f632c000, 0xbd2cfc4634f2a1ee, 0x3ff0a00000000000,
    0xbfa39e87b9fec000, 0x3cf502b7f526feaa, 0x3ff0a00000000000, 0xbfa39e87b9fec000,
    0x3cf502b7f526feaa, 0x3ff0800000000000, 0xbf9f829b0e780000, 0xbd2980267c7e09e4,
    0x3ff0800000000000, 0xbf9f829b0e780000, 0xbd2980267c7e09e4, 0x3ff0600000000000,
    0xbf97b91b07d58000, 0xbd288d5493faa639, 0x3ff0400000000000, 0xbf8fc0a8b0fc0000,
    0xbcdf1e7cf6d3a69c, 0x3ff0400000000000, 0xbf8fc0a8b0fc0000, 0xbcdf1e7cf6d3a69c,
    0x3ff0200000000000, 0xbf7fe02a6b100000, 0xbd19e23f0dda40e4, 0x3ff0200000000000,
    0xbf7fe02a6b100000, 0xbd19e23f0dda40e4, 0x3ff0000000000000, 0x0000000000000000,
    0x0000000000000000, 0x3ff0000000000000, 0x0000000000000000, 0x0000000000000000,
    0x3fefc00000000000, 0x3f80101575890000, 0xbd10c76b999d2be8, 0x3fef800000000000,
    0x3f90205658938000, 0xbd23dc5b06e2f7d2, 0x3fef400000000000, 0x3f98492528c90000,
    0xbd2aa0ba325a0c34, 0x3fef000000000000, 0x3fa0415d89e74000, 0x3d0111c05cf1d753,
    0x3feec00000000000, 0x3fa466aed42e0000, 0xbd2c167375bdfd28, 0x3fee800000000000,
    0x3fa894aa149fc000, 0xbd197995d05a267d, 0x3fee400000000000, 0x3faccb73cdddc000,
    0xbd1a68f247d82807, 0x3fee200000000000, 0x3faeea31c006c000, 0xbd0e113e4fc93b7b,
    0x3fede00000000000, 0x3fb1973bd1466000, 0xbd25325d560d9e9b, 0x3feda00000000000,
    0x3fb3bdf5a7d1e000, 0x3d2cc85ea5db4ed7, 0x3fed600000000000, 0x3fb5e95a4d97a000,
    0xbd2c69063c5d1d1e, 0x3fed400000000000, 0x3fb700d30aeac000, 0x3cec1e8da99ded32,
    0x3fed000000000000, 0x3fb9335e5d594000, 0x3d23115c3abd47da, 0x3fecc00000000000,
    0x3fbb6ac88dad6000, 0xbd1390802bf768e5, 0x3feca00000000000, 0x3fbc885801bc4000,
    0x3d2646d1c65aacd3, 0x3fec600000000000, 0x3fbec739830a2000, 0xbd2dc068afe645e0,
    0x3fec400000000000, 0x3fbfe89139dbe000, 0xbd2534d64fa10afd, 0x3fec000000000000,
    0x3fc1178e8227e000, 0x3d21ef78ce2d07f2, 0x3febe00000000000, 0x3fc1aa2b7e23f000,
    0x3d2ca78e44389934, 0x3feba00000000000, 0x3fc2d1610c868000, 0x3d039d6ccb81b4a1,
    0x3feb800000000000, 0x3fc365fcb0159000, 0x3cc62fa8234b7289, 0x3feb400000000000,
    0x3fc4913d8333b000, 0x3d25837954fdb678, 0x3feb200000000000, 0x3fc527e5e4a1b000,
    0x3d2633e8e5697dc7, 0x3feae00000000000, 0x3fc6574ebe8c1000, 0x3d19cf8b2c3c2e78,
    0x3feac00000000000, 0x3fc6f0128b757000, 0xbd25118de59c21e1, 0x3feaa00000000000,
    0x3fc7898d85445000, 0xbd1c661070914305, 0x3fea600000000000, 0x3fc8beafeb390000,
    0xbd073d54aae92cd1, 0x3fea400000000000, 0x3fc95a5adcf70000, 0x3d07f22858a0ff6f,
    0x3fea000000000000, 0x3fca93ed3c8ae000, 0xbd28724350562169, 0x3fe9e00000000000,
    0x3fcb31d8575bd000, 0xbd0c358d4eace1aa, 0x3fe9c00000000000, 0x3fcbd087383be000,
    0xbd2d4bc4595412b6, 0x3fe9a00000000000, 0x3fcc6ffbc6f01000, 0xbcf1ec72c5962bd2,
    0x3fe9600000000000, 0x3fcdb13db0d49000, 0xbd2aff2af715b035, 0x3fe9400000000000,
    0x3fce530effe71000, 0x3cc212276041f430, 0x3fe9200000000000, 0x3fcef5ade4dd0000,
    0xbcca211565bb8e11, 0x3fe9000000000000, 0x3fcf991c6cb3b000, 0x3d1bcbecca0cdf30,
    0x3fe8c00000000000, 0x3fd07138604d5800, 0x3cf89cdb16ed4e91, 0x3fe8a00000000000,
    0x3fd0c42d67616000, 0x3d27188b163ceae9, 0x3fe8800000000000, 0x3fd1178e8227e800,
    0xbd2c210e63a5f01c, 0x3fe8600000000000, 0x3fd16b5ccbacf800, 0x3d2b9acdf7a51681,
    0x3fe8400000000000, 0x3fd1bf99635a6800, 0x3d2ca6ed5147bdb7, 0x3fe8200000000000,
    0x3fd214456d0eb800, 0x3d0a87deba46baea, 0x3fe7e00000000000, 0x3fd2bef07cdc9000,
    0x3d2a9cfa4a5004f4, 0x3fe7c00000000000, 0x3fd314f1e1d36000, 0xbd28e27ad3213cb8,
    0x3fe7a00000000000, 0x3fd36b6776be1000, 0x3d116ecdb0f177c8, 0x3fe7800000000000,
    0x3fd3c25277333000, 0x3d183b54b606bd5c, 0x3fe7600000000000, 0x3fd419b423d5e800,
    0x3d08e436ec90e09d, 0x3fe7400000000000, 0x3fd4718dc271c800, 0xbd2f27ce0967d675,
    0x3fe7200000000000, 0x3fd4c9e09e173000, 0xbd2e20891b0ad8a4, 0x3fe7000000000000,
    0x3fd522ae0738a000, 0x3d2ebe708164c759, 0x3fe6e00000000000, 0x3fd57bf753c8d000,
    0x3d1fadedee5d40ef, 0x3fe6c00000000000, 0x3fd5d5bddf596000, 0xbd0a0b2a08a465dc,
];
static EXP2F_K: [u64; 9] = [
    0x42e8000000000000, 0x3fac6af84b912394, 0x3fcebfce50fac4f3, 0x3fe62e42ff0c52d6,
    0x4338000000000000, 0x40471547652b82fe, 0x3ebc6af84b912394, 0x3f2ebfce50fac4f3,
    0x3f962e42ff0c52d6,
];
static EXP2F_TAB: [u64; 32] = [
    0x3ff0000000000000, 0x3fefd9b0d3158574, 0x3fefb5586cf9890f, 0x3fef9301d0125b51,
    0x3fef72b83c7d517b, 0x3fef54873168b9aa, 0x3fef387a6e756238, 0x3fef1e9df51fdee1,
    0x3fef06fe0a31b715, 0x3feef1a7373aa9cb, 0x3feedea64c123422, 0x3feece086061892d,
    0x3feebfdad5362a27, 0x3feeb42b569d4f82, 0x3feeab07dd485429, 0x3feea47eb03a5585,
    0x3feea09e667f3bcd, 0x3fee9f75e8ec5f74, 0x3feea11473eb0187, 0x3feea589994cce13,
    0x3feeace5422aa0db, 0x3feeb737b0cdc5e5, 0x3feec49182a3f090, 0x3feed503b23e255d,
    0x3feee89f995ad3ad, 0x3feeff76f2fb5e47, 0x3fef199bdd85529c, 0x3fef3720dcef9069,
    0x3fef5818dcfba487, 0x3fef7c97337b9b5f, 0x3fefa4afa2a490da, 0x3fefd0765b6e4540,
];
static LOGF_K: [u64; 4] = [
    0x3fe62e42fefa39ef, 0xbfd00ea348b88334, 0x3fd5575b0be00b6a, 0xbfdffffef20a4123,
];
static LOGF_TAB: [u64; 32] = [
    0x3ff661ec79f8f3be, 0xbfd57bf7808caade, 0x3ff571ed4aaf883d, 0xbfd2bef0a7c06ddb,
    0x3ff49539f0f010b0, 0xbfd01eae7f513a67, 0x3ff3c995b0b80385, 0xbfcb31d8a68224e9,
    0x3ff30d190c8864a5, 0xbfc6574f0ac07758, 0x3ff25e227b0b8ea0, 0xbfc1aa2bc79c8100,
    0x3ff1bb4a4a1a343f, 0xbfba4e76ce8c0e5e, 0x3ff12358f08ae5ba, 0xbfb1973c5a611ccc,
    0x3ff0953f419900a7, 0xbfa252f438e10c1e, 0x3ff0000000000000, 0x0000000000000000,
    0x3fee608cfd9a47ac, 0x3faaa5aa5df25984, 0x3feca4b31f026aa0, 0x3fbc5e53aa362eb4,
    0x3feb2036576afce6, 0x3fc526e57720db08, 0x3fe9c2d163a1aa2d, 0x3fcbc2860d224770,
    0x3fe886e6037841ed, 0x3fd1058bc8a07ee1, 0x3fe767dcf5534862, 0x3fd4043057b6ee09,
];
static LOG2F_K: [u64; 4] = [
    0xbfd712b6f70a7e4d, 0x3fdecabf496832e0, 0xbfe715479ffae3de, 0x3ff715475f35c8b8,
];
static LOG2F_TAB: [u64; 32] = [
    0x3ff661ec79f8f3be, 0xbfdefec65b963019, 0x3ff571ed4aaf883d, 0xbfdb0b6832d4fca4,
    0x3ff49539f0f010b0, 0xbfd7418b0a1fb77b, 0x3ff3c995b0b80385, 0xbfd39de91a6dcf7b,
    0x3ff30d190c8864a5, 0xbfd01d9bf3f2b631, 0x3ff25e227b0b8ea0, 0xbfc97c1d1b3b7af0,
    0x3ff1bb4a4a1a343f, 0xbfc2f9e393af3c9f, 0x3ff12358f08ae5ba, 0xbfb960cbbf788d5c,
    0x3ff0953f419900a7, 0xbfaa6f9db6475fce, 0x3ff0000000000000, 0x0000000000000000,
    0x3fee608cfd9a47ac, 0x3fb338ca9f24f53d, 0x3feca4b31f026aa0, 0x3fc476a9543891ba,
    0x3feb2036576afce6, 0x3fce840b4ac4e4d2, 0x3fe9c2d163a1aa2d, 0x3fd40645f0c6651c,
    0x3fe886e6037841ed, 0x3fd88e9c2c1b9ff8, 0x3fe767dcf5534862, 0x3fdce0a44eb17bcc,
];
static POWF_LOG2_K: [u64; 5] = [
    0x3fd27616c9496e0b, 0xbfd71969a075c67a, 0x3fdec70a6ca7badd, 0xbfe7154748bef6c8,
    0x3ff71547652ab82b,
];
static POWF_LOG2_TAB: [u64; 32] = [
    0x3ff661ec79f8f3be, 0xbfdefec65b963019, 0x3ff571ed4aaf883d, 0xbfdb0b6832d4fca4,
    0x3ff49539f0f010b0, 0xbfd7418b0a1fb77b, 0x3ff3c995b0b80385, 0xbfd39de91a6dcf7b,
    0x3ff30d190c8864a5, 0xbfd01d9bf3f2b631, 0x3ff25e227b0b8ea0, 0xbfc97c1d1b3b7af0,
    0x3ff1bb4a4a1a343f, 0xbfc2f9e393af3c9f, 0x3ff12358f08ae5ba, 0xbfb960cbbf788d5c,
    0x3ff0953f419900a7, 0xbfaa6f9db6475fce, 0x3ff0000000000000, 0x0000000000000000,
    0x3fee608cfd9a47ac, 0x3fb338ca9f24f53d, 0x3feca4b31f026aa0, 0x3fc476a9543891ba,
    0x3feb2036576afce6, 0x3fce840b4ac4e4d2, 0x3fe9c2d163a1aa2d, 0x3fd40645f0c6651c,
    0x3fe886e6037841ed, 0x3fd88e9c2c1b9ff8, 0x3fe767dcf5534862, 0x3fdce0a44eb17bcc,
];
static SINCOSF: [u64; 28] = [
    0x3ff0000000000000, 0xbff0000000000000, 0xbff0000000000000, 0x3ff0000000000000,
    0x41645f306dc9c883, 0x3ff921fb54442d18, 0x3ff0000000000000, 0xbfdffffffd0c621c,
    0x3fa55553e1068f19, 0xbf56c087e89a359d, 0x3ef99343027bf8c3, 0xbfc555545995a603,
    0x3f81107605230bc4, 0xbf2994eb3774cf24, 0x3ff0000000000000, 0xbff0000000000000,
    0xbff0000000000000, 0x3ff0000000000000, 0x41645f306dc9c883, 0x3ff921fb54442d18,
    0xbff0000000000000, 0x3fdffffffd0c621c, 0xbfa55553e1068f19, 0x3f56c087e89a359d,
    0xbef99343027bf8c3, 0xbfc555545995a603, 0x3f81107605230bc4, 0xbf2994eb3774cf24,
];
static INV_PIO4: [u64; 24] = [
    0x00000000000000a2, 0x000000000000a2f9, 0x0000000000a2f983, 0x00000000a2f9836e,
    0x00000000f9836e4e, 0x00000000836e4e44, 0x000000006e4e4415, 0x000000004e441529,
    0x00000000441529fc, 0x000000001529fc27, 0x0000000029fc2757, 0x00000000fc2757d1,
    0x000000002757d1f5, 0x0000000057d1f534, 0x00000000d1f534dd, 0x00000000f534ddc0,
    0x0000000034ddc0db, 0x00000000ddc0db62, 0x00000000c0db6295, 0x00000000db629599,
    0x000000006295993c, 0x0000000095993c43, 0x00000000993c4390, 0x000000003c439041,
];
