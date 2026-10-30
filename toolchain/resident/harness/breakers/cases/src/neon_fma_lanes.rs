//! The corpus's `fma_nans` turned MISMATCH on AArch64 once another case joined the corpus
//! (#580), which moved `vfmaq_laneq_f64::<1>` and its kin out of what rustc's objects export,
//! so ra-jit built them. rustc builds core's optimized MIR of each NEON fused multiply-add,
//! with `vfmaq_f64`, `simd_neg` and the splat of `c` built in, and LLVM chooses the
//! instruction by that shape: `vfmaq_laneq_f64` is `fmla a, b, c.d[k]` (b first), `vfmsq_f64`
//! is `fmls a, c, b` (which negates c, not b), `vfmsd_laneq_f64::<0>` is `fmsub` with c first.
//! ra-jit built each of its calls, which keeps the other NaN of two, or flips the sign of the
//! other one. Each form here on numbers, quiet and signalling NaNs, b's negative.
//!
//! The harness builds with `-C link-dead-code`, which keeps the `#[inline(always)]` splat of
//! the `_n`, `_lane` and `_laneq` vector forms in one codegen unit of rustc's choosing: LLVM
//! sees it only there, so ra-jit leaves those forms to rustc, and `splatted` with them.

#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;
#[cfg(target_arch = "aarch64")]
use std::mem::transmute;

#[cfg(target_arch = "aarch64")]
pub fn run() -> Vec<u64> {
    const SINGLES: [[u32; 3]; 3] = [
        [0x3fc0_0000, 0x7fc0_0001, 0x7f80_0011],
        [0xc020_0000, 0xffc0_0002, 0xff80_0022],
        [0x4080_0000, 0x7fc0_0003, 0x7f80_0033],
    ];
    const DOUBLES: [[u64; 3]; 3] = [
        [0x3ff8_0000_0000_0000, 0x7ff8_0000_0000_0001, 0x7ff0_0000_0000_0011],
        [0xc004_0000_0000_0000, 0xfff8_0000_0000_0002, 0xfff0_0000_0000_0022],
        [0x4010_0000_0000_0000, 0x7ff8_0000_0000_0003, 0x7ff0_0000_0000_0033],
    ];
    let mut out = Vec::new();
    for i in 0..27 {
        let pick = [i % 3, i / 3 % 3, i / 9];
        let [a, b, c] = [0, 1, 2].map(|k| f32::from_bits(SINGLES[k][pick[k]]));
        let [x, y, z] = [0, 1, 2].map(|k| f64::from_bits(DOUBLES[k][pick[k]]));
        out.extend(direct(a, b, c, x, y, z));
        out.extend(splatted(a, b, c, x, y, z));
    }
    out
}

/// `it` in lane `at` and 1.0 in the others, so a lane of `c` other than the one taken has no
/// NaN.
#[cfg(target_arch = "aarch64")]
fn q(it: f32, at: usize) -> float32x4_t {
    let mut lanes = [1.0f32; 4];
    lanes[at] = it;
    unsafe { transmute::<[f32; 4], float32x4_t>(lanes) }
}

#[cfg(target_arch = "aarch64")]
fn d(it: f32, at: usize) -> float32x2_t {
    let mut lanes = [1.0f32; 2];
    lanes[at] = it;
    unsafe { transmute::<[f32; 2], float32x2_t>(lanes) }
}

#[cfg(target_arch = "aarch64")]
fn qd(it: f64, at: usize) -> float64x2_t {
    let mut lanes = [1.0f64; 2];
    lanes[at] = it;
    unsafe { transmute::<[f64; 2], float64x2_t>(lanes) }
}

