//! `_mm_crc32_u8` (to `_u64`), `_mm512_ternarylogic_epi64` (and `_epi32`) and
//! `_mm512_shuffle_epi8` are LLVM's `x86.sse42.crc32.*`, `x86.avx512.pternlog.*` and
//! `x86.avx512.pshuf.b.512` (and `_mm512_permutexvar_*`, `x86.avx512.permvar.*`), with no body in `core::arch`: crc-fast's functions that reach
//! them (aws-smithy-checksums, so blaze's own build) were left to rustc, and the whole build of
//! blaze refused. The checksum is assembled, the other two are worked out lane by lane.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
fn crcs(bytes: &[u8]) -> [u64; 4] {
    let mut sums = [0u32; 3];
    let mut wide = 0u64;
    for chunk in bytes.chunks_exact(8) {
        let word = u64::from_le_bytes(chunk.try_into().unwrap());
        sums[0] = chunk.iter().fold(sums[0], |sum, &byte| _mm_crc32_u8(sum, byte));
        sums[1] = _mm_crc32_u16(sums[1], word as u16);
        sums[2] = _mm_crc32_u32(sums[2], word as u32);
        wide = _mm_crc32_u64(wide, word);
    }
    [u64::from(sums[0]), u64::from(sums[1]), u64::from(sums[2]), wide]
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
fn wide(a: [u64; 8], b: [u64; 8], c: [u64; 8]) -> Vec<[u64; 8]> {
    unsafe {
        let [a, b, c]: [__m512i; 3] = std::mem::transmute([a, b, c]);
        let out: [__m512i; 8] = [
            _mm512_ternarylogic_epi64::<0x96>(a, b, c),
            _mm512_ternarylogic_epi64::<0xe8>(a, b, c),
            _mm512_ternarylogic_epi64::<0x01>(a, b, c),
            _mm512_ternarylogic_epi32::<0xca>(a, b, c),
            _mm512_ternarylogic_epi32::<0x80>(a, b, c),
            _mm512_shuffle_epi8(a, b),
            _mm512_permutexvar_epi64(b, a),
            _mm512_permutexvar_epi32(c, a),
        ];
        out.iter().map(|it| std::mem::transmute(*it)).collect()
    }
}

#[cfg(target_arch = "x86_64")]
pub fn run() -> (Vec<u64>, Vec<[u64; 8]>) {
    let bytes: Vec<u8> = (0u32..64).map(|it| (it.wrapping_mul(0x9e37_79b9) >> 7) as u8).collect();
    let sums = if is_x86_feature_detected!("sse4.2") { unsafe { crcs(&bytes) }.to_vec() } else { Vec::new() };
    let lanes = |seed: u64| std::array::from_fn(|it| seed.wrapping_mul(it as u64 * 2 + 1).rotate_left(it as u32 * 7));
    let ternary = if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512bw") {
        unsafe { wide(lanes(0x0123_4567_89ab_cdef), lanes(0x8f0f_8e1e_0c3c_7878), lanes(0x5555_aaaa_3333_cccc)) }
    } else {
        Vec::new()
    };
    (sums, ternary)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> (Vec<u64>, Vec<[u64; 8]>) {
    (Vec::new(), Vec::new())
}
