//! `_mm256_permutevar8x32_epi32` and `_mm256_permutevar8x32_ps` are `llvm.x86.avx2.permd` and
//! `llvm.x86.avx2.permps`: lanes chosen by a vector of indices. zlib-rs sums an Adler-32
//! with the first.

#[cfg(target_arch = "x86_64")]
pub fn run() -> ([[u32; 8]; 3], [f32; 8]) {
    use std::arch::x86_64::*;
    if !is_x86_feature_detected!("avx2") {
        return ([[0; 8]; 3], [0.0; 8]);
    }
    #[target_feature(enable = "avx2")]
    unsafe fn permuted(of: [u32; 8], by: [[u32; 8]; 3], singles: [f32; 8]) -> ([[u32; 8]; 3], [f32; 8]) {
        unsafe {
            let table = _mm256_loadu_si256(of.as_ptr().cast());
            let lanes = by.map(|indices| {
                let indices = _mm256_loadu_si256(indices.as_ptr().cast());
                let mut out = [0u32; 8];
                _mm256_storeu_si256(out.as_mut_ptr().cast(), _mm256_permutevar8x32_epi32(table, indices));
                out
            });
            let indices = _mm256_loadu_si256(by[1].as_ptr().cast());
            let mut out = [0f32; 8];
            _mm256_storeu_ps(out.as_mut_ptr(), _mm256_permutevar8x32_ps(_mm256_loadu_ps(singles.as_ptr()), indices));
            (lanes, out)
        }
    }
    let by = [[7, 6, 5, 4, 3, 2, 1, 0], [0, 0, 9, 0xffff_fff3, 4, 12, 7, 1], [1, 3, 5, 7, 0, 2, 4, 6]];
    unsafe { permuted([10, 11, 12, 13, 14, 15, 16, u32::MAX], by, [0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, -7.5]) }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> ([[u32; 8]; 3], [f32; 8]) {
    ([[0; 8]; 3], [0.0; 8])
}
