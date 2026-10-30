//! `_mm{,256,512}_madd52{lo,hi}_epu64` are LLVM's `x86.avx512.vpmadd52{l,h}.uq.{128,256,512}`,
//! with no body in `core::arch`: curve25519-dalek's IFMA backend reaches them (through rustls,
//! so blaze's own build), and the whole build of blaze refused. Worked out lane by lane.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512ifma,avx512vl")]
fn products(a: [u64; 8], b: [u64; 8], c: [u64; 8]) -> Vec<u64> {
    unsafe {
        let [a, b, c]: [__m512i; 3] = std::mem::transmute([a, b, c]);
        let [a2, b2, c2] = [a, b, c].map(|it| _mm512_castsi512_si256(it));
        let [a1, b1, c1] = [a, b, c].map(|it| _mm512_castsi512_si128(it));
        let wide: [[u64; 8]; 2] = std::mem::transmute([
            _mm512_madd52lo_epu64(a, b, c),
            _mm512_madd52hi_epu64(a, b, c),
        ]);
        let half: [[u64; 4]; 2] = std::mem::transmute([
            _mm256_madd52lo_epu64(a2, b2, c2),
            _mm256_madd52hi_epu64(a2, b2, c2),
        ]);
        let quarter: [[u64; 2]; 2] =
            std::mem::transmute([_mm_madd52lo_epu64(a1, b1, c1), _mm_madd52hi_epu64(a1, b1, c1)]);
        wide.iter().flatten().chain(half.iter().flatten()).chain(quarter.iter().flatten()).copied().collect()
    }
}

#[cfg(target_arch = "x86_64")]
pub fn run() -> Vec<u64> {
    if !is_x86_feature_detected!("avx512ifma") || !is_x86_feature_detected!("avx512vl") {
        return Vec::new();
    }
    let lanes = |seed: u64| std::array::from_fn(|it| seed.wrapping_mul(it as u64 * 2 + 1).rotate_left(it as u32 * 7));
    let mut b = lanes(0x8f0f_8e1e_0c3c_7878);
    // The largest factors there are, and the sum wrapping.
    b[0] = u64::MAX;
    let mut c = lanes(0x5555_aaaa_3333_cccc);
    c[0] = u64::MAX;
    let mut a = lanes(0x0123_4567_89ab_cdef);
    a[0] = u64::MAX;
    unsafe { products(a, b, c) }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> Vec<u64> {
    Vec::new()
}
