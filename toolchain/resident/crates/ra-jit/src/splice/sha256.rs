//! The SHA-256 of the pages a Mach-O code signature hashes, with the processor's SHA-256
//! instructions where it has them. sha2 0.10 uses them on aarch64 only with its `asm`
//! feature, which builds C, and is about 250 MB/s without: tens of milliseconds for the
//! pages a splice of a large debug build appends. The instructions do several GB/s.

use sha2::Digest;

/// Whether [`digest`] has the processor's instructions, so that the pages are hashed on one
/// thread as fast as on several.
pub(super) fn fast() -> bool {
    #[cfg(target_arch = "aarch64")]
    if std::arch::is_aarch64_feature_detected!("sha2") {
        return true;
    }
    false
}

pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    #[cfg(target_arch = "aarch64")]
    if std::arch::is_aarch64_feature_detected!("sha2") {
        // SAFETY: the processor has the SHA-256 instructions.
        return unsafe { aarch64::digest(bytes) };
    }
    sha2::Sha256::digest(bytes).into()
}

#[cfg(target_arch = "aarch64")]
mod aarch64 {
    use std::arch::aarch64::{
        uint32x4_t, vaddq_u32, vld1q_u8, vld1q_u32, vreinterpretq_u32_u8, vrev32q_u8,
        vsha256h2q_u32, vsha256hq_u32, vsha256su0q_u32, vsha256su1q_u32, vst1q_u32,
    };

    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    /// # Safety
    ///
    /// The processor has the SHA-256 instructions.
    #[target_feature(enable = "sha2")]
    pub(super) unsafe fn digest(bytes: &[u8]) -> [u8; 32] {
        let mut state = INITIAL;
        let whole = bytes.len() / 64 * 64;
        // SAFETY: as the caller's.
        unsafe { compress(&mut state, &bytes[..whole]) };
        // The rest, the bit 1, zeros and the length in bits in one or two blocks.
        let rest = &bytes[whole..];
        let mut tail = [0u8; 128];
        tail[..rest.len()].copy_from_slice(rest);
        tail[rest.len()] = 0x80;
        let len = if rest.len() < 56 { 64 } else { 128 };
        tail[len - 8..len].copy_from_slice(&(bytes.len() as u64 * 8).to_be_bytes());
        // SAFETY: as the caller's.
        unsafe { compress(&mut state, &tail[..len]) };
        let mut out = [0u8; 32];
        for (word, at) in state.iter().zip(out.chunks_exact_mut(4)) {
            at.copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// # Safety
    ///
    /// The processor has the SHA-256 instructions.
    #[target_feature(enable = "sha2")]
    unsafe fn compress(state: &mut [u32; 8], blocks: &[u8]) {
        // SAFETY: every load and store is of 16 bytes within `state`, `blocks` and `K`.
        unsafe {
            let mut abcd = vld1q_u32(state.as_ptr());
            let mut efgh = vld1q_u32(state.as_ptr().add(4));
            for block in blocks.chunks_exact(64) {
                let (abcd_was, efgh_was) = (abcd, efgh);
                let load =
                    |at: usize| vreinterpretq_u32_u8(vrev32q_u8(vld1q_u8(block[at..].as_ptr())));
                let mut words: [uint32x4_t; 4] = [load(0), load(16), load(32), load(48)];
                for group in 0..16 {
                    let at = group % 4;
                    if group >= 4 {
                        let early = vsha256su0q_u32(words[at], words[(at + 1) % 4]);
                        words[at] =
                            vsha256su1q_u32(early, words[(at + 2) % 4], words[(at + 3) % 4]);
                    }
                    let mixed = vaddq_u32(words[at], vld1q_u32(K[group * 4..].as_ptr()));
                    let was = abcd;
                    abcd = vsha256hq_u32(abcd, efgh, mixed);
                    efgh = vsha256h2q_u32(efgh, was, mixed);
                }
                abcd = vaddq_u32(abcd, abcd_was);
                efgh = vaddq_u32(efgh, efgh_was);
            }
            vst1q_u32(state.as_mut_ptr(), abcd);
            vst1q_u32(state.as_mut_ptr().add(4), efgh);
        }
    }
}

#[cfg(test)]
mod tests {
    use sha2::Digest;

    #[test]
    fn as_sha2() {
        let mut seed = 0x9e3779b97f4a7c15u64;
        let bytes: Vec<u8> = (0..70_000)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        for len in [0, 1, 55, 56, 63, 64, 65, 119, 120, 128, 4096, 16384, 70_000] {
            let want: [u8; 32] = sha2::Sha256::digest(&bytes[..len]).into();
            assert_eq!(super::digest(&bytes[..len]), want, "{len} bytes");
        }
    }
}
