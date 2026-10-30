//! sha1 and sha2 hash with the SHA instructions where the processor has them:
//! `llvm.x86.sha1rnds4`, `sha1msg1`, `sha1msg2`, `sha1nexte`, `sha256rnds2`, `sha256msg1`
//! and `sha256msg2`, none with a body. On a processor without the instructions nothing of it
//! runs, and the case only says that the functions compile.

#[cfg(target_arch = "x86_64")]
pub fn run() -> Vec<[u32; 4]> {
    use std::arch::x86_64::*;
    #[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
    fn rounds(a: [u32; 4], b: [u32; 4], k: [u32; 4]) -> Vec<[u32; 4]> {
        unsafe {
            let a = _mm_loadu_si128(a.as_ptr().cast());
            let b = _mm_loadu_si128(b.as_ptr().cast());
            let k = _mm_loadu_si128(k.as_ptr().cast());
            let all = [
                _mm_sha1msg1_epu32(a, b),
                _mm_sha1msg2_epu32(a, b),
                _mm_sha1nexte_epu32(a, b),
                _mm_sha1rnds4_epu32::<0>(a, b),
                _mm_sha1rnds4_epu32::<1>(a, b),
                _mm_sha1rnds4_epu32::<2>(a, b),
                _mm_sha1rnds4_epu32::<3>(a, b),
                _mm_sha256msg1_epu32(a, b),
                _mm_sha256msg2_epu32(a, b),
                _mm_sha256rnds2_epu32(a, b, k),
            ];
            all.into_iter().map(|it| std::mem::transmute(it)).collect()
        }
    }
    if !is_x86_feature_detected!("sha") || !is_x86_feature_detected!("sse4.1") {
        return Vec::new();
    }
    let a = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let b = [0xc3d2_e1f0, 0x0001_0203, 0x8040_2010, 0xdead_beef];
    let k = [0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5];
    unsafe { rounds(a, b, k) }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> Vec<[u32; 4]> {
    Vec::new()
}
