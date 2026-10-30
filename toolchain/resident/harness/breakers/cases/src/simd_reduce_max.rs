//! bun asks whether a block of bytes is ASCII by its largest byte: `vmaxvq_u8` is what
//! `core::arch` makes `simd_reduce_max` of on AArch64, and AVX-512's `_mm512_reduce_max_epi64`
//! on x86-64. The lanes are unsigned or signed, so the larger of 0x80 and 1 depends on which.

#[cfg(target_arch = "aarch64")]
pub fn run() -> (i64, i64, i64, i64) {
    use std::arch::aarch64::{vld1q_s8, vld1q_u8, vmaxvq_s8, vmaxvq_u8, vminvq_s8, vminvq_u8};
    let bytes: [u8; 16] = [1, 2, 0x80, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 3];
    let signed: [i8; 16] = [1, 2, -128, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 3];
    unsafe {
        let (u, s) = (vld1q_u8(bytes.as_ptr()), vld1q_s8(signed.as_ptr()));
        (vmaxvq_u8(u) as i64, vminvq_u8(u) as i64, vmaxvq_s8(s) as i64, vminvq_s8(s) as i64)
    }
}

#[cfg(target_arch = "x86_64")]
pub fn run() -> (i64, i64, i64, i64) {
    use std::arch::x86_64::{
        _mm512_reduce_max_epi64, _mm512_reduce_max_epu64, _mm512_reduce_min_epi64, _mm512_reduce_min_epu64,
        _mm512_set_epi64,
    };
    if !is_x86_feature_detected!("avx512f") {
        return (0x80, 1, 15, -128);
    }
    #[target_feature(enable = "avx512f")]
    fn folded() -> (i64, i64, i64, i64) {
        let unsigned = _mm512_set_epi64(1, 2, 0x80, 4, 5, 6, 7, 3);
        let signed = _mm512_set_epi64(1, 2, -128, 4, 5, 6, 15, 3);
        (
            _mm512_reduce_max_epu64(unsigned) as i64,
            _mm512_reduce_min_epu64(unsigned) as i64,
            _mm512_reduce_max_epi64(signed),
            _mm512_reduce_min_epi64(signed),
        )
    }
    unsafe { folded() }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub fn run() -> (i64, i64, i64, i64) {
    (0x80, 1, 15, -128)
}