/// The forms whose bodies take `c` whole or one lane of it, which ra-jit builds.
#[cfg(target_arch = "aarch64")]
#[inline(never)]
fn direct(a: f32, b: f32, c: f32, x: f64, y: f64, z: f64) -> Vec<u64> {
    let mut out = Vec::new();
    unsafe {
        let (qa, qb, qc) = (vdupq_n_f32(a), vdupq_n_f32(b), vdupq_n_f32(c));
        let (da, db, dc) = (vdup_n_f32(a), vdup_n_f32(b), vdup_n_f32(c));
        let (qx, qy, qz) = (vdupq_n_f64(x), vdupq_n_f64(y), vdupq_n_f64(z));
        let (dx, dy, dz) = (vdup_n_f64(x), vdup_n_f64(y), vdup_n_f64(z));
        for v in [vfmaq_f32(qa, qb, qc), vfmsq_f32(qa, qb, qc)] {
            out.extend(transmute::<_, [u32; 4]>(v).map(u64::from));
        }
        for v in [vfma_f32(da, db, dc), vfms_f32(da, db, dc)] {
            out.extend(transmute::<_, [u32; 2]>(v).map(u64::from));
        }
        for v in [vfmaq_f64(qx, qy, qz), vfmsq_f64(qx, qy, qz)] {
            out.extend(transmute::<_, [u64; 2]>(v));
        }
        for v in [vfma_f64(dx, dy, dz), vfms_f64(dx, dy, dz)] {
            out.push(transmute::<_, u64>(v));
        }
        for v in [
            vfmas_lane_f32::<1>(a, b, d(c, 1)),
            vfmas_laneq_f32::<0>(a, b, q(c, 0)),
            vfmss_lane_f32::<0>(a, b, d(c, 0)),
            vfmss_laneq_f32::<0>(a, b, q(c, 0)),
            vfmss_laneq_f32::<2>(a, b, q(c, 2)),
        ] {
            out.push(u64::from(v.to_bits()));
        }
        for v in [
            vfmad_lane_f64::<0>(x, y, dz),
            vfmad_laneq_f64::<1>(x, y, qd(z, 1)),
            vfmsd_lane_f64::<0>(x, y, dz),
            vfmsd_laneq_f64::<0>(x, y, qd(z, 0)),
            vfmsd_laneq_f64::<1>(x, y, qd(z, 1)),
        ] {
            out.push(v.to_bits());
        }
    }
    out
}

/// The vector forms whose bodies splat `c`, or one lane of it.
#[cfg(target_arch = "aarch64")]
#[inline(never)]
fn splatted(a: f32, b: f32, c: f32, x: f64, y: f64, z: f64) -> Vec<u64> {
    let mut out = Vec::new();
    unsafe {
        let (qa, qb) = (vdupq_n_f32(a), vdupq_n_f32(b));
        let (da, db) = (vdup_n_f32(a), vdup_n_f32(b));
        let (qx, qy) = (vdupq_n_f64(x), vdupq_n_f64(y));
        let (dx, dy, dz) = (vdup_n_f64(x), vdup_n_f64(y), vdup_n_f64(z));
        for v in [
            vfmaq_n_f32(qa, qb, c),
            vfmsq_n_f32(qa, qb, c),
            vfmaq_laneq_f32::<0>(qa, qb, q(c, 0)),
            vfmaq_laneq_f32::<2>(qa, qb, q(c, 2)),
            vfmsq_laneq_f32::<3>(qa, qb, q(c, 3)),
            vfmaq_lane_f32::<1>(qa, qb, d(c, 1)),
            vfmsq_lane_f32::<0>(qa, qb, d(c, 0)),
        ] {
            out.extend(transmute::<_, [u32; 4]>(v).map(u64::from));
        }
        for v in [
            vfma_n_f32(da, db, c),
            vfms_n_f32(da, db, c),
            vfma_lane_f32::<1>(da, db, d(c, 1)),
            vfms_lane_f32::<0>(da, db, d(c, 0)),
            vfma_laneq_f32::<3>(da, db, q(c, 3)),
            vfms_laneq_f32::<1>(da, db, q(c, 1)),
        ] {
            out.extend(transmute::<_, [u32; 2]>(v).map(u64::from));
        }
        for v in [
            vfmaq_n_f64(qx, qy, z),
            vfmsq_n_f64(qx, qy, z),
            vfmaq_laneq_f64::<0>(qx, qy, qd(z, 0)),
            vfmaq_laneq_f64::<1>(qx, qy, qd(z, 1)),
            vfmsq_laneq_f64::<0>(qx, qy, qd(z, 0)),
            vfmsq_laneq_f64::<1>(qx, qy, qd(z, 1)),
            vfmaq_lane_f64::<0>(qx, qy, dz),
            vfmsq_lane_f64::<0>(qx, qy, dz),
        ] {
            out.extend(transmute::<_, [u64; 2]>(v));
        }
        for v in [
            vfma_n_f64(dx, dy, z),
            vfms_n_f64(dx, dy, z),
            vfma_lane_f64::<0>(dx, dy, dz),
            vfms_lane_f64::<0>(dx, dy, dz),
            vfma_laneq_f64::<0>(dx, dy, qd(z, 0)),
            vfma_laneq_f64::<1>(dx, dy, qd(z, 1)),
            vfms_laneq_f64::<0>(dx, dy, qd(z, 0)),
            vfms_laneq_f64::<1>(dx, dy, qd(z, 1)),
        ] {
            out.push(transmute::<_, u64>(v));
        }
    }
    out
}

#[cfg(not(target_arch = "aarch64"))]
pub fn run() -> Vec<u64> {
    Vec::new()
}
