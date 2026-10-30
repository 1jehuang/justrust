//! `core::arch` of nightly-2026-10-25 reaches `_mm_min_ps`, `_mm_max_ps`, `_mm_round_ps`,
//! `_mm_cvttps_epi32` and `_mm256_srl_epi16` through `llvm.x86.sse.min.ps`, `llvm.x86.sse.max.ps`,
//! `llvm.x86.sse41.round.ps`, `llvm.x86.sse2.cvttps2dq` and `llvm.x86.avx2.psrl.w`, none with a
//! body: a program that links `fearless_simd` had its AVX2 and SSE4.2 levels refused. NaNs and
//! values that do not fit are in the operands, where the instructions differ from the generic
//! operations.

#[cfg(target_arch = "x86_64")]
pub fn run() -> ([f32; 4], [f32; 4], [f32; 4], [f32; 4], [i32; 4], [u16; 4]) {
    use std::arch::x86_64::*;
    #[target_feature(enable = "avx2,sse4.1")]
    fn lanes() -> ([f32; 4], [f32; 4], [f32; 4], [f32; 4], [i32; 4], [u16; 4]) {
        unsafe {
            let a = _mm_set_ps(1.0, -2.5, 3.5, f32::NAN);
            let b = _mm_set_ps(0.5, -1.0, 4.0, 2.0);
            let mut min = [0.0; 4];
            let mut max = [0.0; 4];
            let mut nearest = [0.0; 4];
            let mut floor = [0.0; 4];
            _mm_storeu_ps(min.as_mut_ptr(), _mm_min_ps(a, b));
            _mm_storeu_ps(max.as_mut_ptr(), _mm_max_ps(a, b));
            let halves = _mm_set_ps(1.5, 2.5, -0.5, 0.4);
            _mm_storeu_ps(
                nearest.as_mut_ptr(),
                _mm_round_ps::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(halves),
            );
            _mm_storeu_ps(floor.as_mut_ptr(), _mm_floor_ps(_mm_set_ps(1.5, -2.5, -0.5, 0.4)));
            let mut truncated = [0; 4];
            let wide = _mm_set_ps(1e10, -2.7, 3.9, f32::NAN);
            _mm_storeu_si128(truncated.as_mut_ptr().cast(), _mm_cvttps_epi32(wide));
            let mut shifted = [0u16; 16];
            let words = _mm256_set1_epi16(0x8000u16 as i16);
            let (by_three, by_sixteen) =
                (_mm256_srl_epi16(words, _mm_cvtsi32_si128(3)), _mm256_srl_epi16(words, _mm_cvtsi32_si128(16)));
            _mm256_storeu_si256(shifted.as_mut_ptr().cast(), by_three);
            let (three, _) = (shifted[0], shifted[15]);
            _mm256_storeu_si256(shifted.as_mut_ptr().cast(), by_sixteen);
            (min, max, nearest, floor, truncated, [three, shifted[0], 0, 0])
        }
    }
    if !is_x86_feature_detected!("avx2") || !is_x86_feature_detected!("sse4.1") {
        return ([0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4], [0; 4], [0; 4]);
    }
    unsafe { lanes() }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> ([f32; 4], [f32; 4], [f32; 4], [f32; 4], [i32; 4], [u16; 4]) {
    ([0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4], [0; 4], [0; 4])
}
