//! simdutf8 classifies bytes with `_mm256_subs_epu8`, which is `simd_saturating_sub`, and
//! glam adds vectors of floats with `_mm_add_ps`, which is the `simd_add` that integers have.

#[cfg(target_arch = "x86_64")]
pub fn run() -> ([u8; 16], [f32; 4], [u32; 4]) {
    use std::arch::x86_64::*;
    let mut bytes = [0u8; 16];
    let mut floats = [0f32; 4];
    let mut masks = [0u32; 4];
    unsafe {
        let high = _mm_set_epi8(-1, -2, 100, 3, 0, 9, 50, 7, -128, 127, 1, 0, 60, -60, 5, 4);
        let low = _mm_set1_epi8(8);
        let under = _mm_subs_epu8(high, low);
        let over = _mm_adds_epi8(under, _mm_set1_epi8(120));
        _mm_storeu_si128(bytes.as_mut_ptr().cast(), over);
        let a = _mm_set_ps(1.5, -2.25, 3.0, f32::NAN);
        let b = _mm_set_ps(0.5, 4.0, -3.0, 1.0);
        let mixed = _mm_div_ps(_mm_mul_ps(_mm_add_ps(a, b), _mm_sub_ps(a, b)), b);
        _mm_storeu_ps(floats.as_mut_ptr(), mixed);
        _mm_storeu_ps(masks.as_mut_ptr().cast(), _mm_cmplt_ps(a, b));
    }
    // A NaN's bits are not the same from one way of computing it to the next.
    floats[0] = if floats[0].is_nan() { 0.0 } else { floats[0] };
    (bytes, floats, masks)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> ([u8; 16], [f32; 4], [u32; 4]) {
    let bytes = [124, 120, 127, 127, 120, 121, 127, 127, 127, 127, 121, 120, 127, 127, 127, 127];
    (bytes, [0.0, 0.0, -1.359375, 4.0], [0, 0, u32::MAX, 0])
}
