//! simdutf8 hints the cache at what it validates next: `_mm_prefetch` is `llvm.prefetch`,
//! which changes nothing a program can see.

#[cfg(target_arch = "x86_64")]
pub fn run() -> u64 {
    use std::arch::x86_64::{_MM_HINT_T0, _mm_prefetch};
    let bytes: Vec<u8> = (0..=255).collect();
    let mut sum = 0u64;
    for chunk in bytes.chunks(64) {
        unsafe { _mm_prefetch(chunk.as_ptr().cast::<i8>().wrapping_add(64), _MM_HINT_T0) };
        sum += chunk.iter().map(|&it| u64::from(it)).sum::<u64>();
    }
    sum
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> u64 {
    32640
}
