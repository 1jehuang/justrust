//! encoding_rs asks whether any byte of a block has its top bit set by folding the block's
//! words together: `_mm_movemask_epi8` is not the only way a crate reduces a vector, and
//! `simd_reduce_or` is what `core::arch` makes of the AVX-512 `_mm512_reduce_or_epi64`.

#[cfg(target_arch = "x86_64")]
pub fn run() -> (i64, i64) {
    use std::arch::x86_64::{_mm512_reduce_and_epi64, _mm512_reduce_or_epi64, _mm512_set_epi64};
    if !is_x86_feature_detected!("avx512f") {
        return (0xff, 0);
    }
    #[target_feature(enable = "avx512f")]
    fn folded() -> (i64, i64) {
        let words = _mm512_set_epi64(1, 2, 4, 8, 16, 32, 64, 128);
        (_mm512_reduce_or_epi64(words), _mm512_reduce_and_epi64(words))
    }
    unsafe { folded() }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> (i64, i64) {
    (0xff, 0)
}
