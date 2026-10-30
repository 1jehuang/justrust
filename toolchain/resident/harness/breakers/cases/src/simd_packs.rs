//! simd-json packs the digits of a number with `_mm_packus_epi32`: the pack intrinsics
//! (`llvm.x86.sse2.packsswb.128`, `packuswb`, `packssdw`, `llvm.x86.sse41.packusdw`)
//! saturate each lane to half its width and have no body.

#[cfg(target_arch = "x86_64")]
pub fn run() -> [[u8; 16]; 4] {
    use std::arch::x86_64::*;
    #[target_feature(enable = "sse4.1")]
    fn packs(words: [i16; 8], more: [i16; 8]) -> [[u8; 16]; 4] {
        unsafe {
            let a = _mm_loadu_si128(words.as_ptr().cast());
            let b = _mm_loadu_si128(more.as_ptr().cast());
            [
                std::mem::transmute(_mm_packs_epi16(a, b)),
                std::mem::transmute(_mm_packus_epi16(a, b)),
                std::mem::transmute(_mm_packs_epi32(a, b)),
                std::mem::transmute(_mm_packus_epi32(a, b)),
            ]
        }
    }
    if !is_x86_feature_detected!("sse4.1") {
        return [[0; 16]; 4];
    }
    let words = [0, 127, 128, -128, -129, 255, 256, i16::MAX];
    let more = [i16::MIN, -1, 1, 300, -300, 1, 0, -2];
    unsafe { packs(words, more) }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> [[u8; 16]; 4] {
    [[0; 16]; 4]
}
