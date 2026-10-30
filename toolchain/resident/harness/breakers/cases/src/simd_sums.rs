//! simd-adler32 sums a block of bytes with `_mm_sad_epu8` and weighs it with
//! `_mm_maddubs_epi16` and `_mm_madd_epi16`: `llvm.x86.sse2.psad.bw`,
//! `llvm.x86.ssse3.pmadd.ub.sw.128` and `llvm.x86.sse2.pmadd.wd`, none with a body.

#[cfg(target_arch = "x86_64")]
pub fn run() -> ([u64; 2], [i32; 4], [i16; 8]) {
    use std::arch::x86_64::*;
    #[target_feature(enable = "ssse3")]
    fn sums(bytes: [u8; 16], weights: [i8; 16]) -> ([u64; 2], [i32; 4], [i16; 8]) {
        unsafe {
            let a = _mm_loadu_si128(bytes.as_ptr().cast());
            let b = _mm_loadu_si128(weights.as_ptr().cast());
            let apart = _mm_sad_epu8(a, b);
            let words = _mm_madd_epi16(a, b);
            let saturated = _mm_maddubs_epi16(a, b);
            (std::mem::transmute(apart), std::mem::transmute(words), std::mem::transmute(saturated))
        }
    }
    if !is_x86_feature_detected!("ssse3") {
        return ([0; 2], [0; 4], [0; 8]);
    }
    let bytes = [0, 255, 7, 200, 128, 127, 1, 99, 255, 255, 250, 3, 64, 190, 33, 254];
    let weights = [127, 127, -128, 5, -1, 100, 0, -77, 127, 127, -128, -128, 16, -3, 90, 1];
    unsafe { sums(bytes, weights) }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> ([u64; 2], [i32; 4], [i16; 8]) {
    ([0; 2], [0; 4], [0; 8])
}
