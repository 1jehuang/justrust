//! `_mm_clmulepi64_si128` is LLVM's `x86.pclmulqdq`, with no body in `core::arch` and no
//! instruction in Cranelift: crc32fast's and simd-json's functions that reach it were left to
//! rustc. It is multiplied out in software now.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "pclmulqdq")]
fn products(a: __m128i, b: __m128i) -> [[u64; 2]; 4] {
    unsafe {
        [
            std::mem::transmute(_mm_clmulepi64_si128::<0x00>(a, b)),
            std::mem::transmute(_mm_clmulepi64_si128::<0x10>(a, b)),
            std::mem::transmute(_mm_clmulepi64_si128::<0x01>(a, b)),
            std::mem::transmute(_mm_clmulepi64_si128::<0x11>(a, b)),
        ]
    }
}

#[cfg(target_arch = "x86_64")]
pub fn run() -> Vec<[[u64; 2]; 4]> {
    if !is_x86_feature_detected!("pclmulqdq") {
        return Vec::new();
    }
    unsafe {
        let a = _mm_set_epi64x(0x7b5b546573745665, 0x63746f725d53475d);
        let b = _mm_set_epi64x(0x4869285368617929, 0x5b477565726f6e5d);
        let ones = _mm_set1_epi8(-1);
        vec![products(a, b), products(b, ones), products(ones, ones), products(a, _mm_setzero_si128())]
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> Vec<[[u64; 2]; 4]> {
    Vec::new()
}
