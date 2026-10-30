//! half converts `f32` to `f16` with `_mm_cvtps_ph`, which is `llvm.x86.vcvtps2ph.128`: an
//! instruction Cranelift has no name for. It is a function of that instruction, as `xgetbv` is.

#[cfg(target_arch = "x86_64")]
pub fn run() -> [[u16; 8]; 5] {
    use std::arch::x86_64::*;
    if !is_x86_feature_detected!("f16c") || !is_x86_feature_detected!("avx") {
        return [[0; 8]; 5];
    }
    #[target_feature(enable = "f16c,avx")]
    unsafe fn halves(of: [f32; 8]) -> [[u16; 8]; 5] {
        unsafe {
            let (four, eight) = (_mm_loadu_ps(of.as_ptr()), _mm256_loadu_ps(of.as_ptr()));
            let all = [
                _mm_cvtps_ph::<_MM_FROUND_TO_NEAREST_INT>(four),
                _mm_cvtps_ph::<_MM_FROUND_TO_ZERO>(four),
                _mm256_cvtps_ph::<_MM_FROUND_TO_NEAREST_INT>(eight),
                _mm256_cvtps_ph::<_MM_FROUND_TO_NEG_INF>(eight),
                _mm256_cvtps_ph::<_MM_FROUND_TO_POS_INF>(eight),
            ];
            all.map(|halves| {
                let mut out = [0u16; 8];
                _mm_storeu_si128(out.as_mut_ptr().cast(), halves);
                out
            })
        }
    }
    unsafe { halves([1.0, -2.5, 65504.0, 1e-7, 0.1, -0.3, 1e9, 3.14159]) }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> [[u16; 8]; 5] {
    [[0; 8]; 5]
}
